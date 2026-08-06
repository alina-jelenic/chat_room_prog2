use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use chat_room_prog2::{
    controller::{
        auth::{Claims, SESSION_COOKIE, create_jwt, validate_jwt_secret, verify_jwt},
        forms::{admin_reset_legacy_password, normalize_username, verify_password},
        rooms::{ensure_default_room, prepare_database_schema},
        tipi::{MESSAGE_COOLDOWN, ServerState},
        web::build_router,
    },
    entities::{
        client, message,
        prelude::{Client, MessageReactions, RoomMember, Soba},
        room_member, soba,
    },
};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{EncodingKey, Header, encode};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use std::net::SocketAddr;
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WsError, Message as WsMessage, client::IntoClientRequest},
};
use tower::ServiceExt;

const TEST_SECRET: &str = "test-secret-that-is-longer-than-32-characters";

async fn test_app() -> (Router, DatabaseConnection) {
    // Pri SQLite in-memory bazi mora isti test uporabljati eno samo povezavo,
    // sicer bi vsaka povezava dobila svojo prazno bazo.
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();
    prepare_database_schema(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();

    let state = ServerState::new(db.clone(), TEST_SECRET.to_string());
    (build_router(state), db)
}

fn form_request(method: &str, uri: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(COOKIE, cookie);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

async fn register_and_login(app: &Router, username: &str) -> String {
    let register_body = format!("username={username}&password=skrivnost1&confirm=skrivnost1");
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/register", &register_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let login_body = format!("username={username}&password=skrivnost1");
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/login", &login_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["HX-Redirect"], "/index.html");

    response.headers()[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn start_server(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, server)
}

fn websocket_request(
    address: SocketAddr,
    room_name: &str,
    cookie: &str,
) -> axum::http::Request<()> {
    let mut request = format!("ws://{address}/ws?room_name={room_name}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(COOKIE, cookie.parse().unwrap());
    request
}

fn assert_login_redirect(response: &axum::response::Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("HX-Redirect").unwrap(),
        "/authorisation.html"
    );
}

#[test]
fn username_rules_are_deterministic() {
    assert_eq!(normalize_username("  Alina_2  ").unwrap(), "alina_2");
    assert!(normalize_username("ab").is_err());
    assert!(normalize_username("2alina").is_err());
    assert!(normalize_username("ime s presledkom").is_err());
    assert!(normalize_username("<script>").is_err());
    assert!(normalize_username(&"a".repeat(25)).is_err());
}

#[test]
fn jwt_secret_and_signature_are_validated() {
    assert!(validate_jwt_secret("prekratko").is_err());
    assert!(create_jwt(1, "alina", "prekratko").is_err());

    let token = create_jwt(1, "alina", TEST_SECRET).unwrap();
    let claims = verify_jwt(&token, TEST_SECRET).unwrap();
    assert_eq!(claims.sub, 1);
    assert_eq!(claims.username, "alina");
    assert!(verify_jwt(&token, "a-different-secret-that-is-also-long-enough").is_none());
    assert!(verify_jwt("to-ni-jwt", TEST_SECRET).is_none());
}

#[test]
fn expired_jwt_is_rejected() {
    let expired_claims = Claims {
        sub: 1,
        username: "alina".to_string(),
        exp: 1,
    };
    let token = encode(
        &Header::default(),
        &expired_claims,
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .unwrap();

    assert!(verify_jwt(&token, TEST_SECRET).is_none());
}

#[test]
fn frontend_reports_websocket_connection_state() {
    let html = include_str!("../static/index.html");
    for event_name in [
        "htmx:wsConnecting",
        "htmx:wsOpen",
        "htmx:wsClose",
        "htmx:wsError",
    ] {
        assert!(html.contains(event_name));
    }
    assert!(html.contains("data-connection-status"));
    assert!(html.contains("Ponovno povezujem"));
}

#[tokio::test]
async fn migrations_and_default_room_are_idempotent() {
    let (_app, db) = test_app().await;

    prepare_database_schema(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();

    assert_eq!(
        Soba::find()
            .filter(soba::Column::Name.eq("general"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn migrations_preserve_data_from_a_populated_legacy_database() {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();

    // Prve tri migracije predstavljajo staro različico baze: uporabnik še nima
    // gesla, sporočilo pa še ni povezano s sobo.
    Migrator::up(&db, Some(3)).await.unwrap();
    db.execute_unprepared("INSERT INTO client (id, username) VALUES (1, 'Stari_Uporabnik')")
        .await
        .unwrap();
    db.execute_unprepared("INSERT INTO soba (id, name) VALUES (123456, 'stara_soba')")
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO message (id, sender_id, content, timestamp) \
         VALUES (1, 1, 'staro sporocilo', 1700000000)",
    )
    .await
    .unwrap();

    prepare_database_schema(&db).await.unwrap();

    let legacy_user = Client::find_by_id(1).one(&db).await.unwrap().unwrap();
    assert_eq!(legacy_user.username, "Stari_Uporabnik");
    assert_eq!(legacy_user.geslo, "");
    assert!(!verify_password("karkoli", &legacy_user.geslo).unwrap());

    assert!(
        admin_reset_legacy_password(&db, "Stari_Uporabnik", None, "12345")
            .await
            .is_err()
    );
    let reset_user = admin_reset_legacy_password(&db, "Stari_Uporabnik", None, "nova_skrivnost")
        .await
        .unwrap();
    assert_eq!(reset_user.username, "stari_uporabnik");
    assert!(verify_password("nova_skrivnost", &reset_user.geslo).unwrap());
    assert!(
        admin_reset_legacy_password(&db, "stari_uporabnik", None, "druga_skrivnost")
            .await
            .is_err()
    );

    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(Soba::find_by_id(123456).one(&db).await.unwrap().is_some());

    let legacy_message = message::Entity::find_by_id(1)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy_message.content, "staro sporocilo");
    assert_eq!(legacy_message.sender_id, Some(1));
    assert_eq!(legacy_message.soba_id, general.id);
}

#[tokio::test]
async fn messages_survive_reopening_a_file_database() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database_path = std::env::temp_dir().join(format!(
        "chat-room-prog2-{}-{unique}.db",
        std::process::id()
    ));
    let database_url = format!("sqlite://{}?mode=rwc", database_path.to_string_lossy());

    let mut options = ConnectOptions::new(&database_url);
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();
    prepare_database_schema(&db).await.unwrap();
    ensure_default_room(&db).await.unwrap();

    let state = ServerState::new(db.clone(), TEST_SECRET.to_string());
    let app = build_router(state);
    let cookie = register_and_login(&app, "alina").await;
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "general", &cookie))
        .await
        .unwrap();
    socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo po ponovnem zagonu"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "sporočilo po ponovnem zagonu"),
    )
    .await
    .expect("sporočilo pred ponovnim odpiranjem baze ni bilo prejeto");
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
    drop(db);

    let mut reopened_options = ConnectOptions::new(&database_url);
    reopened_options.max_connections(1).sqlx_logging(false);
    let reopened = Database::connect(reopened_options).await.unwrap();
    prepare_database_schema(&reopened).await.unwrap();

    let stored = message::Entity::find()
        .filter(message::Column::SobaId.eq(general.id))
        .one(&reopened)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content, "sporočilo po ponovnem zagonu");
    drop(reopened);

    let _ = std::fs::remove_file(&database_path);
    let _ = std::fs::remove_file(database_path.with_extension("db-shm"));
    let _ = std::fs::remove_file(database_path.with_extension("db-wal"));
}

#[tokio::test]
async fn protected_http_endpoints_require_a_valid_session() {
    let (app, _db) = test_app().await;

    for request in [
        form_request("GET", "/me", "", None),
        form_request("GET", "/rooms", "", None),
        form_request("GET", "/rooms/general/panel", "", None),
        form_request("GET", "/rooms/general/messages", "", None),
        form_request("POST", "/rooms", "name=skrivna", None),
        form_request("POST", "/rooms/join", "id=123456", None),
        form_request("DELETE", "/rooms/skrivna/membership", "", None),
        form_request("DELETE", "/rooms/skrivna", "", None),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_login_redirect(&response);
    }

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms",
            "",
            Some("chat_session=ponarejen-token"),
        ))
        .await
        .unwrap();
    assert_login_redirect(&response);
}

#[tokio::test]
async fn login_me_and_logout_manage_the_session_cookie() {
    let (app, _db) = test_app().await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=neobstaja&password=napacno",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Napačno ime ali geslo"));

    let register_body = "username=Alina&password=skrivnost1&confirm=skrivnost1";
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/register", register_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=alina&password=napacno",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Napačno ime ali geslo"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=ALINA&password=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.headers()["HX-Redirect"], "/index.html");
    let set_cookie = response.headers()[SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("chat_session="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Path=/"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(form_request("GET", "/me", "", Some(&cookie)))
        .await
        .unwrap();
    let me = body_text(response).await;
    assert!(me.contains("id=\"user-display\">alina</span>"));
    assert!(me.contains("id=\"user-avatar\">a</div>"));

    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/logout", "", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.headers()["HX-Redirect"], "/authorisation.html");
    let removal = response.headers()[SET_COOKIE].to_str().unwrap();
    assert!(removal.contains("chat_session="));
    assert!(removal.contains("Max-Age=0"));
    assert!(removal.contains("Path=/"));
}

#[tokio::test]
async fn registration_login_room_panel_and_deletion_work_together() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "Alina").await;

    let user = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(user.geslo, "skrivnost1");
    assert!(verify_password("skrivnost1", &user.geslo).unwrap());

    let response = app
        .clone()
        .oneshot(form_request("POST", "/rooms", "name=rust", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("# rust"));

    let response = app
        .clone()
        .oneshot(form_request("GET", "/rooms/rust/panel", "", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let panel = body_text(response).await;
    assert!(panel.contains("data-current-username=\"alina\""));
    assert!(panel.contains("ws-connect=\"/ws?room_name=rust\""));
    assert!(panel.contains("data-connection-status"));
    assert!(panel.contains("class=\"send-btn\" aria-label=\"Pošlji\" disabled"));
    assert!(!panel.contains("username=gost"));
    assert!(panel.contains("hx-delete=\"/rooms/rust\""));
    assert!(!panel.contains("hx-delete=\"/rooms/rust/membership\""));
    assert!(!panel.contains("copy-id-btn"));
    assert!(!panel.contains("onclick="));

    let room = chat_room_prog2::entities::prelude::Soba::find()
        .filter(chat_room_prog2::entities::soba::Column::Name.eq("rust"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    message::ActiveModel {
        sender_id: Set(Some(user.id as i64)),
        content: Set("za brisanje".to_string()),
        timestamp: Set(1),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(form_request("DELETE", "/rooms/rust", "", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let panel = body_text(response).await;
    assert!(panel.contains("room_name=general"));
    assert!(!panel.contains("hx-delete=\"/rooms/general\""));

    let response = app
        .clone()
        .oneshot(form_request("DELETE", "/rooms/general", "", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    assert!(
        chat_room_prog2::entities::prelude::Soba::find()
            .filter(chat_room_prog2::entities::soba::Column::Name.eq("rust"))
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 0);
}

#[tokio::test]
async fn invalid_and_case_insensitive_duplicate_usernames_are_rejected() {
    let (app, db) = test_app().await;

    for body in [
        "username=alina&password=kratko&confirm=drugo",
        "username=alina&password=12345&confirm=12345",
    ] {
        let response = app
            .clone()
            .oneshot(form_request("POST", "/api/register", body, None))
            .await
            .unwrap();
        assert!(body_text(response).await.contains("server-msg error"));
    }
    assert_eq!(Client::find().count(&db).await.unwrap(), 0);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/register",
            "username=%3Cscript%3E&password=skrivnost1&confirm=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Uporabniško ime"));
    assert_eq!(Client::find().count(&db).await.unwrap(), 0);

    register_and_login(&app, "Alina").await;
    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/register",
            "username=ALINA&password=skrivnost1&confirm=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že zasedeno"));
    assert_eq!(Client::find().count(&db).await.unwrap(), 1);

    // Unikatnost ni samo preverba v handlerju, temveč tudi omejitev baze.
    let duplicate = client::ActiveModel {
        username: Set("alina".to_string()),
        geslo: Set("ni-pomembno".to_string()),
        ..Default::default()
    }
    .insert(&db)
    .await;
    assert!(duplicate.is_err());
}

#[tokio::test]
async fn invalid_and_duplicate_room_names_do_not_create_rooms() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;

    for body in [
        "name=",
        "name=ime+s+presledkom",
        "name=%3Cscript%3E",
        "name=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let response = app
            .clone()
            .oneshot(form_request("POST", "/rooms", body, Some(&cookie)))
            .await
            .unwrap();
        assert!(
            body_text(response)
                .await
                .contains("room-action-message error")
        );
    }
    assert_eq!(Soba::find().count(&db).await.unwrap(), 1);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=general",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že obstaja"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=Rust_Chat",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("#rust_chat"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=RUST_CHAT",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že obstaja"));
    assert_eq!(Soba::find().count(&db).await.unwrap(), 2);
}

#[tokio::test]
async fn room_membership_controls_listing_history_and_deletion() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=projekt",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("projekt"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request("GET", "/rooms", "", Some(&member_cookie)))
        .await
        .unwrap();
    assert!(
        !body_text(response)
            .await
            .contains("data-room-name=\"projekt\"")
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("še nimaš dostopa"));

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/messages",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    for invalid_id in ["", "abc", "-1"] {
        let response = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/rooms/join",
                &format!("id={invalid_id}"),
                Some(&member_cookie),
            ))
            .await
            .unwrap();
        assert!(
            body_text(response)
                .await
                .contains("room-action-message error")
        );
    }

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(
        body_text(response)
            .await
            .contains("Zdaj si v sobi #projekt")
    );

    let member = Client::find()
        .filter(client::Column::Username.eq("jovan"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Že si v sobi"));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    let panel = body_text(response).await;
    assert!(panel.contains("data-current-username=\"jovan\""));
    assert!(!panel.contains("hx-delete=\"/rooms/projekt\""));
    assert!(panel.contains("hx-delete=\"/rooms/projekt/membership\""));

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt/membership",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("kot njen lastnik"));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(owner.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("lahko izbriše samo"));
    assert!(Soba::find_by_id(room.id).one(&db).await.unwrap().is_some());

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt/membership",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let panel = body_text(response).await;
    assert!(panel.contains("room_name=general"));
    assert!(panel.contains("Nisi več v sobi #projekt"));
    assert!(!panel.contains("data-room-name=\"projekt\""));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/messages",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("še nimaš dostopa"));

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(Soba::find_by_id(room.id).one(&db).await.unwrap().is_none());
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn message_history_is_paginated_ordered_and_html_escaped() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;
    let user = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let room = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    for number in 1..=55 {
        message::ActiveModel {
            sender_id: Set(Some(user.id as i64)),
            content: Set(format!("msg-{number:03}")),
            timestamp: Set(number as i64),
            soba_id: Set(room.id),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    message::ActiveModel {
        sender_id: Set(Some(user.id as i64)),
        content: Set("<script>alert('x')</script>&".to_string()),
        timestamp: Set(56),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/general/messages",
            "",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let newest = body_text(response).await;
    assert!(!newest.contains("msg-006"));
    assert!(newest.contains("msg-007"));
    assert!(newest.contains("msg-055"));
    assert!(newest.contains("before_id=7"));
    assert!(newest.contains("&lt;script&gt;alert(&#x27;x&#x27;)&lt;/script&gt;&amp;"));
    assert!(!newest.contains("<script>alert"));
    assert!(newest.find("msg-055").unwrap() < newest.find("msg-007").unwrap());

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/general/messages?before_id=7",
            "",
            Some(&cookie),
        ))
        .await
        .unwrap();
    let older = body_text(response).await;
    assert!(older.contains("msg-001"));
    assert!(older.contains("msg-006"));
    assert!(!older.contains("msg-007"));
    assert!(!older.contains("Naloži starejša sporočila"));
    assert!(older.find("msg-006").unwrap() < older.find("msg-001").unwrap());
}

#[tokio::test]
async fn websocket_rejects_missing_and_invalid_sessions() {
    let (app, _db) = test_app().await;
    let (address, server) = start_server(app).await;

    let error = match connect_async(format!("ws://{address}/ws?room_name=general")).await {
        Ok(_) => panic!("WebSocket brez seje ne bi smel biti sprejet"),
        Err(error) => error,
    };
    match error {
        WsError::Http(response) => {
            assert_eq!(
                response.status().as_u16(),
                StatusCode::UNAUTHORIZED.as_u16()
            )
        }
        other => panic!("pričakovan je bil HTTP 401, dobljeno: {other}"),
    }

    let request = websocket_request(address, "general", "chat_session=neveljaven-podpis");
    let error = match connect_async(request).await {
        Ok(_) => panic!("WebSocket z neveljavno sejo ne bi smel biti sprejet"),
        Err(error) => error,
    };
    match error {
        WsError::Http(response) => {
            assert_eq!(
                response.status().as_u16(),
                StatusCode::UNAUTHORIZED.as_u16()
            )
        }
        other => panic!("pričakovan je bil HTTP 401, dobljeno: {other}"),
    }

    server.abort();
}

#[tokio::test]
async fn websocket_closes_when_user_is_not_a_room_member() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let outsider_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=zasebna",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "zasebna", &outsider_cookie))
        .await
        .unwrap();

    let outcome = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("strežnik ni pravočasno zaprl nedovoljene povezave");
    match outcome {
        None | Some(Ok(WsMessage::Close(_))) | Some(Err(_)) => {}
        Some(Ok(other)) => panic!("nedovoljena povezava je prejela sporočilo: {other:?}"),
    }
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 0);

    server.abort();
}

#[tokio::test]
async fn websocket_closes_after_member_leaves_the_room() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=odhod",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("odhod"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Zdaj si v sobi"));

    let http_app = app.clone();
    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "odhod", &member_cookie))
        .await
        .unwrap();
    socket
        .send(WsMessage::Text(
            r#"{"content":"povezava je pripravljena"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "povezava je pripravljena"),
    )
    .await
    .expect("članov WebSocket ni postal pripravljen");

    message::Entity::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&db)
        .await
        .unwrap();

    let response = http_app
        .oneshot(form_request(
            "DELETE",
            "/rooms/odhod/membership",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Nisi več v sobi #odhod"));

    let _ = socket
        .send(WsMessage::Text(
            r#"{"content":"tega sporočila ne sme shraniti"}"#.into(),
        ))
        .await;
    wait_for_socket_close(&mut socket).await;

    assert_eq!(
        message::Entity::find()
            .filter(message::Column::SobaId.eq(room.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );

    server.abort();
}

#[tokio::test]
async fn passive_websocket_closes_after_member_leaves_while_owner_keeps_chatting() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=pasivni-odhod",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("pasivni-odhod"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Zdaj si v sobi"));

    let http_app = app.clone();
    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) =
        connect_async(websocket_request(address, "pasivni-odhod", &owner_cookie))
            .await
            .unwrap();
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "pasivni-odhod", &member_cookie))
            .await
            .unwrap();

    owner_socket
        .send(WsMessage::Text(r#"{"content":"owner-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "owner-ready"),
    )
    .await
    .expect("lastnikov WebSocket ni postal pripravljen");

    member_socket
        .send(WsMessage::Text(r#"{"content":"member-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "member-ready"),
    )
    .await
    .expect("članov WebSocket ni postal pripravljen");
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "member-ready"),
    )
    .await
    .expect("lastnik ni prejel članovega pripravljalnega sporočila");

    message::Entity::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&db)
        .await
        .unwrap();

    let response = http_app
        .oneshot(form_request(
            "DELETE",
            "/rooms/pasivni-odhod/membership",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Nisi več v sobi"));

    // Član po odhodu namenoma ne pošlje ničesar. Strežnik mora njegovo
    // pasivno povezavo vseeno sam zapreti.
    wait_for_socket_close(&mut member_socket).await;

    sleep(MESSAGE_COOLDOWN + Duration::from_millis(50)).await;
    owner_socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo po pasivnem odhodu"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "sporočilo po pasivnem odhodu"),
    )
    .await
    .expect("lastnik po članovem odhodu ni mogel nadaljevati pogovora");

    let stored = message::Entity::find()
        .filter(message::Column::SobaId.eq(room.id))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content, "sporočilo po pasivnem odhodu");
    assert_eq!(
        message::Entity::find()
            .filter(message::Column::SobaId.eq(room.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    owner_socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn websocket_message_is_authenticated_persisted_and_broadcast() {
    let (app, db) = test_app().await;
    let password_hash = chat_room_prog2::controller::forms::hash_password("skrivnost1").unwrap();
    let user = client::ActiveModel {
        username: Set("jovan".to_string()),
        geslo: Set(password_hash),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let token = create_jwt(user.id, &user.username, TEST_SECRET).unwrap();

    let (address, server) = start_server(app).await;

    let request = websocket_request(address, "general", &format!("{SESSION_COOKIE}={token}"));
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(WsMessage::Text(r#"{"content":"   "}"#.into()))
        .await
        .unwrap();
    socket
        .send(WsMessage::Text(
            serde_json::json!({"content": "x".repeat(2001)})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let length_error = timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "največ 2000 znakov"),
    )
    .await
    .expect("uporabnik ni prejel opozorila o predolgem sporočilu");
    assert!(length_error.contains("message-status"));
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 0);

    socket
        .send(WsMessage::Text(
            r#"{"content":"pozdrav iz websocket testa"}"#.into(),
        ))
        .await
        .unwrap();

    let received = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("strežnik ni pravočasno oddal sporočila")
        .expect("WebSocket se je nepričakovano zaprl")
        .unwrap()
        .into_text()
        .unwrap();
    assert!(received.contains("pozdrav iz websocket testa"));
    assert!(received.contains("jovan"));

    let reset = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("strežnik ni pravočasno počistil vnosnega polja")
        .expect("WebSocket se je nepričakovano zaprl")
        .unwrap()
        .into_text()
        .unwrap();
    assert!(reset.contains("id=\"msg-input\""));
    assert!(reset.contains("hx-swap-oob=\"true\""));
    assert!(!reset.contains("pozdrav iz websocket testa"));

    let stored = message::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(stored.content, "pozdrav iz websocket testa");
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 1);

    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn websocket_rate_limit_is_shared_between_connections() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let (address, server) = start_server(app).await;
    let (mut first_socket, _) = connect_async(websocket_request(address, "general", &cookie))
        .await
        .unwrap();
    let (mut second_socket, _) = connect_async(websocket_request(address, "general", &cookie))
        .await
        .unwrap();

    first_socket
        .send(WsMessage::Text(r#"{"content":"prvo sporočilo"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut first_socket, "prvo sporočilo"),
    )
    .await
    .expect("prvo sporočilo ni bilo sprejeto");

    // Isti uporabnik poskusi omejitev obiti prek druge povezave.
    second_socket
        .send(WsMessage::Text(
            r#"{"content":"prehitro sporočilo"}"#.into(),
        ))
        .await
        .unwrap();
    let warning = timeout(
        Duration::from_secs(2),
        recv_until(&mut second_socket, "pošiljaš prehitro"),
    )
    .await
    .expect("uporabnik ni prejel opozorila o omejitvi");
    assert!(warning.contains("message-status"));
    assert_eq!(
        message::Entity::find()
            .filter(message::Column::SobaId.eq(general.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    sleep(MESSAGE_COOLDOWN + Duration::from_millis(50)).await;
    second_socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo po premoru"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut second_socket, "sporočilo po premoru"),
    )
    .await
    .expect("sporočilo po cooldownu ni bilo sprejeto");
    assert_eq!(
        message::Entity::find()
            .filter(message::Column::SobaId.eq(general.id))
            .count(&db)
            .await
            .unwrap(),
        2
    );

    first_socket.close(None).await.unwrap();
    second_socket.close(None).await.unwrap();
    server.abort();
}

async fn recv_until<S>(socket: &mut S, needle: &str) -> String
where
    S: futures_util::Stream<Item = Result<WsMessage, WsError>> + Unpin,
{
    loop {
        let msg = socket
            .next()
            .await
            .expect("povezava se je nepričakovano zaprla")
            .unwrap();
        let text = msg.into_text().unwrap().to_string();
        if text.contains(needle) {
            return text;
        }
    }
}

async fn wait_for_socket_close<S>(socket: &mut S)
where
    S: futures_util::Stream<Item = Result<WsMessage, WsError>> + Unpin,
{
    timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                None | Some(Ok(WsMessage::Close(_))) | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .expect("strežnik ni pravočasno zaprl WebSocket povezave");
}

#[tokio::test]
async fn websocket_broadcast_reaches_two_joined_users() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=skupina",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("skupina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Zdaj si v sobi"));

    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) = connect_async(websocket_request(address, "skupina", &owner_cookie))
        .await
        .unwrap();
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "skupina", &member_cookie))
            .await
            .unwrap();

    // Handshake se zaključi tik preden se strežniška naloga naroči na broadcast.
    // Z dvema kratkima sporočiloma zato najprej deterministično preverimo, da
    // sta oba odjemalca zares pripravljena, in se izognemo časovno občutljivemu testu.
    // `recv_until` bere naprej, dokler ne najde iskanega niza, in tako ne pusti
    // neprebranih okvirjev za ponastavitev vnosnega polja, ki bi zmedli poznejše branje.
    owner_socket
        .send(WsMessage::Text(r#"{"content":"owner-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "owner-ready"),
    )
    .await
    .expect("lastnikov WebSocket ni postal pripravljen");

    member_socket
        .send(WsMessage::Text(r#"{"content":"member-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "member-ready"),
    )
    .await
    .expect("članov WebSocket ni postal pripravljen");
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "member-ready"),
    )
    .await
    .expect("lastnik ni prejel potrditve članove pripravljenosti");

    message::Entity::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&db)
        .await
        .unwrap();

    sleep(MESSAGE_COOLDOWN + Duration::from_millis(50)).await;
    member_socket
        .send(WsMessage::Text(r#"{"content":"sporočilo za oba"}"#.into()))
        .await
        .unwrap();

    let owner_received = timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "sporočilo za oba"),
    )
    .await
    .expect("lastnik ni pravočasno prejel sporočila");
    let member_received = timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "sporočilo za oba"),
    )
    .await
    .expect("član ni pravočasno prejel sporočila");

    for received in [owner_received, member_received] {
        assert!(received.contains("sporočilo za oba"));
        assert!(received.contains("jovan"));
    }
    assert_eq!(
        message::Entity::find()
            .filter(message::Column::SobaId.eq(room.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    owner_socket.close(None).await.unwrap();
    member_socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn deleting_a_room_notifies_connected_users_and_blocks_further_messages() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=brisanje",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("brisanje"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Zdaj si v sobi"));

    let http_app = app.clone();
    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) =
        connect_async(websocket_request(address, "brisanje", &owner_cookie))
            .await
            .unwrap();
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "brisanje", &member_cookie))
            .await
            .unwrap();

    owner_socket
        .send(WsMessage::Text(r#"{"content":"owner-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "owner-ready"),
    )
    .await
    .expect("lastnikov WebSocket ni postal pripravljen");

    member_socket
        .send(WsMessage::Text(r#"{"content":"member-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "member-ready"),
    )
    .await
    .expect("članov WebSocket ni postal pripravljen");
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "member-ready"),
    )
    .await
    .expect("lastnik ni prejel članovega pripravljalnega sporočila");

    message::Entity::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&db)
        .await
        .unwrap();

    let response = http_app
        .oneshot(form_request(
            "DELETE",
            "/rooms/brisanje",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let owner_notification = timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "je bila izbrisana"),
    )
    .await
    .expect("lastnik ni prejel obvestila o izbrisu");
    let member_notification = timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "je bila izbrisana"),
    )
    .await
    .expect("član ni prejel obvestila o izbrisu");

    for notification in [owner_notification, member_notification] {
        assert!(notification.contains("hx-get=\"/rooms/general/panel\""));
        assert!(notification.contains("id=\"room-list\""));
    }

    let _ = owner_socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo po izbrisu lastnika"}"#.into(),
        ))
        .await;
    let _ = member_socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo po izbrisu člana"}"#.into(),
        ))
        .await;
    wait_for_socket_close(&mut owner_socket).await;
    wait_for_socket_close(&mut member_socket).await;

    assert!(Soba::find_by_id(room.id).one(&db).await.unwrap().is_none());
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 0);

    server.abort();
}

#[tokio::test]
async fn concurrent_room_creation_with_same_name_only_creates_one_room() {
    let (app, db) = test_app().await;
    let cookie_a = register_and_login(&app, "prva").await;
    let cookie_b = register_and_login(&app, "druga").await;

    let (response_a, response_b) = tokio::join!(
        app.clone().oneshot(form_request(
            "POST",
            "/rooms",
            "name=dirka",
            Some(&cookie_a)
        )),
        app.clone().oneshot(form_request(
            "POST",
            "/rooms",
            "name=dirka",
            Some(&cookie_b)
        )),
    );
    let response_a = response_a.unwrap();
    let response_b = response_b.unwrap();

    // Oba zahtevka morata dobiti smiseln odgovor, ne surovega strežniškega 500.
    assert_eq!(response_a.status(), StatusCode::OK);
    assert_eq!(response_b.status(), StatusCode::OK);

    let text_a = body_text(response_a).await;
    let text_b = body_text(response_b).await;

    let successes = [&text_a, &text_b]
        .into_iter()
        .filter(|t| t.contains("je ustvarjena"))
        .count();
    assert_eq!(successes, 1, "natanko en zahtevek bi moral uspeti");

    assert_eq!(
        Soba::find()
            .filter(soba::Column::Name.eq("dirka"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn reaction_toggle_updates_counts_and_broadcasts_to_room() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=reakcije",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("reakcije"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Zdaj si v sobi"));

    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let target_message = message::ActiveModel {
        sender_id: Set(Some(owner.id as i64)),
        content: Set("sporočilo za reakcijo".to_string()),
        timestamp: Set(1),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let message_id = target_message.id;

    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) =
        connect_async(websocket_request(address, "reakcije", &owner_cookie))
            .await
            .unwrap();
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "reakcije", &member_cookie))
            .await
            .unwrap();

    owner_socket
        .send(WsMessage::Text(r#"{"content":"owner-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "owner-ready"),
    )
    .await
    .expect("lastnikov WebSocket ni postal pripravljen");

    member_socket
        .send(WsMessage::Text(r#"{"content":"member-ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "member-ready"),
    )
    .await
    .expect("članov WebSocket ni postal pripravljen");
    timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, "member-ready"),
    )
    .await
    .expect("lastnik ni prejel članovega pripravljalnega sporočila");

    // Član doda reakcijo 👍.
    member_socket
        .send(WsMessage::Text(
            format!(r#"{{"reaction_message_id":"{message_id}","reaction_emoji":"👍"}}"#).into(),
        ))
        .await
        .unwrap();

    let owner_saw_reaction = timeout(Duration::from_secs(2), recv_until(&mut owner_socket, "👍"))
        .await
        .expect("lastnik ni prejel obvestila o reakciji");
    assert!(owner_saw_reaction.contains(&format!("id=\"reactions-{message_id}\"")));
    assert!(owner_saw_reaction.contains("👍 1"));

    let member_saw_reaction = timeout(Duration::from_secs(2), recv_until(&mut member_socket, "👍"))
        .await
        .expect("član ni prejel potrditve svoje reakcije");
    assert!(member_saw_reaction.contains("👍 1"));

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 1);

    // Ponoven klik na isto reakcijo jo mora odstraniti (toggle).
    member_socket
        .send(WsMessage::Text(
            format!(r#"{{"reaction_message_id":"{message_id}","reaction_emoji":"👍"}}"#).into(),
        ))
        .await
        .unwrap();

    let owner_saw_removal = timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, &format!("id=\"reactions-{message_id}\"")),
    )
    .await
    .expect("lastnik ni prejel obvestila o odstranitvi reakcije");
    assert!(!owner_saw_removal.contains("👍"));

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 0);

    owner_socket.close(None).await.unwrap();
    member_socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn reaction_is_ignored_for_a_message_outside_the_current_room() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=prva",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));

    // Sporočilo namenoma vstavimo v sobo #general, ne v #prva — nato bova
    // preverila, da povezava, odprta na #prva, nanj ne more reagirati.
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let foreign_message = message::ActiveModel {
        sender_id: Set(None),
        content: Set("sporočilo iz druge sobe".to_string()),
        timestamp: Set(1),
        soba_id: Set(general.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "prva", &owner_cookie))
        .await
        .unwrap();

    socket
        .send(WsMessage::Text(
            format!(
                r#"{{"reaction_message_id":"{}","reaction_emoji":"👍"}}"#,
                foreign_message.id
            )
            .into(),
        ))
        .await
        .unwrap();

    // Ker sporočilo ne pripada sobi #prva, se reakcija tiho zavrže —
    // strežnik ne sme ničesar poslati nazaj.
    let outcome = timeout(Duration::from_millis(300), socket.next()).await;
    assert!(
        outcome.is_err(),
        "strežnik ne bi smel poslati ničesar za sporočilo iz druge sobe"
    );

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 0);

    server.abort();
}
