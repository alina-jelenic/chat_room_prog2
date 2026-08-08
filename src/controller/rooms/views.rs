//! HTML-prikaz sob in članov ter realnočasovna obvestila o spremembah dostopa.

use super::messages::{MAX_MESSAGE_LENGTH, MAX_SEARCH_LENGTH};
use super::room_for_websocket;
use crate::controller::auth::AuthUser;
use crate::controller::tipi::{RoomAccessRevokedReason, SharedState};
use crate::controller::web::AppError;
use crate::entities::prelude::{Client, RoomMember, Soba};
use crate::entities::{client, room_member, soba};
use axum::response::{Html, IntoResponse, Response};
use crate::controller::util::html_escape;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
pub(super) async fn render_room_list(
    db: &DatabaseConnection,
    user_id: i32,
    active_room_name: &str,
) -> Result<String, AppError> {
    let mut room_ids = RoomMember::find()
        .filter(room_member::Column::ClientId.eq(user_id))
        .all(db)
        .await?
        .into_iter()
        .map(|membership| membership.soba_id)
        .collect::<Vec<_>>();

    if let Some(general) = room_for_websocket(db, "general").await? {
        room_ids.push(general.id);
    }
    room_ids.sort_unstable();
    room_ids.dedup();

    let rooms = Soba::find()
        .filter(soba::Column::Id.is_in(room_ids))
        .order_by_asc(soba::Column::Name)
        .all(db)
        .await?;

    let mut html = String::new();
    for room in rooms {
        html.push_str(&render_room_button(&room, room.name == active_room_name));
    }

    Ok(html)
}

pub(super) async fn render_room_members(
    db: &DatabaseConnection,
    room: &soba::Model,
) -> Result<String, AppError> {
    let memberships = RoomMember::find()
        .filter(room_member::Column::SobaId.eq(room.id))
        .all(db)
        .await?;
    let member_ids = memberships
        .into_iter()
        .map(|membership| membership.client_id)
        .collect::<Vec<_>>();
    let members = Client::find()
        .filter(client::Column::Id.is_in(member_ids))
        .order_by_asc(client::Column::Username)
        .all(db)
        .await?;

    let mut html = r#"<div class="members-title">Člani sobe</div>"#.to_string();
    if members.len() <= 1 {
        html.push_str(r#"<div class="members-empty">V sobi še ni drugih članov.</div>"#);
    }

    for member in members {
        let username = html_escape(&member.username);
        if room.owner_id == Some(member.id) {
            html.push_str(&format!(
                r#"<div class="member-row"><span>{username}</span><span class="owner-label">lastnik</span></div>"#
            ));
        } else {
            html.push_str(&format!(
                r##"<div class="member-row">
                  <span>{username}</span>
                  <button type="button" class="kick-member-btn"
                      hx-delete="/rooms/{room_name}/members/{user_id}"
                      hx-target="#room-members"
                      hx-swap="innerHTML"
                      hx-confirm="Ali res želiš izgnati uporabnika {username} iz sobe #{room_name}?">
                    Izženi
                  </button>
                </div>"##,
                room_name = html_escape(&room.name),
                user_id = member.id,
            ));
        }
    }

    Ok(html)
}

pub(super) fn render_chat_panel(room: &soba::Model, user: &AuthUser) -> String {
    render_chat_panel_variant(room, user, false)
}

pub(super) fn render_chat_panel_oob(room: &soba::Model, user: &AuthUser) -> String {
    render_chat_panel_variant(room, user, true)
}

fn render_chat_panel_variant(room: &soba::Model, user: &AuthUser, oob: bool) -> String {
    let name = html_escape(&room.name);
    let username = html_escape(&user.username);
    let id = room.id;
    let oob_attribute = if oob {
        r#" hx-swap-oob="outerHTML""#
    } else {
        ""
    };
    let room_control = if room.name == "general" {
        String::new()
    } else if room.owner_id == Some(user.id) {
        format!(
            r##"<button type="button" class="delete-room-btn"
                hx-delete="/rooms/{name}"
                hx-target="#chat-panel"
                hx-swap="outerHTML"
                hx-confirm="Ali res želiš izbrisati sobo #{name} in vsa njena sporočila?">
              Izbriši sobo
            </button>"##
        )
    } else {
        format!(
            r##"<button type="button" class="leave-room-btn"
                hx-delete="/rooms/{name}/membership"
                hx-target="#chat-panel"
                hx-swap="outerHTML"
                hx-confirm="Ali res želiš zapustiti sobo #{name}?">
              Zapusti sobo
            </button>"##
        )
    };
    let room_members = if room.owner_id == Some(user.id) {
        format!(
            r##"<section class="room-members" id="room-members"
                 hx-get="/rooms/{name}/members"
                 hx-trigger="load"
                 hx-swap="innerHTML">
              <div class="members-empty">Nalagam člane …</div>
            </section>"##
        )
    } else {
        String::new()
    };

    format!(
        r##"
<div class="main current-user-{user_id}" id="chat-panel"{oob_attribute}
     data-current-user-id="{user_id}"
     data-current-username="{username}"
     hx-ext="ws" ws-connect="/ws?room_name={name}">
  <style>.current-user-{user_id} .sender-{user_id} .message-delete-btn {{ display: inline-flex; }}</style>
  <div class="chat-header">
    <span class="chat-header-hash">#</span>
    <span class="chat-header-name" id="room-title">{name}</span>
    <span class="room-id" style="font-size:0.7rem; color:var(--muted); margin-left:8px; background:rgba(0,0,0,0.05); padding:2px 8px; border-radius:10px;">ID: {id}</span>
    <span class="connection-status connecting" data-connection-status role="status" aria-live="polite">Povezujem …</span>
    {room_control}
  </div>

  {room_members}

  <div class="message-search">
    <form class="message-search-form"
          hx-get="/rooms/{name}/messages/search"
          hx-target="#message-search-results"
          hx-swap="innerHTML">
      <input type="search" name="q" maxlength="{max_search_length}"
             placeholder="Išči po zgodovini …" aria-label="Išči po zgodovini sporočil" required>
      <button type="submit" class="search-btn">Išči</button>
      <button type="reset" class="clear-search-btn"
              hx-get="/rooms/{name}/messages/search"
              hx-params="none"
              hx-target="#message-search-results"
              hx-swap="innerHTML">Počisti</button>
    </form>
    <div class="message-search-results" id="message-search-results" aria-live="polite"></div>
  </div>

  <div class="messages" id="messages"
    hx-get="/rooms/{name}/messages"
    hx-trigger="load"
    hx-swap="innerHTML">
    <div class="sys-msg">Nalaganje sporočil za #{name}…</div>
  </div>

  <div class="input-area">
    <div id="message-status" class="message-status" role="status" aria-live="polite"></div>
    <div id="reply-banner" class="reply-banner" hidden>
      <span class="reply-banner-text" id="reply-banner-text"></span>
      <button type="button" class="reply-cancel-btn" id="reply-cancel-btn" aria-label="Prekliči odgovor">✕</button>
    </div>
    <form id="msg-form" ws-send>
      <input type="hidden" name="reply_to_id" id="reply-to-input" value="">
      <div class="input-row">
        <textarea name="content" id="msg-input" rows="1" maxlength="{max_message_length}" placeholder="Sporočilo…" required></textarea>
        <button type="submit" class="send-btn" aria-label="Pošlji" disabled>
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
        oob_attribute = oob_attribute,
        room_control = room_control,
        room_members = room_members,
        max_message_length = MAX_MESSAGE_LENGTH,
        max_search_length = MAX_SEARCH_LENGTH,
    )
}

pub(super) fn render_room_list_oob(room_list: &str) -> String {
    format!(r#"<div id="room-list" hx-swap-oob="innerHTML">{room_list}</div>"#)
}

pub(super) fn render_room_action_message_oob(kind: &str, message: &str) -> String {
    format!(
        r#"<div id="room-action-msg" hx-swap-oob="innerHTML">{}</div>"#,
        render_room_action_message(kind, message)
    )
}

fn render_room_list_reload_oob() -> String {
    r#"<div class="room-list" id="room-list" hx-swap-oob="outerHTML"
         hx-get="/rooms" hx-trigger="load" hx-swap="innerHTML"></div>"#
        .to_string()
}

pub(super) fn render_room_action_message(kind: &str, message: &str) -> String {
    format!(
        r#"<div class="room-action-message {}">{}</div>"#,
        html_escape(kind),
        html_escape(message)
    )
}

pub(super) fn room_action_response(kind: &str, message: &str) -> Response {
    Html(render_room_action_message(kind, message)).into_response()
}

pub(super) fn room_action_retarget_response(kind: &str, message: &str) -> Response {
    (
        [
            ("HX-Retarget", "#room-action-msg"),
            ("HX-Reswap", "innerHTML"),
        ],
        Html(render_room_action_message(kind, message)),
    )
        .into_response()
}

pub(super) fn room_error_panel(message: &str) -> Response {
    Html(format!(
        r#"<div class="main" id="chat-panel">
          <div class="room-panel-error">{}</div>
        </div>"#,
        html_escape(message)
    ))
    .into_response()
}

pub(super) fn notify_room_deleted(
    state: &SharedState,
    deleted_room_id: i32,
    deleted_room_name: &str,
) -> Result<(), AppError> {
    let (deleted_room_sender, other_senders) = {
        let mut state = state
            .lock()
            .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?;
        let deleted = state.soba_tx.remove(&deleted_room_id);
        let others = state.soba_tx.values().cloned().collect::<Vec<_>>();
        (deleted, others)
    };

    let room_list_oob = render_room_list_reload_oob();
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

pub(super) fn broadcast_room_html(
    state: &SharedState,
    room_id: i32,
    html: String,
) -> Result<(), AppError> {
    let sender = state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
        .get_or_create_room_tx(room_id);
    let _ = sender.send(html);
    Ok(())
}

pub(super) fn notify_room_access_revoked(
    state: &SharedState,
    room_id: i32,
    user_id: i32,
    reason: RoomAccessRevokedReason,
) -> Result<(), AppError> {
    state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
        .revoke_room_access(room_id, user_id, reason);

    Ok(())
}

pub fn render_kicked_redirect(room_name: &str) -> String {
    let room_name = html_escape(room_name);
    format!(
        r##"<div class="main" id="chat-panel" hx-swap-oob="outerHTML"
             hx-get="/rooms/general/panel" hx-trigger="load" hx-swap="outerHTML">
          <div class="sys-msg">Lastnik te je izgnal iz sobe #{room_name}. Odpiram #general …</div>
        </div>{}"##,
        render_room_list_reload_oob()
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
