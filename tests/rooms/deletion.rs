use crate::integration::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    wait_for_socket_close, websocket_request,
};
use axum::http::StatusCode;
use chat_room_prog2::entities::{message, prelude::Soba, soba};
use futures_util::SinkExt;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use tokio::time::{Duration, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

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
