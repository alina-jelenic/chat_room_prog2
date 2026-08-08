//! Nastavljanje in čiščenje sporočila, na katerega uporabnik odgovarja.

use std::collections::HashMap;

use super::{authenticated_user, db_from_state, room_for_websocket, user_can_access_room};
use crate::controller::tipi::SharedState;
use crate::controller::util::html_escape;
use crate::controller::web::AppError;
use crate::entities::prelude::{Client, Message};
use crate::entities::{client, message};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

pub async fn set_reply_target(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path((room_name, message_id)): Path<(String, i32)>,
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

    let target = match Message::find_by_id(message_id)
        .filter(message::Column::SobaId.eq(room.id))
        .one(&db)
        .await?
    {
        Some(target) => target,
        // Sporočilo je bilo medtem izbrisano — banner pač počistimo.
        None => return Ok(Html(render_reply_clear_oob()).into_response()),
    };

    let sender_name = match target.sender_id {
        Some(id) => Client::find_by_id(id as i32)
            .one(&db)
            .await?
            .map(|c| c.username)
            .unwrap_or_else(|| "neznan uporabnik".to_string()),
        None => "neznan uporabnik".to_string(),
    };

    Ok(Html(render_reply_target_oob(
        target.id,
        &sender_name,
        &target.content,
    ))
    .into_response())
}

pub async fn clear_reply_target() -> Response {
    Html(render_reply_clear_oob()).into_response()
}

fn render_reply_target_oob(message_id: i32, sender: &str, content: &str) -> String {
    format!(
        r#"<input type="hidden" name="reply_to_id" id="reply-to-input" value="{message_id}" hx-swap-oob="true">
<div id="reply-banner" class="reply-banner" hx-swap-oob="true">
  <span class="reply-banner-text" id="reply-banner-text">Odgovarjaš <strong>{sender}</strong>: {preview}</span>
  <button type="button" class="reply-cancel-btn" id="reply-cancel-btn" aria-label="Prekliči odgovor"
      hx-get="/reply/clear" hx-target="this" hx-swap="none">✕</button>
</div>"#,
        message_id = message_id,
        sender = html_escape(sender),
        preview = html_escape(&truncate_preview(content)),
    )
}

pub(super) fn render_reply_clear_oob() -> String {
    r#"<input type="hidden" name="reply_to_id" id="reply-to-input" value="" hx-swap-oob="true">
<div id="reply-banner" class="reply-banner" hidden hx-swap-oob="true">
  <span class="reply-banner-text" id="reply-banner-text"></span>
  <button type="button" class="reply-cancel-btn" id="reply-cancel-btn" aria-label="Prekliči odgovor"
      hx-get="/reply/clear" hx-target="this" hx-swap="none">✕</button>
</div>"#
        .to_string()
}

pub struct ReplyPreview {
    pub(crate) sender_name: String,
    pub(crate) content: String,
}

pub async fn reply_previews_for_messages(
    db: &DatabaseConnection,
    messages: &[message::Model],
) -> Result<HashMap<i32, ReplyPreview>, AppError> {
    let reply_ids: Vec<i32> = messages.iter().filter_map(|m| m.reply_to_id).collect();
    if reply_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let targets = Message::find()
        .filter(message::Column::Id.is_in(reply_ids))
        .all(db)
        .await?;

    let sender_ids: Vec<i64> = targets.iter().filter_map(|t| t.sender_id).collect();
    let clients = Client::find()
        .filter(client::Column::Id.is_in(sender_ids))
        .all(db)
        .await?;
    let sender_map: HashMap<i64, String> = clients
        .into_iter()
        .map(|c| (c.id as i64, c.username))
        .collect();

    Ok(targets
        .into_iter()
        .map(|t| {
            let sender_name = t
                .sender_id
                .and_then(|id| sender_map.get(&id).cloned())
                .unwrap_or_else(|| "neznan uporabnik".to_string());
            (
                t.id,
                ReplyPreview {
                    sender_name,
                    content: t.content,
                },
            )
        })
        .collect())
}

pub fn truncate_preview(content: &str) -> String {
    const MAX_PREVIEW_LENGTH: usize = 60;
    if content.chars().count() <= MAX_PREVIEW_LENGTH {
        content.to_string()
    } else {
        let truncated: String = content.chars().take(MAX_PREVIEW_LENGTH).collect();
        format!("{}…", truncated)
    }
}
