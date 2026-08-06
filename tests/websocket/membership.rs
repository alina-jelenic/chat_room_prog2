use crate::integration::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    wait_for_socket_close, websocket_request,
};
use chat_room_prog2::{
    controller::tipi::MESSAGE_COOLDOWN,
    entities::{message, prelude::Soba, soba},
};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

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
