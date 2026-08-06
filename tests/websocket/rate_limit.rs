use crate::common::{recv_until, register_and_login, start_server, test_app, websocket_request};
use chat_room_prog2::{
    controller::tipi::MESSAGE_COOLDOWN,
    entities::{message, prelude::Soba, soba},
};
use futures_util::SinkExt;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

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
