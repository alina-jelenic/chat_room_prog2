use crate::common::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    websocket_request,
};
use axum::http::StatusCode;
use chat_room_prog2::{
    controller::{
        rooms::{ensure_default_room, prepare_database_schema},
        tipi::ServerState,
        web::build_router,
    },
    entities::{
        client, message,
        prelude::{Client, Soba},
        soba,
    },
};
use futures_util::SinkExt;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, EntityTrait, QueryFilter, Set,
};

use tokio::time::{Duration, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;
const TEST_SECRET: &str = "test-secret-that-is-longer-than-32-characters";

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
