use crate::common::{
    form_request, recv_until, register_and_login, start_server, test_app, websocket_request,
};
use chat_room_prog2::{
    controller::tipi::MESSAGE_COOLDOWN,
    entities::{message, prelude::Soba, soba},
};
use futures_util::SinkExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

#[tokio::test]
async fn websocket_reply_includes_quoted_preview_of_original_message() {
    let (app, _db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "general", &cookie))
        .await
        .unwrap();

    socket
        .send(WsMessage::Text(
            r#"{"content":"prvo sporočilo za odgovor"}"#.into(),
        ))
        .await
        .unwrap();
    let original = timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "prvo sporočilo za odgovor"),
    )
    .await
    .expect("prvo sporočilo ni bilo prejeto");

    // ID izvirnega sporočila razberemo iz atributa id="msg-N" v prejetem HTML-ju,
    // ker ga strežnik ne vrne ločeno kot strukturiran podatek.
    let original_id: i32 = original
        .split("id=\"msg-")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|id| id.parse().ok())
        .expect("id izvirnega sporočila ni bilo mogoče razbrati");
    sleep(MESSAGE_COOLDOWN + Duration::from_millis(100)).await;
    socket
        .send(WsMessage::Text(
            format!(r#"{{"content":"to je odgovor","reply_to_id":"{original_id}"}}"#).into(),
        ))
        .await
        .unwrap();

    let reply = timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "to je odgovor"),
    )
    .await
    .expect("odgovor ni bil prejet");

    assert!(reply.contains("reply-quote"));
    assert!(reply.contains("alina"));
    assert!(reply.contains("prvo sporočilo za odgovor"));

    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn reply_to_message_from_another_room_is_ignored() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;

    app.clone()
        .oneshot(form_request("POST", "/rooms", "name=druga", Some(&cookie)))
        .await
        .unwrap();
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    // Sporočilo namenoma vstavimo v #general, nato preverimo, da povezava
    // v sobi #druga nanj ne more odgovoriti.
    let foreign_message = message::ActiveModel {
        sender_id: Set(None),
        content: Set("sporočilo iz sobe general".to_string()),
        timestamp: Set(1),
        soba_id: Set(general.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "druga", &cookie))
        .await
        .unwrap();

    socket
        .send(WsMessage::Text(
            format!(
                r#"{{"content":"poskus odgovora čez sobe","reply_to_id":"{}"}}"#,
                foreign_message.id
            )
            .into(),
        ))
        .await
        .unwrap();

    let received = timeout(
        Duration::from_secs(2),
        recv_until(&mut socket, "poskus odgovora čez sobe"),
    )
    .await
    .expect("sporočilo ni bilo prejeto");

    assert!(!received.contains("reply-quote"));

    socket.close(None).await.unwrap();
    server.abort();
}
