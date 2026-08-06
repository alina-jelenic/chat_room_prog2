use crate::common::{
    body_text, form_request, recv_until, register_and_login, start_server, test_app,
    websocket_request,
};
use chat_room_prog2::entities::{
    client, message,
    prelude::{Client, MessageReactions, Soba},
    soba,
};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

#[tokio::test]
async fn reaction_toggle_updates_counts_and_broadcasts_to_room() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=reakcije",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("reakcije"))
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

    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let target_message = message::ActiveModel {
        sender_id: Set(Some(owner.id as i64)),
        content: Set("sporočilo za reakcijo".to_string()),
        timestamp: Set(1),
        soba_id: Set(room.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let message_id = target_message.id;

    let (address, server) = start_server(app).await;
    let (mut owner_socket, _) =
        connect_async(websocket_request(address, "reakcije", &owner_cookie))
            .await
            .unwrap();
    let (mut member_socket, _) =
        connect_async(websocket_request(address, "reakcije", &member_cookie))
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

    // Član doda reakcijo 👍.
    member_socket
        .send(WsMessage::Text(
            format!(r#"{{"reaction_message_id":"{message_id}","reaction_emoji":"👍"}}"#).into(),
        ))
        .await
        .unwrap();

    let owner_saw_reaction = timeout(Duration::from_secs(2), recv_until(&mut owner_socket, "👍"))
        .await
        .expect("lastnik ni prejel obvestila o reakciji");
    assert!(owner_saw_reaction.contains(&format!("id=\"reactions-{message_id}\"")));
    assert!(owner_saw_reaction.contains("👍 1"));

    let member_saw_reaction = timeout(Duration::from_secs(2), recv_until(&mut member_socket, "👍"))
        .await
        .expect("član ni prejel potrditve svoje reakcije");
    assert!(member_saw_reaction.contains("👍 1"));

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 1);

    // Ponoven klik na isto reakcijo jo mora odstraniti (toggle).
    member_socket
        .send(WsMessage::Text(
            format!(r#"{{"reaction_message_id":"{message_id}","reaction_emoji":"👍"}}"#).into(),
        ))
        .await
        .unwrap();

    let owner_saw_removal = timeout(
        Duration::from_secs(2),
        recv_until(&mut owner_socket, &format!("id=\"reactions-{message_id}\"")),
    )
    .await
    .expect("lastnik ni prejel obvestila o odstranitvi reakcije");
    assert!(!owner_saw_removal.contains("👍"));

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 0);

    owner_socket.close(None).await.unwrap();
    member_socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn reaction_is_ignored_for_a_message_outside_the_current_room() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=prva",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));

    // Sporočilo namenoma vstavimo v sobo #general, ne v #prva — nato bova
    // preverila, da povezava, odprta na #prva, nanj ne more reagirati.
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let foreign_message = message::ActiveModel {
        sender_id: Set(None),
        content: Set("sporočilo iz druge sobe".to_string()),
        timestamp: Set(1),
        soba_id: Set(general.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let (address, server) = start_server(app).await;
    let (mut socket, _) = connect_async(websocket_request(address, "prva", &owner_cookie))
        .await
        .unwrap();

    socket
        .send(WsMessage::Text(
            format!(
                r#"{{"reaction_message_id":"{}","reaction_emoji":"👍"}}"#,
                foreign_message.id
            )
            .into(),
        ))
        .await
        .unwrap();

    // Ker sporočilo ne pripada sobi #prva, se reakcija tiho zavrže —
    // strežnik ne sme ničesar poslati nazaj.
    let outcome = timeout(Duration::from_millis(300), socket.next()).await;
    assert!(
        outcome.is_err(),
        "strežnik ne bi smel poslati ničesar za sporočilo iz druge sobe"
    );

    assert_eq!(MessageReactions::find().count(&db).await.unwrap(), 0);

    server.abort();
}
