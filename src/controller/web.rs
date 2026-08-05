// HTTP endpointi skrbijo za avtentikacijo, sobe in nalaganje zgodovine.
// WebSocket povezava skrbi samo za realnočasovno izmenjavo sporočil.

use crate::controller::auth::{AuthUser, auth_user_from_jar, unauthorized_response};
use crate::controller::forms::{login_handler, register_handler};
use crate::controller::rooms;
use crate::controller::tipi::SharedState;
use axum::http::StatusCode;
use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use futures_util::{SinkExt, StreamExt};
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

#[derive(Debug)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0).into_response()
    }
}

impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError(e.to_string())
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError(e)
    }
}

impl From<&str> for AppError {
    fn from(e: &str) -> Self {
        AppError(e.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    room_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsIncomingMessage {
    content: Option<String>,
}

pub async fn run_websocket(state: SharedState) -> Result<(), Box<dyn std::error::Error>> {
    let app = build_router(state);
    let server_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());

    let listener = tokio::net::TcpListener::bind(&server_addr).await?;
    println!("ChatRoom posluša na http://{server_addr}");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Router je ločena funkcija, da ga lahko integracijski testi poženejo
/// brez vezave na fiksna vrata 3000.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/me", get(crate::controller::auth::me_handler))
        .route("/api/login", post(login_handler))
        .route("/api/logout", post(crate::controller::auth::logout_handler))
        .route("/api/register", post(register_handler))
        .route("/rooms", get(rooms::list_rooms).post(rooms::create_room))
        .route("/rooms/{name}/panel", get(rooms::room_panel))
        .route("/rooms/{name}/messages", get(rooms::list_messages))
        .route(
            "/rooms/{name}/membership",
            axum::routing::delete(rooms::leave_room),
        )
        .route("/rooms/{name}", axum::routing::delete(rooms::delete_room))
        .route("/rooms/join", post(rooms::join_room))
        .fallback_service(ServeDir::new("static"))
        .with_state(state)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    jar: CookieJar,
    State(state): State<SharedState>,
    Query(query): Query<WsQuery>,
) -> Response {
    let secret = match state.lock() {
        Ok(state) => state.jwt_secret.clone(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let user = match auth_user_from_jar(&jar, &secret) {
        Some(user) => user,
        None => return unauthorized_response(),
    };

    let room_name = query
        .room_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "general".to_string());

    ws.on_upgrade(move |socket| handle_socket(socket, user, room_name, state))
        .into_response()
}

async fn handle_socket(socket: WebSocket, user: AuthUser, room_name: String, state: SharedState) {
    let db = match db_from_state(&state) {
        Ok(db) => db,
        Err(_) => return,
    };

    let room = match rooms::room_for_websocket(&db, &room_name).await {
        Ok(Some(room)) => room,
        Ok(None) => return,
        Err(_) => return,
    };
    match rooms::user_can_access_room(&db, &room, user.id).await {
        Ok(true) => {}
        Ok(false) | Err(_) => return,
    }

    let (tx, mut rx, mut outgoing_revocations, mut incoming_revocations) = {
        let mut state = match state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        let tx = state.get_or_create_room_tx(room.id);
        (
            tx.clone(),
            tx.subscribe(),
            state.subscribe_to_room_access_revocations(),
            state.subscribe_to_room_access_revocations(),
        )
    };

    // Kanal za odgovore, namenjene samo trenutnemu uporabniku,
    // na primer za praznjenje njegovega vnosnega polja.
    let (personal_tx, mut personal_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let (mut sender, mut receiver) = socket.split();

    let outgoing_db = db.clone();
    let outgoing_room = room.clone();
    let outgoing_user_id = user.id;

    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                // Obvestilo o odvzemu dostopa ima prednost pred novimi
                // sporočili, da pasivna povezava po odhodu iz sobe ne more
                // več prejemati vsebine drugih uporabnikov.
                biased;

                revoked = outgoing_revocations.recv() => {
                    match revoked {
                        Ok(event)
                            if event.room_id == outgoing_room.id
                                && event.user_id == outgoing_user_id => break,
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            match rooms::user_can_access_room(
                                &outgoing_db,
                                &outgoing_room,
                                outgoing_user_id,
                            )
                            .await
                            {
                                Ok(true) => continue,
                                Ok(false) | Err(_) => break,
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                broadcasted = rx.recv() => {
                    match broadcasted {
                        Ok(message) => {
                            if sender.send(Message::Text(message.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                personal = personal_rx.recv() => {
                    match personal {
                        Some(message) => {
                            if sender.send(Message::Text(message.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });

    loop {
        tokio::select! {
            biased;

            revoked = incoming_revocations.recv() => {
                match revoked {
                    Ok(event) if event.room_id == room.id && event.user_id == user.id => break,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        match rooms::user_can_access_room(&db, &room, user.id).await {
                            Ok(true) => continue,
                            Ok(false) | Err(_) => break,
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            result = receiver.next() => {
                match result {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(content) = websocket_content(&text) {
                            // Članstvo se lahko spremeni tudi po vzpostavitvi povezave.
                            // Preverba pred shranjevanjem ostane dodatna zaščita pred
                            // sporočilom, ki prispe istočasno z odhodom uporabnika.
                            match rooms::user_can_access_room(&db, &room, user.id).await {
                                Ok(true) => {}
                                Ok(false) | Err(_) => break,
                            }

                            if let Err(error) = rooms::validate_message_content(&content) {
                                let _ = personal_tx.send(rooms::render_message_error(&error.0));
                                continue;
                            }

                            let may_send = match state.lock() {
                                Ok(mut state) => state.reserve_message_send(user.id),
                                Err(_) => break,
                            };
                            if !may_send {
                                let _ = personal_tx.send(rooms::render_rate_limit_warning());
                                continue;
                            }

                            match rooms::create_websocket_message(&db, room.id, &user, &content).await {
                                Ok(html) if !html.is_empty() => {
                                    let _ = tx.send(html);
                                    let _ = personal_tx.send(rooms::render_message_input_reset());
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("Napaka pri shranjevanju sporočila WebSocket: {e}");
                                    let _ = personal_tx.send(rooms::render_message_error(
                                        "Sporočila ni bilo mogoče poslati.",
                                    ));
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    send_task.abort();
}

fn db_from_state(state: &SharedState) -> Result<DatabaseConnection, AppError> {
    Ok(state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
        .db
        .clone())
}

fn websocket_content(text: &str) -> Option<String> {
    // HTMX ws-send pošlje formo kot JSON, npr. {"content":"živjo", "HEADERS":{...}}.
    // Fallback podpira tudi ročno WebSocket testiranje z navadnim tekstom.
    if let Ok(message) = serde_json::from_str::<WsIncomingMessage>(text) {
        return message
            .content
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty());
    }

    let content = text.trim().to_string();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}
