//! Nalaganje in iskanje zgodovine, brisanje ter prikaz sporočil.

use super::reactions::{
    reaction_counts_for_messages, render_quick_reaction_buttons, render_reaction_add_form,
    render_reaction_oznaka,
};
use super::views::broadcast_room_html;
use super::{authenticated_user, db_from_state, room_for_websocket, user_can_access_room};
use crate::controller::auth::AuthUser;
use crate::controller::rooms::reply::{
    ReplyPreview, reply_previews_for_messages, truncate_preview,
};
use crate::controller::tipi::SharedState;
use crate::controller::util::html_escape;
use crate::controller::web::AppError;
use crate::entities::prelude::{Client, Message, Soba};
use crate::entities::{client, message, soba};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Local, TimeZone};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};
pub(super) const MAX_MESSAGE_LENGTH: usize = 2000;

const MESSAGES_PAGE_SIZE: u64 = 50;
const SEARCH_RESULTS_PAGE_SIZE: u64 = 30;
pub(super) const MAX_SEARCH_LENGTH: usize = 100;

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub before_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SearchMessagesQuery {
    pub q: Option<String>,
    pub before_id: Option<i32>,
}

pub async fn list_messages(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &room_name).await? {
        Some(room) => room,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    if !user_can_access_room(&db, &room, user.id).await? {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    Ok(Html(render_messages_page(&db, &room, query.before_id).await?).into_response())
}

pub async fn search_messages(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &room_name).await? {
        Some(room) => room,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    if !user_can_access_room(&db, &room, user.id).await? {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    let search_term = query.q.as_deref().unwrap_or_default().trim();
    if search_term.is_empty() {
        return Ok(Html(String::new()).into_response());
    }
    if search_term.chars().count() > MAX_SEARCH_LENGTH {
        return Ok(Html(format!(
            r#"<div class="search-message error">Iskalni niz ima lahko največ {MAX_SEARCH_LENGTH} znakov.</div>"#
        ))
        .into_response());
    }

    Ok(
        Html(render_message_search_page(&db, &room, search_term, query.before_id).await?)
            .into_response(),
    )
}

pub async fn delete_message(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(message_id): Path<i32>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let stored_message = match Message::find_by_id(message_id).one(&db).await? {
        Some(message) => message,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let room = match Soba::find_by_id(stored_message.soba_id).one(&db).await? {
        Some(room) => room,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    if !user_can_access_room(&db, &room, user.id).await? {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    if stored_message.sender_id != Some(user.id) {
        return Ok((
            StatusCode::FORBIDDEN,
            "Izbrišeš lahko samo svoja sporočila.",
        )
            .into_response());
    }

    Message::delete_by_id(stored_message.id).exec(&db).await?;
    let deletion = render_message_deletion_oob(stored_message.id);
    broadcast_room_html(&state, room.id, deletion.clone())?;

    Ok(Html(deletion).into_response())
}

pub async fn create_websocket_message(
    db: &DatabaseConnection,
    room_id: i32,
    user: &AuthUser,
    content: &str,
    reply_to_id: Option<i32>,
) -> Result<String, AppError> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(String::new());
    }
    validate_message_content(content)?;

    let room = match Soba::find_by_id(room_id).one(db).await? {
        Some(room) => room,
        None => return Ok(String::new()),
    };

    let reply_to_id = match reply_to_id {
        Some(id) => {
            let exists = Message::find_by_id(id)
                .filter(message::Column::SobaId.eq(room_id))
                .one(db)
                .await?
                .is_some();
            exists.then_some(id)
        }
        None => None,
    };

    let msg = insert_message(db, room_id, Some(user.id), content, reply_to_id).await?;
    let previews = reply_previews_for_messages(db, std::slice::from_ref(&msg)).await?;
    let reply_preview = msg.reply_to_id.and_then(|id| previews.get(&id));
    Ok(render_message_oob(
        &room.name,
        &msg,
        Some(&user.username),
        msg.timestamp,
        reply_preview,
    ))
}

pub fn validate_message_content(content: &str) -> Result<(), AppError> {
    if content.chars().count() > MAX_MESSAGE_LENGTH {
        return Err(AppError(format!(
            "Sporočilo ima lahko največ {MAX_MESSAGE_LENGTH} znakov."
        )));
    }
    Ok(())
}

async fn render_messages_page(
    db: &DatabaseConnection,
    room: &soba::Model,
    before_id: Option<i32>,
) -> Result<String, AppError> {
    let mut query = Message::find().filter(message::Column::SobaId.eq(room.id));

    if let Some(before_id) = before_id {
        query = query.filter(message::Column::Id.lt(before_id));
    }

    // Vzamemo eno sporočilo več, da ugotovimo, ali obstajajo še starejša.
    let mut messages = query
        .order_by_desc(message::Column::Id)
        .limit(MESSAGES_PAGE_SIZE + 1)
        .all(db)
        .await?;

    let has_more = messages.len() as u64 > MESSAGES_PAGE_SIZE;
    if has_more {
        messages.truncate(MESSAGES_PAGE_SIZE as usize);
    }
    messages.reverse(); // nazaj v kronološki vrstni red za izpis

    let sender_ids: Vec<i64> = messages.iter().filter_map(|msg| msg.sender_id).collect();
    let clients = Client::find()
        .filter(client::Column::Id.is_in(sender_ids))
        .all(db)
        .await?;
    let sender_map: HashMap<i64, String> = clients
        .into_iter()
        .map(|client| (client.id, client.username))
        .collect();

    // Bloke (ločila + sporočila) gradimo v kronološkem vrstnem redu,
    // nato jih obrnemo, ker mora biti v DOM-u najnovejše sporočilo prvo
    // (zaradi flex-direction: column-reverse v CSS-ju).
    let mut blocks: Vec<String> = Vec::new();

    if messages.is_empty() && before_id.is_none() {
        blocks.push(format!(
            r#"<div class="sys-msg">To je začetek pogovora v #{}</div>"#,
            html_escape(&room.name)
        ));
    }

    let mut last_date = String::new();
    let message_ids: Vec<i32> = messages.iter().map(|msg| msg.id).collect();
    let reactions_counts = reaction_counts_for_messages(db, &message_ids).await?;
    let empty_counts = BTreeMap::new();
    let reply_previews = reply_previews_for_messages(db, &messages).await?;
    for msg in &messages {
        let sender_name = msg
            .sender_id
            .and_then(|id| sender_map.get(&id))
            .map(String::as_str);

        let date_str = Local
            .timestamp_opt(msg.timestamp, 0)
            .single()
            .map(|dt| dt.format("%d. %m. %Y").to_string())
            .unwrap_or_else(|| "neznan datum".to_string());

        if date_str != last_date {
            blocks.push(format!(r#"<div class="date-sep">{}</div>"#, date_str));
            last_date = date_str;
        }

        let counts = reactions_counts.get(&msg.id).unwrap_or(&empty_counts);
        let preview = msg.reply_to_id.and_then(|id| reply_previews.get(&id));
        blocks.push(render_message(
            &room.name,
            msg,
            sender_name,
            msg.timestamp,
            counts,
            preview,
        ));
    }

    blocks.reverse();

    let mut html = String::new();
    for block in blocks {
        html.push_str(&block);
    }

    // Gumb za starejša sporočila je vizualno na vrhu, torej zadnji v DOM-u.
    if has_more && let Some(oldest) = messages.first() {
        html.push_str(&render_load_older_button(&room.name, oldest.id));
    }

    Ok(html)
}

fn render_load_older_button(room_name: &str, before_id: i32) -> String {
    let name = html_escape(room_name);
    format!(
        r##"<button type="button" class="load-older-btn"
            hx-get="/rooms/{name}/messages?before_id={before_id}"
            hx-target="this"
            hx-swap="outerHTML">
          Naloži starejša sporočila
        </button>"##
    )
}

fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

async fn render_message_search_page(
    db: &DatabaseConnection,
    room: &soba::Model,
    search_term: &str,
    before_id: Option<i32>,
) -> Result<String, AppError> {
    let escaped = escape_like(search_term);
    let pattern = format!("%{}%", escaped);
    let mut query = Message::find()
        .filter(message::Column::SobaId.eq(room.id))
        .filter(Expr::cust_with_values(
            "LOWER(content) LIKE LOWER(?) ESCAPE '\\'",
            [pattern],
        ));
    if let Some(before_id) = before_id {
        query = query.filter(message::Column::Id.lt(before_id));
    }

    let mut messages = query
        .order_by_desc(message::Column::Id)
        .limit(SEARCH_RESULTS_PAGE_SIZE + 1)
        .all(db)
        .await?;
    let has_more = messages.len() as u64 > SEARCH_RESULTS_PAGE_SIZE;
    if has_more {
        messages.truncate(SEARCH_RESULTS_PAGE_SIZE as usize);
    }

    let sender_ids = messages
        .iter()
        .filter_map(|message| message.sender_id)
        .collect::<Vec<_>>();
    let clients = Client::find()
        .filter(client::Column::Id.is_in(sender_ids))
        .all(db)
        .await?;
    let sender_map = clients
        .into_iter()
        .map(|client| (client.id, client.username))
        .collect::<HashMap<_, _>>();

    let mut html = String::new();
    if before_id.is_none() {
        html.push_str(&format!(
            r#"<div class="search-summary">Rezultati za »{}«</div>"#,
            html_escape(search_term)
        ));
    }
    if messages.is_empty() && before_id.is_none() {
        html.push_str(
            r#"<div class="search-message">V tej sobi ni sporočil s tem besedilom.</div>"#,
        );
        return Ok(html);
    }

    for stored_message in &messages {
        let sender_name = stored_message
            .sender_id
            .and_then(|id| sender_map.get(&id))
            .map(String::as_str);
        html.push_str(&render_search_message(stored_message, sender_name));
    }

    if has_more && let Some(oldest) = messages.last() {
        html.push_str(&render_search_load_more_button(
            &room.name,
            search_term,
            oldest.id,
        ));
    }

    Ok(html)
}

fn render_search_load_more_button(room_name: &str, search_term: &str, before_id: i32) -> String {
    format!(
        r##"<form class="search-load-more"
              hx-get="/rooms/{room_name}/messages/search"
              hx-target="this"
              hx-swap="outerHTML">
          <input type="hidden" name="q" value="{search_term}">
          <input type="hidden" name="before_id" value="{before_id}">
          <button type="submit" class="load-older-btn">Naloži več rezultatov</button>
        </form>"##,
        room_name = html_escape(room_name),
        search_term = html_escape(search_term),
    )
}

async fn insert_message(
    db: &DatabaseConnection,
    room_id: i32,
    sender_id: Option<i64>,
    content: &str,
    reply_to_id: Option<i32>,
) -> Result<message::Model, AppError> {
    let msg = message::ActiveModel {
        sender_id: Set(sender_id),
        content: Set(content.to_string()),
        timestamp: Set(current_timestamp()),
        soba_id: Set(room_id),
        reply_to_id: Set(reply_to_id),
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

fn render_message(
    room_name: &str,
    msg: &message::Model,
    sender_name: Option<&str>,
    timestamp: i64,
    reactions: &BTreeMap<String, u32>,
    reply_preview: Option<&ReplyPreview>,
) -> String {
    let sender = sender_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("neznan uporabnik");

    let time_str = Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());

    let (sender_class, sender_id, delete_button) = message_owner_controls(msg);
    let quote_html = reply_preview
        .map(|p| {
            format!(
                r#"<div class="reply-quote"><span class="reply-quote-sender">{}</span><span class="reply-quote-text">{}</span></div>"#,
                html_escape(&p.sender_name),
                html_escape(&truncate_preview(&p.content)),
            )
        })
        .unwrap_or_default();

    let reply_button = format!(
        r##"<button type="button" class="message-reply-btn"
                aria-label="Odgovori na sporočilo"
                hx-get="/rooms/{room_name}/reply/{id}"
                hx-target="this"
                hx-swap="none">Odgovori</button>"##,
        room_name = html_escape(room_name),
        id = msg.id,
    );

    format!(
        r#"<div class="msg {sender_class}" id="msg-{id}" data-sender-id="{sender_id}">
  <div class="msg-head"><span class="msg-sender">{sender}</span><span class="time">{time}</span>{reply_button}{delete_button}</div>
  {quote_html}
  <div class="msg-text">{content}</div>
  <div class="reactions" id="reactions-{id}">{oznaka}</div>
  <div class="reaction-controls">{quick}{add_form}</div>
</div>"#,
        id = msg.id,
        sender_class = sender_class,
        sender_id = sender_id,
        sender = html_escape(sender),
        time = time_str,
        reply_button = reply_button,
        delete_button = delete_button,
        quote_html = quote_html,
        content = html_escape(&msg.content),
        oznaka = render_reaction_oznaka(msg.id, reactions),
        quick = render_quick_reaction_buttons(msg.id),
        add_form = render_reaction_add_form(msg.id),
    )
}

fn render_search_message(msg: &message::Model, sender_name: Option<&str>) -> String {
    let sender = sender_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("neznan uporabnik");
    let time_str = Local
        .timestamp_opt(msg.timestamp, 0)
        .single()
        .map(|dt| dt.format("%d. %m. %Y ob %H:%M").to_string())
        .unwrap_or_else(|| "neznan čas".to_string());
    let (sender_class, sender_id, delete_button) = message_owner_controls(msg);

    format!(
        r#"<div class="msg search-message-result {sender_class}" id="search-msg-{id}" data-sender-id="{sender_id}">
  <div class="msg-head"><span class="msg-sender">{sender}</span><span class="time">{time}</span>{delete_button}</div>
  <div class="msg-text">{content}</div>
</div>"#,
        id = msg.id,
        sender_class = sender_class,
        sender_id = sender_id,
        sender = html_escape(sender),
        time = time_str,
        delete_button = delete_button,
        content = html_escape(&msg.content),
    )
}

fn message_owner_controls(msg: &message::Model) -> (String, String, String) {
    match msg.sender_id {
        Some(sender_id) => (
            format!("sender-{sender_id}"),
            sender_id.to_string(),
            format!(
                r##"<button type="button" class="message-delete-btn"
                    aria-label="Izbriši sporočilo"
                    hx-delete="/messages/{message_id}"
                    hx-swap="none"
                    hx-confirm="Ali res želiš izbrisati to sporočilo?">Izbriši</button>"##,
                message_id = msg.id,
            ),
        ),
        None => ("sender-unknown".to_string(), String::new(), String::new()),
    }
}

fn render_message_oob(
    room_name: &str,
    msg: &message::Model,
    sender_name: Option<&str>,
    timestamp: i64,
    reply_preview: Option<&ReplyPreview>,
) -> String {
    format!(
        r#"<div id="messages" hx-swap-oob="afterbegin">{}</div>"#,
        render_message(
            room_name,
            msg,
            sender_name,
            timestamp,
            &BTreeMap::new(),
            reply_preview
        )
    )
}

fn render_message_deletion_oob(message_id: i32) -> String {
    format!(
        r#"<div id="msg-{message_id}" hx-swap-oob="delete"></div>
<div id="search-msg-{message_id}" hx-swap-oob="delete"></div>"#
    )
}

/// Po uspešno oddanem sporočilu je treba izprazniti polje za vnos samo pri
/// pošiljatelju — ne prek broadcast kanala sobe, ker bi to izbrisalo
/// nedokončano besedilo drugim uporabnikom, ki ravno takrat tipkajo.
pub fn render_message_input_reset() -> String {
    format!(
        r#"<textarea name="content" id="msg-input" rows="1" maxlength="{max_message_length}" placeholder="Sporočilo…" required hx-swap-oob="true"></textarea>
<div id="message-status" class="message-status" role="status" aria-live="polite" hx-swap-oob="true"></div>
{reply_clear}"#,
        max_message_length = MAX_MESSAGE_LENGTH,
        reply_clear = super::reply::render_reply_clear_oob(),
    )
}

pub fn render_rate_limit_warning() -> String {
    render_message_status(
        "warning",
        "Sporočila pošiljaš prehitro. Počakaj trenutek in poskusi znova.",
    )
}

pub fn render_message_error(message: &str) -> String {
    render_message_status("error", message)
}

fn render_message_status(kind: &str, message: &str) -> String {
    format!(
        r#"<div id="message-status" class="message-status {}" role="alert" aria-live="assertive" hx-swap-oob="true">{}</div>"#,
        html_escape(kind),
        html_escape(message)
    )
}
