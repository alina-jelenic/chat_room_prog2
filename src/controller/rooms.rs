use crate::controller::auth::{require_auth, AuthUser};
use crate::controller::tipi::SharedState;
use crate::controller::web::AppError;
use crate::entities::prelude::{Client, Message, Soba};
use crate::entities::{client, message, soba};
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, TimeZone, Utc};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
pub struct CreateRoomForm {
    pub name: String,
}

const MAX_MESSAGE_LENGTH: usize = 2000;

fn db_from_state(state: &SharedState) -> Result<DatabaseConnection, AppError> {
    // Pomembno: mutex držimo samo toliko časa, da kloniramo DatabaseConnection.
    // Nikoli ne držimo locka čez .await, ker lahko to hitro povzroči čudne blokade.
    Ok(state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?
        .db
        .clone())
}

fn authenticated_user(jar: &CookieJar, state: &SharedState) -> Result<AuthUser, Response> {
    let secret = match state.lock() {
        Ok(state) => state.jwt_secret.clone(),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };

    require_auth(jar, &secret)
}

pub async fn prepare_database_schema(db: &DatabaseConnection) -> Result<(), AppError> {
    // Migrator::up izvede samo migracije, ki še niso zapisane v seaql_migrations.
    // Zato je varno klicati to funkcijo ob vsakem startu aplikacije.
    // Na sveži SQLite bazi se s tem samodejno ustvarijo vse potrebne tabele.
    Migrator::up(db, None).await?;
    Ok(())
}

pub async fn ensure_default_room(db: &DatabaseConnection) -> Result<(), AppError> {
    // Aplikacija trenutno predvideva sobo "general" že v HTML-ju.
    // Zato jo ustvarimo ob zagonu, če še ne obstaja.
    ensure_room_exists(db, "general").await?;
    Ok(())
}

pub async fn room_for_websocket(
    db: &DatabaseConnection,
    name: &str,
) -> Result<Option<soba::Model>, AppError> {
    let clean_name = normalize_room_name(name)?;
    Ok(Soba::find()
        .filter(soba::Column::Name.eq(clean_name))
        .one(db)
        .await?)
}

async fn ensure_room_exists(db: &DatabaseConnection, name: &str) -> Result<soba::Model, AppError> {
    let clean_name = normalize_room_name(name)?;

    if let Some(room) = Soba::find()
        .filter(soba::Column::Name.eq(&clean_name))
        .one(db)
        .await?
    {
        return Ok(room);
    }

    use rand::Rng;
    let code = rand::thread_rng().gen_range(100000..=999999);

    let room = soba::ActiveModel {
        id: Set(code),
        name: Set(clean_name),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(room)
}

pub fn normalize_room_name(name: &str) -> Result<String, AppError> {
    let clean = name.trim().to_lowercase();

    if clean.is_empty() {
        return Err(AppError("Ime sobe ne sme biti prazno.".to_string()));
    }

    if clean.len() > 32 {
        return Err(AppError("Ime sobe je predolgo.".to_string()));
    }

    // Namerno omejeno: manj težav pri URL-jih, selectorjih in kasnejšem WebSocket query parametru.
    if !clean
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(AppError(
            "Ime sobe lahko vsebuje samo črke, številke, '-' ali '_'.".to_string(),
        ));
    }

    Ok(clean)
}

pub async fn list_rooms(
    jar: CookieJar,
    State(state): State<SharedState>,
) -> Result<Response, AppError> {
    if let Err(response) = authenticated_user(&jar, &state) {
        return Ok(response);
    }

    let db = db_from_state(&state)?;
    ensure_default_room(&db).await?;

    Ok(Html(render_room_list(&db, "general").await?).into_response())
}

pub async fn create_room(
    jar: CookieJar,
    State(state): State<SharedState>,
    Form(form): Form<CreateRoomForm>,
) -> Result<Response, AppError> {
    if let Err(response) = authenticated_user(&jar, &state) {
        return Ok(response);
    }

    let db = db_from_state(&state)?;
    let clean_name = normalize_room_name(&form.name)?;
    if room_for_websocket(&db, &clean_name).await?.is_some() {
        // Gumb za obstoječo sobo je že v seznamu, zato ga ne podvojimo.
        return Ok(Html(String::new()).into_response());
    }
    let room = ensure_room_exists(&db, &clean_name).await?;

    Ok(Html(render_room_button(&room, false)).into_response())
}

pub async fn room_panel(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &room_name).await? {
        Some(room) => room,
        None => {
            return Ok((
                StatusCode::NOT_FOUND,
                Html(r#"<div class="sys-msg">Soba ne obstaja.</div>"#),
            )
                .into_response())
        }
    };

    let mut html = render_chat_panel(&room, &user);

    // Ko uporabnik zamenja sobo, hkrati posodobimo še aktiven gumb v seznamu sob.
    html.push_str(&format!(
        r#"<div id="room-list" hx-swap-oob="innerHTML">{}</div>"#,
        render_room_list(&db, &room.name).await?
    ));

    Ok(Html(html).into_response())
}

pub async fn list_messages(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
) -> Result<Response, AppError> {
    if let Err(response) = authenticated_user(&jar, &state) {
        return Ok(response);
    }

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &room_name).await? {
        Some(room) => room,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };

    Ok(Html(render_messages_for_room(&db, &room).await?).into_response())
}

pub async fn delete_room(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let clean_name = normalize_room_name(&room_name)?;
    if clean_name == "general" {
        return Ok((StatusCode::BAD_REQUEST, "Sobe general ni mogoče izbrisati.").into_response());
    }

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &clean_name).await? {
        Some(room) => room,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };

    // Sporočila in sobo izbrišemo v isti transakciji, da v bazi ne more
    // ostati napol izvedena operacija.
    let transaction = db.begin().await?;
    Message::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&transaction)
        .await?;
    Soba::delete_by_id(room.id).exec(&transaction).await?;
    transaction.commit().await?;

    let general = ensure_room_exists(&db, "general").await?;
    let room_list = render_room_list(&db, &general.name).await?;
    notify_room_deleted(&state, room.id, &room.name, &room_list)?;

    let mut html = render_chat_panel(&general, &user);
    html.push_str(&render_room_list_oob(&room_list));
    Ok(Html(html).into_response())
}

pub async fn create_websocket_message(
    db: &DatabaseConnection,
    room_id: i32,
    user: &AuthUser,
    content: &str,
) -> Result<String, AppError> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(String::new());
    }
    if content.chars().count() > MAX_MESSAGE_LENGTH {
        return Err(AppError(format!(
            "Sporočilo ima lahko največ {MAX_MESSAGE_LENGTH} znakov."
        )));
    }

    let msg = insert_message(db, room_id, Some(user.id as i64), content).await?;
    Ok(render_message_oob(&msg, Some(&user.username), msg.timestamp))
}

async fn render_room_list(
    db: &DatabaseConnection,
    active_room_name: &str,
) -> Result<String, AppError> {
    let rooms = Soba::find()
        .order_by_asc(soba::Column::Name)
        .all(db)
        .await?;

    let mut html = String::new();
    for room in rooms {
        html.push_str(&render_room_button(&room, room.name == active_room_name));
    }

    Ok(html)
}

async fn render_messages_for_room(
    db: &DatabaseConnection,
    room: &soba::Model,
) -> Result<String, AppError> {
    let messages = Message::find()
        .filter(message::Column::SobaId.eq(room.id))
        .order_by_asc(message::Column::Timestamp)
        .order_by_asc(message::Column::Id)
        .all(db)
        .await?;

    let sender_ids: Vec<i64> = messages.iter().filter_map(|msg| msg.sender_id).collect();

    let clients = Client::find()
        .filter(client::Column::Id.is_in(sender_ids))
        .all(db)
        .await?;

    let sender_map: HashMap<i64, String> = clients
        .into_iter()
        .map(|client| (client.id as i64, client.username))
        .collect();

    let mut html = String::new();
    let mut last_date = String::new();

    if messages.is_empty() {
        html.push_str(&format!(
            r#"<div class="sys-msg">Dobrodošla v #{}</div>"#,
            html_escape(&room.name)
        ));
    }

    for msg in messages {
        let sender_name = msg
            .sender_id
            .and_then(|id| sender_map.get(&id))
            .map(String::as_str);

        let date_str = DateTime::from_timestamp(msg.timestamp, 0)
            .map(|dt| dt.format("%d. %m. %Y").to_string())
            .unwrap_or_else(|| "neznan datum".to_string());

        if date_str != last_date {
            html.push_str(&format!(r#"<div class="date-sep">{}</div>"#, date_str));
            last_date = date_str;
        }

        html.push_str(&render_message(&msg, sender_name, msg.timestamp));
    }

    Ok(html)
}

async fn insert_message(
    db: &DatabaseConnection,
    room_id: i32,
    sender_id: Option<i64>,
    content: &str,
) -> Result<message::Model, AppError> {
    let msg = message::ActiveModel {
        sender_id: Set(sender_id),
        content: Set(content.to_string()),
        timestamp: Set(current_timestamp()),
        soba_id: Set(room_id),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(msg)
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn render_chat_panel(room: &soba::Model, user: &AuthUser) -> String {
    let name = html_escape(&room.name);
    let username = html_escape(&user.username);
    let delete_control = if room.name == "general" {
        String::new()
    } else {
        format!(
            r##"<button type="button" class="delete-room-btn"
                hx-delete="/rooms/{name}"
                hx-target="#chat-panel"
                hx-swap="outerHTML"
                hx-confirm="Ali res želiš izbrisati sobo #{name} in vsa njena sporočila?">
              Izbriši sobo
            </button>"##
        )
    };

    format!(
        r##"
<div class="main" id="chat-panel"
     data-current-user-id="{user_id}"
     data-current-username="{username}"
     hx-ext="ws" ws-connect="/ws?room_name={name}">
  <div id="offline-banner">WebSocket povezava je aktivna za #{name}</div>

  <div class="chat-header">
    <span class="chat-header-hash">#</span>
    <span class="chat-header-name" id="room-title">{name}</span>
    <div class="connection-dot connected" id="conn-dot"></div>
    <span class="connection-label" id="conn-label">websocket</span>
    {delete_control}
  </div>

  <div class="messages" id="messages"
    hx-get="/rooms/{name}/messages"
    hx-trigger="load"
    hx-swap="innerHTML">
    <div class="sys-msg">Nalaganje sporočil za #{name}…</div>
  </div>

  <div class="input-area">
    <form id="msg-form" ws-send>
      <div class="input-row">
        <textarea name="content" id="msg-input" rows="1" maxlength="{max_message_length}" placeholder="Sporočilo…" required></textarea>
        <button type="submit" class="send-btn" aria-label="Pošlji">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M2 8L14 2L8 14L7 9L2 8Z" fill="white" stroke="white" stroke-width=".5" stroke-linejoin="round"/>
          </svg>
        </button>
      </div>
    </form>
  </div>
</div>
"##,
        name = name,
        username = username,
        user_id = user.id,
        delete_control = delete_control,
        max_message_length = MAX_MESSAGE_LENGTH,
    )
}

fn render_room_list_oob(room_list: &str) -> String {
    format!(r#"<div id="room-list" hx-swap-oob="innerHTML">{room_list}</div>"#)
}

fn notify_room_deleted(
    state: &SharedState,
    deleted_room_id: i32,
    deleted_room_name: &str,
    room_list: &str,
) -> Result<(), AppError> {
    let (deleted_room_sender, other_senders) = {
        let mut state = state
            .lock()
            .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?;
        let deleted = state.soba_tx.remove(&deleted_room_id);
        let others = state.soba_tx.values().cloned().collect::<Vec<_>>();
        (deleted, others)
    };

    let room_list_oob = render_room_list_oob(room_list);
    for sender in other_senders {
        let _ = sender.send(room_list_oob.clone());
    }

    if let Some(sender) = deleted_room_sender {
        let deleted_name = html_escape(deleted_room_name);
        let redirect_to_general = format!(
            r##"<div class="main" id="chat-panel" hx-swap-oob="outerHTML"
                 hx-get="/rooms/general/panel" hx-trigger="load" hx-swap="outerHTML">
              <div class="sys-msg">Soba #{deleted_name} je bila izbrisana. Odpiram #general…</div>
            </div>"##
        );
        let _ = sender.send(format!("{redirect_to_general}{room_list_oob}"));
    }

    Ok(())
}

fn render_message(msg: &message::Model, sender_name: Option<&str>, timestamp: i64) -> String {
    let sender = sender_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("neznan uporabnik");

    let time_str = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());

    format!(
        r#"<div class="msg"><strong>{}</strong> <span class="time">{}</span>: {}</div>"#,
        html_escape(sender),
        time_str,
        html_escape(&msg.content)
    )
}

fn render_message_oob(msg: &message::Model, sender_name: Option<&str>, timestamp: i64) -> String {
    format!(
        r#"<div id="messages" hx-swap-oob="beforeend">{}</div>"#,
        render_message(msg, sender_name, timestamp)
    )
}

fn render_room_button(room: &soba::Model, active: bool) -> String {
    let active_class = if active { " active" } else { "" };
    let pressed = if active { "true" } else { "false" };
    let name = html_escape(&room.name);

    format!(
        r##"
<button
    type="button"
    class="room-item{active_class}"
    data-room-id="{id}"
    data-room-name="{name}"
    aria-pressed="{pressed}"
    hx-get="/rooms/{name}/panel"
    hx-target="#chat-panel"
    hx-swap="outerHTML">
    # {name}
</button>
"##,
        active_class = active_class,
        id = room.id,
        name = name,
        pressed = pressed,
    )
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
