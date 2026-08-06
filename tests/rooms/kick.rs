use crate::common::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    wait_for_socket_close, websocket_request,
};
use axum::http::StatusCode;
use chat_room_prog2::entities::{
    client,
    prelude::{Client, RoomMember, Soba},
    room_member, soba,
};
use futures_util::SinkExt;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

#[tokio::test]
async fn room_owner_can_kick_a_member_and_their_socket_is_closed() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    app.clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=moderacija",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    let room = Soba::find()
        .filter(soba::Column::Name.eq("moderacija"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    app.clone()
        .oneshot(form_request(
            "POST",
            "/rooms/join",
            &format!("id={}", room.id),
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let member = Client::find()
        .filter(client::Column::Username.eq("jovan"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let members = body_text(
        app.clone()
            .oneshot(form_request(
                "GET",
                "/rooms/moderacija/members",
                "",
                Some(&owner_cookie),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(members.contains("jovan"));
    assert!(members.contains(&format!("/rooms/moderacija/members/{}", member.id)));

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            &format!("/rooms/moderacija/members/{}", owner.id),
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let http_app = app.clone();
    let (address, server) = start_server(app).await;
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "moderacija", &member_cookie))
            .await
            .unwrap();
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

    let response = http_app
        .clone()
        .oneshot(form_request(
            "DELETE",
            &format!("/rooms/moderacija/members/{}", member.id),
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!body_text(response).await.contains("jovan"));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );

    let notification = timeout(
        Duration::from_secs(2),
        recv_until(&mut member_socket, "te je izgnal"),
    )
    .await
    .expect("izgnjeni uporabnik ni prejel obvestila");
    assert!(notification.contains("hx-get=\"/rooms/general/panel\""));
    wait_for_socket_close(&mut member_socket).await;

    let response = http_app
        .oneshot(form_request(
            "GET",
            "/rooms/moderacija/messages",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    server.abort();
}
