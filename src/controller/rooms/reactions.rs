//! Shranjevanje, seštevanje in HTML-prikaz reakcij na sporočila.

use super::views::html_escape;
use crate::controller::web::AppError;
use crate::entities::prelude::{Message, MessageReactions};
use crate::entities::{message, message_reactions};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::collections::{BTreeMap, HashMap};

const QUICK_REACTIONS: [&str; 5] = ["👍", "❤️", "😂", "😮", "😢"];
pub async fn toggle_reaction(
    db: &DatabaseConnection,
    room_id: i32,
    user_id: i32,
    message_id: i32,
    emoji: &str,
) -> Result<Option<String>, AppError> {
    let emoji = emoji.trim();
    if emoji.is_empty() || emoji.chars().count() > 8 {
        return Ok(None);
    }
    let message_exists = Message::find_by_id(message_id)
        .filter(message::Column::SobaId.eq(room_id))
        .one(db)
        .await?
        .is_some();
    if !message_exists {
        return Ok(None);
    }

    let existing = MessageReactions::find()
        .filter(message_reactions::Column::MessageId.eq(message_id))
        .filter(message_reactions::Column::ClientId.eq(user_id))
        .filter(message_reactions::Column::Emoji.eq(emoji))
        .one(db)
        .await?;

    match existing {
        Some(reaction) => {
            MessageReactions::delete_by_id(reaction.id).exec(db).await?;
        }
        None => {
            message_reactions::ActiveModel {
                message_id: Set(message_id),
                client_id: Set(user_id),
                emoji: Set(emoji.to_string()),
                ..Default::default()
            }
            .insert(db)
            .await?;
        }
    }

    let counts = reaction_counts_for_message(db, message_id).await?;
    Ok(Some(render_reactions_oznaka_oob(message_id, &counts)))
}

async fn reaction_counts_for_message(
    db: &DatabaseConnection,
    message_id: i32,
) -> Result<BTreeMap<String, u32>, AppError> {
    let reactions = MessageReactions::find()
        .filter(message_reactions::Column::MessageId.eq(message_id))
        .all(db)
        .await?;

    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for r in &reactions {
        *counts.entry(r.emoji.clone()).or_insert(0) += 1;
    }
    Ok(counts)
}
//poizve za vse reakcije na enkar pri nalaganju starejših sporočil
pub(super) async fn reaction_counts_for_messages(
    db: &DatabaseConnection,
    message_ids: &[i32],
) -> Result<HashMap<i32, BTreeMap<String, u32>>, AppError> {
    let reactions = MessageReactions::find()
        .filter(message_reactions::Column::MessageId.is_in(message_ids.to_vec()))
        .all(db)
        .await?;

    let mut map: HashMap<i32, BTreeMap<String, u32>> = HashMap::new();
    for r in &reactions {
        *map.entry(r.message_id)
            .or_default()
            .entry(r.emoji.clone())
            .or_insert(0) += 1;
    }
    Ok(map)
}

//da se izpiše število in kateri emoji pod sporočilom
fn render_reactions_oznaka_oob(message_id: i32, counts: &BTreeMap<String, u32>) -> String {
    format!(
        r#"<div class="reactions" id="reactions-{id}" hx-swap-oob="innerHTML">{oznaka}</div>"#,
        id = message_id,
        oznaka = render_reaction_oznaka(message_id, counts),
    )
}

pub(super) fn render_reaction_oznaka(message_id: i32, counts: &BTreeMap<String, u32>) -> String {
    let mut oznaka = String::new();
    for (emoji, count) in counts {
        oznaka.push_str(&format!(
            r#"<form class="reaction-pill-form" ws-send>
    <input type="hidden" name="reaction_message_id" value="{id}">
    <input type="hidden" name="reaction_emoji" value="{emoji}">
    <button type="submit" class="reaction-pill">{emoji} {count}</button>
  </form>"#,
            id = message_id,
            emoji = html_escape(emoji),
            count = count,
        ));
    }
    oznaka
}

pub(super) fn render_quick_reaction_buttons(message_id: i32) -> String {
    let mut html = String::new();
    for emoji in QUICK_REACTIONS {
        html.push_str(&format!(
            r#"<form class="reaction-quick-form" ws-send>
    <input type="hidden" name="reaction_message_id" value="{id}">
    <input type="hidden" name="reaction_emoji" value="{emoji}">
    <button type="submit" class="reaction-quick-btn">{emoji}</button>
  </form>"#,
            id = message_id,
            emoji = emoji,
        ));
    }
    html
}

pub(super) fn render_reaction_add_form(message_id: i32) -> String {
    format!(
        r#"<form class="reaction-add-form" ws-send>
    <input type="hidden" name="reaction_message_id" value="{id}">
    <input type="text" name="reaction_emoji" maxlength="8"
      placeholder="+ drugo" class="reaction-add-input">
  </form>"#,
        id = message_id,
    )
}
