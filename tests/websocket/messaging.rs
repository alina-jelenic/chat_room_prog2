use crate::integration::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    websocket_request,
};
use chat_room_prog2::{
    controller::{
        auth::{SESSION_COOKIE, create_jwt},
        tipi::MESSAGE_COOLDOWN,
    },
    entities::{client, message, prelude::Soba, soba},
};
use futures_util::{SinkExt, StreamExt};

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;
const TEST_SECRET: &str = "test-secret-that-is-longer-than-32-characters";

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
