// router - dod uporabnikom, dod sob, navigiraš po straniš, naložiš sporočila itd
//websocket - je pa samo za pogovor, da se komunikacija direklno pogovarja

use crate::controller::tipi::{ServerState, SharedState};
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query, State},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
// use std::sync::Arc;
use tower_http::services::ServeDir;
use axum::http::StatusCode;
use crate::controller::forms::{login_handler, register_handler};
use crate::controller::rooms;
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
    username: Option<String>,
}

pub async fn run_websocket(state: SharedState) -> Result<(), Box<dyn std::error::Error>> {
    

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/login", post(login_handler))
        .route("/api/register", post(register_handler))
        .route("/rooms", get(rooms::list_rooms).post(rooms::create_room))
        .route("/rooms/{name}/messages", get(rooms::list_messages))
        .route("/rooms/{name}/messages", post(rooms::create_message))
        .fallback_service(ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("WebSocket chat posluša na ws://127.0.0.1:3000/ws?username=ime");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    Query(query): Query<WsQuery>,
) -> impl IntoResponse {
    let username = query
        .username
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "gost".to_string());

    ws.on_upgrade(move |socket| handle_socket(socket, username, state))
}

async fn handle_socket(socket: WebSocket, username: String, state: SharedState) {
    let (tx, mut rx) = {
        let state = state.lock().unwrap();
        (state.tx.clone(), state.tx.subscribe())
    };

    let (mut sender, mut receiver) = socket.split();

    let _ = tx.send(format!("*** {username} se je pridružil ***"));
    let user = username.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(message) = rx.recv().await {
            if !message.starts_with(&format!("{user}:")) {
                if sender.send(Message::Text(message.into())).await.is_err() {
                    break;
                }
            }
        }
    });


    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                let text = text.trim();
                if !text.is_empty() {
                    let _ = tx.send(format!("{username}: {text}"));
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    send_task.abort();
    let _ = tx.send(format!("*** {username} je odšel ***"));
}
