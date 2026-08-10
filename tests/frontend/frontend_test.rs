use crate::common::{body_text, form_request, register_and_login, test_app};

use axum::http::StatusCode;
use chat_room_prog2::{
    controller::forms::verify_password,
    entities::{client, message, prelude::Client},
};

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

use tower::ServiceExt;

#[test]
fn frontend_reports_websocket_connection_state() {
    let html = include_str!("../../static/index.html");
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
        sender_id: Set(Some(user.id)),
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

#[test]
fn frontend_loads_htmx_from_local_files() {
    let index = include_str!("../../static/index.html");
    let authorisation = include_str!("../../static/authorisation.html");

    assert!(index.contains(r#"src="/vendor/htmx.min.js""#));
    assert!(index.contains(r#"src="/vendor/ws.js""#));
    assert!(authorisation.contains(r#"src="/vendor/htmx.min.js""#));

    assert!(!index.contains("unpkg.com/htmx"));
    assert!(!authorisation.contains("unpkg.com/htmx"));

    assert!(!include_bytes!("../../static/vendor/htmx.min.js").is_empty());
    assert!(!include_bytes!("../../static/vendor/ws.js").is_empty());
}
