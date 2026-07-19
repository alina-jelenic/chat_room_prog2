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
        auth::{create_jwt, SESSION_COOKIE},
        forms::{normalize_username, verify_password},
        rooms::{ensure_default_room, prepare_database_schema},
        tipi::ServerState,
        web::build_router,
    },
    entities::{client, message, prelude::Client},
};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, Set,
};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as WsMessage},
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
    let register_body = format!(
        "username={username}&password=skrivnost1&confirm=skrivnost1"
    );
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

#[test]
fn username_rules_are_deterministic() {
    assert_eq!(normalize_username("  Alina_2  ").unwrap(), "alina_2");
    assert!(normalize_username("ab").is_err());
    assert!(normalize_username("2alina").is_err());
    assert!(normalize_username("<script>").is_err());
    assert!(normalize_username(&"a".repeat(25)).is_err());
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut request = format!("ws://{address}/ws?room_name=general")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        COOKIE,
        format!("{SESSION_COOKIE}={token}").parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
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

    socket.close(None).await.unwrap();
    server.abort();
}
