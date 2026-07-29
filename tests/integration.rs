use axum::{
    body::{to_bytes, Body},
    http::{
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
        Request, StatusCode,
    },
    Router,
};
use chat_room_prog2::{
    controller::{
        auth::{create_jwt, validate_jwt_secret, verify_jwt, SESSION_COOKIE},
        forms::{normalize_username, verify_password},
        rooms::{ensure_default_room, prepare_database_schema},
        tipi::ServerState,
        web::build_router,
    },
    entities::{
        client, message,
        prelude::{Client, RoomMember, Soba},
        room_member, soba,
    },
};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, Set,
};
use std::net::SocketAddr;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Error as WsError, Message as WsMessage},
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, server)
}

fn websocket_request(address: SocketAddr, room_name: &str, cookie: &str) -> axum::http::Request<()> {
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
    assert!(verify_jwt(
        &token,
        "a-different-secret-that-is-also-long-enough"
    )
    .is_none());
    assert!(verify_jwt("to-ni-jwt", TEST_SECRET).is_none());
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
async fn protected_http_endpoints_require_a_valid_session() {
    let (app, _db) = test_app().await;

    for request in [
        form_request("GET", "/me", "", None),
        form_request("GET", "/rooms", "", None),
        form_request("GET", "/rooms/general/panel", "", None),
        form_request("GET", "/rooms/general/messages", "", None),
        form_request("POST", "/rooms", "name=skrivna", None),
        form_request("POST", "/rooms/join", "id=123456", None),
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
    assert!(!panel.contains("username=gost"));
    assert!(panel.contains("hx-delete=\"/rooms/rust\""));

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

    assert!(chat_room_prog2::entities::prelude::Soba::find()
        .filter(chat_room_prog2::entities::soba::Column::Name.eq("rust"))
        .one(&db)
        .await
        .unwrap()
        .is_none());
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
        assert!(body_text(response).await.contains("room-action-message error"));
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
    assert!(!body_text(response).await.contains("data-room-name=\"projekt\""));

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
    assert!(body_text(response).await.contains("še nisi pridružen"));

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
        assert!(body_text(response).await.contains("room-action-message error"));
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
    assert!(body_text(response).await.contains("Pridružil si se sobi #projekt"));

    let member = Client::find()
        .filter(client::Column::Username.eq("jovan"))
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
    assert!(body_text(response).await.contains("že pridružen"));
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
    assert!(newest.find("msg-007").unwrap() < newest.find("msg-055").unwrap());

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
    assert!(older.find("msg-001").unwrap() < older.find("msg-006").unwrap());
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
            assert_eq!(response.status().as_u16(), StatusCode::UNAUTHORIZED.as_u16())
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
            assert_eq!(response.status().as_u16(), StatusCode::UNAUTHORIZED.as_u16())
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
    let (mut socket, _) =
        connect_async(websocket_request(address, "zasebna", &outsider_cookie))
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

    let stored = message::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content, "pozdrav iz websocket testa");
    assert_eq!(message::Entity::find().count(&db).await.unwrap(), 1);

    socket.close(None).await.unwrap();
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
    assert!(body_text(response).await.contains("Pridružil si se"));

    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) =
        connect_async(websocket_request(address, "skupina", &owner_cookie))
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
    // za sabo neprebranih "reset textarea" frame-ov, ki bi zmedli poznejše branje.
    owner_socket
        .send(WsMessage::Text(
            r#"{"content":"owner-ready"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(2), recv_until(&mut owner_socket, "owner-ready"))
        .await
        .expect("lastnikov WebSocket ni postal pripravljen");

    member_socket
        .send(WsMessage::Text(
            r#"{"content":"member-ready"}"#.into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(2), recv_until(&mut member_socket, "member-ready"))
        .await
        .expect("članov WebSocket ni postal pripravljen");
    timeout(Duration::from_secs(2), recv_until(&mut owner_socket, "member-ready"))
        .await
        .expect("lastnik ni prejel potrditve članove pripravljenosti");

    message::Entity::delete_many()
        .filter(message::Column::SobaId.eq(room.id))
        .exec(&db)
        .await
        .unwrap();

    member_socket
        .send(WsMessage::Text(
            r#"{"content":"sporočilo za oba"}"#.into(),
        ))
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
