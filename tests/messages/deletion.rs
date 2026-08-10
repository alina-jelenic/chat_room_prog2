use crate::common::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    websocket_request,
};
use axum::http::StatusCode;
use chat_room_prog2::entities::{
    client, message, message_reactions,
    prelude::{Client, MessageReactions, Soba},
    soba,
};
use chat_room_prog2::migration::*;
use futures_util::SinkExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

#[tokio::test]
async fn only_sender_can_delete_a_message_and_connected_users_are_notified() {
    let (app, db) = test_app().await;
    Migrator::up(&db, None).await.unwrap();

    let sender_cookie = register_and_login(&app, "alina").await;
    let other_cookie = register_and_login(&app, "jovan").await;
    let sender = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let other = Client::find()
        .filter(client::Column::Username.eq("jovan"))
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
    let stored_message = message::ActiveModel {
        sender_id: Set(Some(sender.id as i64)),
        content: Set("sporočilo za izbris".to_string()),
        timestamp: Set(1),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    message_reactions::ActiveModel {
        message_id: Set(stored_message.id),
        client_id: Set(other.id),
        emoji: Set("👍".to_string()),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            &format!("/messages/{}", stored_message.id),
            "",
            Some(&other_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        message::Entity::find_by_id(stored_message.id)
            .one(&db)
            .await
            .unwrap()
            .is_some()
    );

    let http_app = app.clone();
    let (address, server) = start_server(app).await;
    let (mut other_socket, _) = connect_async(websocket_request(address, "general", &other_cookie))
        .await
        .unwrap();
    other_socket
        .send(WsMessage::Text(r#"{"content":"ready"}"#.into()))
        .await
        .unwrap();
    timeout(
        Duration::from_secs(2),
        recv_until(&mut other_socket, "ready"),
    )
    .await
    .expect("WebSocket ni postal pripravljen");

    let response = http_app
        .oneshot(form_request(
            "DELETE",
            &format!("/messages/{}", stored_message.id),
            "",
            Some(&sender_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deletion_response = body_text(response).await;
    assert!(deletion_response.contains(&format!("id=\"msg-{}\"", stored_message.id)));
    assert!(deletion_response.contains("hx-swap-oob=\"delete\""));

    let notification = timeout(
        Duration::from_secs(2),
        recv_until(&mut other_socket, &format!("msg-{}", stored_message.id)),
    )
    .await
    .expect("drugi uporabnik ni prejel izbrisa");
    assert!(notification.contains("hx-swap-oob=\"delete\""));
    assert!(
        message::Entity::find_by_id(stored_message.id)
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 0);

    other_socket.close(None).await.unwrap();
    server.abort();
}
