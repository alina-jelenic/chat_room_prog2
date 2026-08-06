use crate::controller::auth::{AuthUser, require_auth};
use crate::controller::tipi::{RoomAccessRevokedReason, SharedState};
use crate::controller::web::AppError;
use crate::entities::prelude::{Client, Message, RoomMember, Soba};
use crate::entities::{client, message, room_member, soba};
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Local, TimeZone};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
pub struct CreateRoomForm {
    pub name: Option<String>,
}

const MAX_MESSAGE_LENGTH: usize = 2000;

const MESSAGES_PAGE_SIZE: u64 = 50;
const SEARCH_RESULTS_PAGE_SIZE: u64 = 30;
const MAX_SEARCH_LENGTH: usize = 100;

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub before_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SearchMessagesQuery {
    pub q: Option<String>,
    pub before_id: Option<i32>,
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
    if stored_message.sender_id != Some(user.id as i64) {
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
    validate_message_content(content)?;

    let room_still_exists = Soba::find_by_id(room_id).one(db).await?.is_some();
    if !room_still_exists {
        // Soba je bila med tem, ko je uporabnik tipkal, že izbrisana.
        // Sporočila ne shranimo — vrnemo prazen niz, enako kot pri praznem sporočilu.
        return Ok(String::new());
    }

    let msg = insert_message(db, room_id, Some(user.id as i64), content).await?;
    Ok(render_message_oob(
        &msg,
        Some(&user.username),
        msg.timestamp,
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

async fn render_room_list(
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
        .map(|client| (client.id as i64, client.username))
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

        blocks.push(render_message(msg, sender_name, msg.timestamp));
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

async fn render_message_search_page(
    db: &DatabaseConnection,
    room: &soba::Model,
    search_term: &str,
    before_id: Option<i32>,
) -> Result<String, AppError> {
    let mut query = Message::find()
        .filter(message::Column::SobaId.eq(room.id))
        .filter(message::Column::Content.contains(search_term));
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
        .map(|client| (client.id as i64, client.username))
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
        html.push_str(&render_message_variant(
            stored_message,
            sender_name,
            stored_message.timestamp,
            "search-message",
        ));
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

async fn render_room_members(
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
    render_chat_panel_variant(room, user, false)
}

fn render_chat_panel_oob(room: &soba::Model, user: &AuthUser) -> String {
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
    <form id="msg-form" ws-send>
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

fn render_room_list_oob(room_list: &str) -> String {
    format!(r#"<div id="room-list" hx-swap-oob="innerHTML">{room_list}</div>"#)
}

fn render_room_action_message_oob(kind: &str, message: &str) -> String {
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

fn render_room_action_message(kind: &str, message: &str) -> String {
    format!(
        r#"<div class="room-action-message {}">{}</div>"#,
        html_escape(kind),
        html_escape(message)
    )
}

fn room_action_response(kind: &str, message: &str) -> Response {
    Html(render_room_action_message(kind, message)).into_response()
}

fn room_action_retarget_response(kind: &str, message: &str) -> Response {
    (
        [
            ("HX-Retarget", "#room-action-msg"),
            ("HX-Reswap", "innerHTML"),
        ],
        Html(render_room_action_message(kind, message)),
    )
        .into_response()
}

fn room_error_panel(message: &str) -> Response {
    Html(format!(
        r#"<div class="main" id="chat-panel">
          <div class="room-panel-error">{}</div>
        </div>"#,
        html_escape(message)
    ))
    .into_response()
}

fn notify_room_deleted(
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

fn broadcast_room_html(state: &SharedState, room_id: i32, html: String) -> Result<(), AppError> {
    let sender = state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
        .get_or_create_room_tx(room_id);
    let _ = sender.send(html);
    Ok(())
}

fn notify_room_access_revoked(
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

fn render_message(msg: &message::Model, sender_name: Option<&str>, timestamp: i64) -> String {
    render_message_variant(msg, sender_name, timestamp, "message")
}

fn render_message_variant(
    msg: &message::Model,
    sender_name: Option<&str>,
    timestamp: i64,
    id_prefix: &str,
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

    let (sender_class, sender_id, delete_button) = match msg.sender_id {
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
    };

    format!(
        r#"<div class="msg {sender_class}" id="{id_prefix}-{message_id}" data-sender-id="{sender_id}">
  <div class="msg-head"><span class="msg-sender">{sender}</span><span class="time">{time}</span>{delete_button}</div>
  <div class="msg-text">{content}</div>
</div>"#,
        sender_class = sender_class,
        id_prefix = html_escape(id_prefix),
        message_id = msg.id,
        sender_id = sender_id,
        sender = html_escape(sender),
        time = time_str,
        delete_button = delete_button,
        content = html_escape(&msg.content),
    )
}

fn render_message_oob(msg: &message::Model, sender_name: Option<&str>, timestamp: i64) -> String {
    format!(
        r#"<div id="messages" hx-swap-oob="afterbegin">{}</div>"#,
        render_message(msg, sender_name, timestamp)
    )
}

fn render_message_deletion_oob(message_id: i32) -> String {
    format!(
        r#"<div id="message-{message_id}" hx-swap-oob="delete"></div>
<div id="search-message-{message_id}" hx-swap-oob="delete"></div>"#
    )
}

/// Po uspešno oddanem sporočilu je treba izprazniti polje za vnos samo pri
/// pošiljatelju — ne prek broadcast kanala sobe, ker bi to izbrisalo
/// nedokončano besedilo drugim uporabnikom, ki ravno takrat tipkajo.
pub fn render_message_input_reset() -> String {
    format!(
        r#"<textarea name="content" id="msg-input" rows="1" maxlength="{max_message_length}" placeholder="Sporočilo…" required hx-swap-oob="true"></textarea>
<div id="message-status" class="message-status" role="status" aria-live="polite" hx-swap-oob="true"></div>"#,
        max_message_length = MAX_MESSAGE_LENGTH,
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
