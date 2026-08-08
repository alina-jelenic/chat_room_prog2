//! Osnovna logika sob: ustvarjanje, članstvo, dostop in brisanje sob.
//! Sporočila, prikaz HTML in reakcije so razdeljeni v podmodule.

use crate::controller::auth::{AuthUser, require_auth};
use crate::controller::tipi::{RoomAccessRevokedReason, SharedState};
use crate::controller::web::AppError;
use crate::entities::prelude::{Message, RoomMember, Soba};
use crate::entities::{message, room_member, soba};
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, TransactionTrait,
};
use serde::Deserialize;

mod messages;
mod reactions;
pub mod reply;
mod views;

pub use messages::{
    create_websocket_message, delete_message, list_messages, render_message_error,
    render_message_input_reset, render_rate_limit_warning, search_messages,
    validate_message_content,
};
pub use reactions::toggle_reaction;
pub use views::render_kicked_redirect;

use views::{
    notify_room_access_revoked, notify_room_deleted, render_chat_panel, render_chat_panel_oob,
    render_room_action_message, render_room_action_message_oob, render_room_list,
    render_room_list_oob, render_room_members, room_action_response, room_action_retarget_response,
    room_error_panel,
};

#[derive(Debug, Deserialize)]
pub struct CreateRoomForm {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JoinRoomParams {
    pub id: Option<String>,
}

fn db_from_state(state: &SharedState) -> Result<DatabaseConnection, AppError> {
    // Pomembno: mutex držimo samo toliko časa, da kloniramo DatabaseConnection.
    // Nikoli ne držimo locka čez .await, ker lahko to hitro povzroči čudne blokade.
    Ok(state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
        .db
        .clone())
}

// Axumov Response namenoma vrnemo neposredno, da handler ohrani status in HTMX glave.
#[allow(clippy::result_large_err)]
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
    ensure_room_exists(db, "general", None).await?;
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
fn is_unique_violation(err: &sea_orm::DbErr) -> bool {
    err.to_string().contains("UNIQUE constraint failed")
}

const MAX_ID_ATTEMPTS: u32 = 5;

async fn ensure_room_exists(
    db: &DatabaseConnection,
    name: &str,
    owner_id: Option<i32>,
) -> Result<soba::Model, AppError> {
    let clean_name = normalize_room_name(name)?;

    if let Some(room) = Soba::find()
        .filter(soba::Column::Name.eq(&clean_name))
        .one(db)
        .await?
    {
        return Ok(room);
    }

    use rand::Rng;

    for attempt in 0..MAX_ID_ATTEMPTS {
        let code = rand::thread_rng().gen_range(100_000..=999_999);

        let result = soba::ActiveModel {
            id: Set(code),
            name: Set(clean_name.clone()),
            owner_id: Set(owner_id),
        }
        .insert(db)
        .await;

        match result {
            Ok(room) => {
                return Ok(room);
            }
            Err(e) if is_unique_violation(&e) => {
                if attempt + 1 == MAX_ID_ATTEMPTS {
                    return Err(AppError(
                        "Sobe trenutno ni bilo mogoče ustvariti, poskusi znova.".to_string(),
                    ));
                }
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    unreachable!("zanka se vedno konča z return-om na zadnjem poskusu")
}

async fn ensure_room_membership<C>(db: &C, room_id: i32, user_id: i32) -> Result<bool, AppError>
where
    C: ConnectionTrait,
{
    let already_joined = RoomMember::find()
        .filter(room_member::Column::SobaId.eq(room_id))
        .filter(room_member::Column::ClientId.eq(user_id))
        .one(db)
        .await?
        .is_some();

    if already_joined {
        return Ok(false);
    }

    room_member::ActiveModel {
        soba_id: Set(room_id),
        client_id: Set(user_id),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(true)
}

async fn is_room_member<C>(db: &C, room_id: i32, user_id: i32) -> Result<bool, AppError>
where
    C: ConnectionTrait,
{
    Ok(RoomMember::find()
        .filter(room_member::Column::SobaId.eq(room_id))
        .filter(room_member::Column::ClientId.eq(user_id))
        .one(db)
        .await?
        .is_some())
}

pub async fn user_can_access_room(
    db: &DatabaseConnection,
    room: &soba::Model,
    user_id: i32,
) -> Result<bool, AppError> {
    if room.name == "general" {
        return Ok(true);
    }

    is_room_member(db, room.id, user_id).await
}

enum RoomCreationError {
    NameTaken,
    Other(AppError),
}

impl From<sea_orm::DbErr> for RoomCreationError {
    fn from(e: sea_orm::DbErr) -> Self {
        RoomCreationError::Other(AppError(e.to_string()))
    }
}

impl From<AppError> for RoomCreationError {
    fn from(e: AppError) -> Self {
        RoomCreationError::Other(e)
    }
}

async fn create_owned_room(
    db: &DatabaseConnection,
    name: String,
    owner_id: i32,
) -> Result<soba::Model, RoomCreationError> {
    use rand::Rng;

    for attempt in 0..MAX_ID_ATTEMPTS {
        let code = rand::thread_rng().gen_range(100_000..=999_999);
        let transaction = db.begin().await?;
        let result = soba::ActiveModel {
            id: Set(code),
            name: Set(name.clone()),
            owner_id: Set(Some(owner_id)),
        }
        .insert(&transaction)
        .await;

        match result {
            Ok(room) => {
                ensure_room_membership(&transaction, room.id, owner_id).await?;
                transaction.commit().await?;
                return Ok(room);
            }
            Err(error) if is_unique_violation(&error) => {
                transaction.rollback().await?;
                //ločimo težavo med trk id ali imena
                let name_taken = Soba::find()
                    .filter(soba::Column::Name.eq(&name))
                    .one(db)
                    .await?
                    .is_some();

                if name_taken {
                    return Err(RoomCreationError::NameTaken);
                }
                if attempt + 1 == MAX_ID_ATTEMPTS {
                    return Err(RoomCreationError::Other(AppError(
                        "Sobe trenutno ni bilo mogoče ustvariti, poskusi znova.".to_string(),
                    )));
                }
            }
            Err(error) => {
                transaction.rollback().await?;
                return Err(error.into());
            }
        }
    }

    unreachable!("zanka se vedno konča z return-om na zadnjem poskusu")
}

pub fn normalize_room_name(name: &str) -> Result<String, AppError> {
    let clean = name.trim().to_lowercase();

    if clean.is_empty() {
        return Err(AppError("Ime sobe ne sme biti prazno.".to_string()));
    }

    if clean.len() > 32 {
        return Err(AppError("Ime sobe je predolgo.".to_string()));
    }

    // Nabor znakov je namenoma omejen, da se izognemo težavam v URL-jih,
    // izbirnikih in poizvedbenem parametru povezave WebSocket.
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
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    ensure_default_room(&db).await?;

    Ok(Html(render_room_list(&db, user.id, "general").await?).into_response())
}

pub async fn create_room(
    jar: CookieJar,
    State(state): State<SharedState>,
    Form(form): Form<CreateRoomForm>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let clean_name = match normalize_room_name(form.name.as_deref().unwrap_or_default()) {
        Ok(name) => name,
        Err(error) => return Ok(room_action_response("error", &error.0)),
    };

    if room_for_websocket(&db, &clean_name).await?.is_some() {
        return Ok(room_action_response(
            "error",
            "Soba s tem imenom že obstaja. Pridruži se ji z njenim ID-jem.",
        ));
    }

    let room = match create_owned_room(&db, clean_name, user.id).await {
        Ok(room) => room,
        Err(RoomCreationError::NameTaken) => {
            return Ok(room_action_response(
                "error",
                "Soba s tem imenom že obstaja. Pridruži se ji z njenim ID-jem.",
            ));
        }
        Err(RoomCreationError::Other(e)) => return Err(e),
    };
    let room_list = render_room_list(&db, user.id, &room.name).await?;
    let mut html =
        render_room_action_message("success", &format!("Soba #{} je ustvarjena.", room.name));
    html.push_str(&render_chat_panel_oob(&room, &user));
    html.push_str(&render_room_list_oob(&room_list));

    Ok(Html(html).into_response())
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
    let room = match room_for_websocket(&db, &room_name).await {
        Err(error) => return Ok(room_error_panel(&error.0)),
        Ok(Some(room)) => room,
        Ok(None) => return Ok(room_error_panel("Soba ne obstaja.")),
    };

    if !user_can_access_room(&db, &room, user.id).await? {
        return Ok(room_error_panel(
            "Do te sobe še nimaš dostopa. Uporabi njen ID v obrazcu »Pridruži se«.",
        ));
    };

    let mut html = render_chat_panel(&room, &user);

    // Ko uporabnik zamenja sobo, hkrati posodobimo še aktiven gumb v seznamu sob.
    html.push_str(&format!(
        r#"<div id="room-list" hx-swap-oob="innerHTML">{}</div>"#,
        render_room_list(&db, user.id, &room.name).await?
    ));
    html.push_str(r#"<div id="room-action-msg" hx-swap-oob="innerHTML"></div>"#);

    Ok(Html(html).into_response())
}

pub async fn join_room(
    jar: CookieJar,
    State(state): State<SharedState>,
    Form(params): Form<JoinRoomParams>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let db = db_from_state(&state)?;
    let room_id = match params.id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => match id.parse::<i32>() {
            Ok(id) => id,
            Err(_) => {
                return Ok(room_action_response(
                    "error",
                    "ID sobe mora biti veljavno število.",
                ));
            }
        },
        _ => return Ok(room_action_response("error", "Vnesi ID sobe.")),
    };

    let room = match Soba::find()
        .filter(soba::Column::Id.eq(room_id))
        .one(&db)
        .await?
    {
        Some(room) => room,
        None => {
            return Ok(room_action_response(
                "error",
                "Soba s tem ID-jem ne obstaja.",
            ));
        }
    };

    let joined_now = if room.name == "general" {
        false
    } else {
        ensure_room_membership(&db, room.id, user.id).await?
    };
    let room = if room.owner_id.is_none() && room.name != "general" {
        let mut active: soba::ActiveModel = room.into();
        active.owner_id = Set(Some(user.id));
        active.update(&db).await?
    } else {
        room
    };

    let room_list = render_room_list(&db, user.id, &room.name).await?;
    let message = if joined_now {
        format!("Zdaj si v sobi #{}.", room.name)
    } else {
        format!("Že si v sobi #{}.", room.name)
    };

    let mut html =
        render_room_action_message(if joined_now { "success" } else { "info" }, &message);
    html.push_str(&render_chat_panel_oob(&room, &user));
    html.push_str(&render_room_list_oob(&room_list));
    Ok(Html(html).into_response())
}

pub async fn leave_room(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path(room_name): Path<String>,
) -> Result<Response, AppError> {
    let user = match authenticated_user(&jar, &state) {
        Ok(user) => user,
        Err(response) => return Ok(response),
    };

    let clean_name = match normalize_room_name(&room_name) {
        Ok(name) => name,
        Err(error) => return Ok(room_action_retarget_response("error", &error.0)),
    };
    if clean_name == "general" {
        return Ok(room_action_retarget_response(
            "error",
            "Sobe general ni mogoče zapustiti.",
        ));
    }

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &clean_name).await? {
        Some(room) => room,
        None => {
            return Ok(room_action_retarget_response(
                "error",
                "Soba ne obstaja ali je bila že izbrisana.",
            ));
        }
    };

    if room.owner_id == Some(user.id) {
        return Ok(room_action_retarget_response(
            "error",
            "Sobe kot njen lastnik ne moreš zapustiti; lahko jo izbrišeš.",
        ));
    }

    let result = RoomMember::delete_many()
        .filter(room_member::Column::SobaId.eq(room.id))
        .filter(room_member::Column::ClientId.eq(user.id))
        .exec(&db)
        .await?;
    if result.rows_affected == 0 {
        return Ok(room_action_retarget_response(
            "error",
            "Do te sobe nimaš dostopa.",
        ));
    }

    // O morebitnem odhodu iz drugega zavihka obvestimo vse uporabnikove
    // obstoječe WebSocket povezave v tej sobi. Tako se zaprejo tudi pasivne
    // povezave, ki po odhodu ne pošljejo nobenega novega sporočila.
    notify_room_access_revoked(&state, room.id, user.id, RoomAccessRevokedReason::Left)?;

    let general = ensure_room_exists(&db, "general", None).await?;
    let room_list = render_room_list(&db, user.id, &general.name).await?;
    let mut html = render_chat_panel(&general, &user);
    html.push_str(&render_room_list_oob(&room_list));
    html.push_str(&render_room_action_message_oob(
        "success",
        &format!("Nisi več v sobi #{}.", room.name),
    ));

    Ok(Html(html).into_response())
}

pub async fn list_room_members(
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
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    if room.owner_id != Some(user.id) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    Ok(Html(render_room_members(&db, &room).await?).into_response())
}

pub async fn kick_room_member(
    jar: CookieJar,
    State(state): State<SharedState>,
    Path((room_name, user_id)): Path<(String, i32)>,
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
    if room.owner_id != Some(user.id) {
        return Ok((
            StatusCode::FORBIDDEN,
            "Uporabnika lahko izžene samo lastnik sobe.",
        )
            .into_response());
    }
    if user_id == user.id {
        return Ok((
            StatusCode::BAD_REQUEST,
            "Lastnik ne more izgnati samega sebe.",
        )
            .into_response());
    }

    let result = RoomMember::delete_many()
        .filter(room_member::Column::SobaId.eq(room.id))
        .filter(room_member::Column::ClientId.eq(user_id))
        .exec(&db)
        .await?;
    if result.rows_affected == 0 {
        return Ok(Html(render_room_members(&db, &room).await?).into_response());
    }

    notify_room_access_revoked(&state, room.id, user_id, RoomAccessRevokedReason::Kicked)?;

    Ok(Html(render_room_members(&db, &room).await?).into_response())
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

    let clean_name = match normalize_room_name(&room_name) {
        Ok(name) => name,
        Err(error) => return Ok(room_action_retarget_response("error", &error.0)),
    };
    if clean_name == "general" {
        return Ok((StatusCode::BAD_REQUEST, "Sobe general ni mogoče izbrisati.").into_response());
    }

    let db = db_from_state(&state)?;
    let room = match room_for_websocket(&db, &clean_name).await? {
        Some(room) => room,
        None => {
            return Ok(room_action_retarget_response(
                "error",
                "Soba ne obstaja ali je bila že izbrisana.",
            ));
        }
    };

    if room.owner_id != Some(user.id) {
        return Ok(room_action_retarget_response(
            "error",
            "Sobo lahko izbriše samo uporabnik, ki jo je ustvaril.",
        ));
    }

    // Članstva, sporočila in sobo izbrišemo v isti transakciji, da v bazi ne
    // more ostati napol izvedena operacija.
    let transaction = db.begin().await?;
    RoomMember::delete_many()
        .filter(room_member::Column::SobaId.eq(room.id))
        .exec(&transaction)
        .await?;
    Message::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&transaction)
        .await?;
    Soba::delete_by_id(room.id).exec(&transaction).await?;
    transaction.commit().await?;

    let general = ensure_room_exists(&db, "general", None).await?;
    let room_list = render_room_list(&db, user.id, &general.name).await?;
    notify_room_deleted(&state, room.id, &room.name)?;

    let mut html = render_chat_panel(&general, &user);
    html.push_str(&render_room_list_oob(&room_list));
    Ok(Html(html).into_response())
}
