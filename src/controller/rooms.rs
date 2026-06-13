use crate::controller::tipi::SharedState;
use crate::controller::web::AppError;
use crate::entities::{client, message, soba};
use crate::entities::prelude::{Client, Message, Soba};
use axum::{
    extract::{Form, Path, State},
    response::Html,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set, Statement,
};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use migration::{Migrator, MigratorTrait};

#[derive(Debug, Deserialize)]
pub struct CreateRoomForm {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessageForm {
    pub content: String,
    // Zaenkrat username pride iz forme ali ga kasneje doda frontend iz prijave.
    // Če ga ni, sporočilo shranimo brez sender_id, da endpoint ostane uporaben tudi za gosta.
    pub username: Option<String>,
}

fn db_from_state(state: &SharedState) -> Result<DatabaseConnection, AppError> {
    // Pomembno: mutex držimo samo toliko časa, da kloniramo DatabaseConnection.
    // Nikoli ne držimo locka čez .await, ker lahko to hitro povzroči čudne blokade.
    Ok(state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?
        .db
        .clone())
}


pub async fn prepare_database_schema(db: &DatabaseConnection) -> Result<(), AppError> {
    // To je praktična varovalka za razvojno verzijo projekta: aplikacija lahko zažene svežo
    // SQLite bazo brez ročnega pripravljanja tabel. Migracije so še vedno koristne, ampak za
    // primerjalni ZIP je bolj prijazno, da osnovna shema nastane sama.
    let backend = db.get_database_backend();
    Migrator::up(db, None).await?;
    Ok(())
}

pub async fn ensure_default_room(db: &DatabaseConnection) -> Result<(), AppError> {
    // Aplikacija trenutno predvideva sobo "general" že v HTML-ju.
    // Zato jo ustvarimo ob zagonu, če še ne obstaja.
    ensure_room_exists(db, "general").await?;
    Ok(())
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

    let room = soba::ActiveModel {
        name: Set(clean_name),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(room)
}

fn normalize_room_name(name: &str) -> Result<String, AppError> {
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

pub async fn list_rooms(State(state): State<SharedState>) -> Result<Html<String>, AppError> {
    let db = db_from_state(&state)?;
    ensure_default_room(&db).await?;

    let rooms = Soba::find()
        .order_by_asc(soba::Column::Name)
        .all(&db)
        .await?;

    // Vračamo HTML, ker trenutni frontend uporablja HTMX in hx-swap.
    // Če kasneje preidete na čist JS/JSON, lahko ta handler enostavno zamenjamo z Json<Vec<...>>.
    let mut html = String::new();
    for room in rooms {
        html.push_str(&format!(
            r##"<button class="room-item" hx-get="/rooms/{name}/messages" hx-target="#messages" hx-swap="innerHTML" onclick="document.getElementById('room-title').textContent='{name}'"># {name}</button>"##,
            name = html_escape(&room.name)
        ));
    }

    Ok(Html(html))
}

pub async fn create_room(
    State(state): State<SharedState>,
    Form(form): Form<CreateRoomForm>,
) -> Result<Html<String>, AppError> {
    let db = db_from_state(&state)?;
    let room = ensure_room_exists(&db, &form.name).await?;

    Ok(Html(format!(
        r##"<button class="room-item" hx-get="/rooms/{name}/messages" hx-target="#messages" hx-swap="innerHTML" onclick="document.getElementById('room-title').textContent='{name}'"># {name}</button>"##,
        name = html_escape(&room.name)
    )))
}

pub async fn list_messages(
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
) -> Result<Html<String>, AppError> {
    let db = db_from_state(&state)?;
    let room = ensure_room_exists(&db, &room_name).await?;

    let messages = Message::find()
        .filter(message::Column::SobaId.eq(room.id))
        .order_by_asc(message::Column::Timestamp)
        .all(&db)
        .await?;

    let mut html = String::from(r#"<div class="date-sep">Today</div>"#);

    if messages.is_empty() {
        html.push_str(&format!(
            r#"<div class="sys-msg">Dobrodošla v #{}</div>"#,
            html_escape(&room.name)
        ));
    }

    for msg in messages {
        html.push_str(&render_message(&msg, None));
    }

    Ok(Html(html))
}

pub async fn create_message(
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
    Form(form): Form<CreateMessageForm>,
) -> Result<Html<String>, AppError> {
    let content = form.content.trim();
    if content.is_empty() {
        return Ok(Html(String::new()));
    }

    let db = db_from_state(&state)?;
    let room = ensure_room_exists(&db, &room_name).await?;

    let sender_id = match form.username.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(username) => Client::find()
            .filter(client::Column::Username.eq(username))
            .one(&db)
            .await?
            .map(|user| user.id as i64),
        None => None,
    };

    let msg = message::ActiveModel {
        sender_id: Set(sender_id),
        content: Set(content.to_string()),
        timestamp: Set(current_timestamp()),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    Ok(Html(render_message(&msg, form.username.as_deref())))
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn render_message(msg: &message::Model, username_hint: Option<&str>) -> String {
    let sender = username_hint
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("gost");

    format!(
        r#"<div class="msg"><strong>{}</strong>: {}</div>"#,
        html_escape(sender),
        html_escape(&msg.content)
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
