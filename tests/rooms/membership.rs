use crate::integration::{body_text, form_request, register_and_login, test_app};
use axum::http::StatusCode;
use chat_room_prog2::entities::{
    client,
    prelude::{Client, RoomMember, Soba},
    room_member, soba,
};

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use tower::ServiceExt;

#[tokio::test]
async fn room_membership_controls_listing_history_and_deletion() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=projekt",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("je ustvarjena"));
    let room = Soba::find()
        .filter(soba::Column::Name.eq("projekt"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let member_cookie = register_and_login(&app, "jovan").await;

    let response = app
        .clone()
        .oneshot(form_request("GET", "/rooms", "", Some(&member_cookie)))
        .await
        .unwrap();
    assert!(
        !body_text(response)
            .await
            .contains("data-room-name=\"projekt\"")
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("še nimaš dostopa"));

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/messages",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    for invalid_id in ["", "abc", "-1"] {
        let response = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/rooms/join",
                &format!("id={invalid_id}"),
                Some(&member_cookie),
            ))
            .await
            .unwrap();
        assert!(
            body_text(response)
                .await
                .contains("room-action-message error")
        );
    }

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
    assert!(
        body_text(response)
            .await
            .contains("Zdaj si v sobi #projekt")
    );

    let member = Client::find()
        .filter(client::Column::Username.eq("jovan"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

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
    assert!(body_text(response).await.contains("Že si v sobi"));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    let panel = body_text(response).await;
    assert!(panel.contains("data-current-username=\"jovan\""));
    assert!(!panel.contains("hx-delete=\"/rooms/projekt\""));
    assert!(panel.contains("hx-delete=\"/rooms/projekt/membership\""));

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt/membership",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("kot njen lastnik"));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(owner.id))
            .count(&db)
            .await
            .unwrap(),
        1
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("lahko izbriše samo"));
    assert!(Soba::find_by_id(room.id).one(&db).await.unwrap().is_some());

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt/membership",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let panel = body_text(response).await;
    assert!(panel.contains("room_name=general"));
    assert!(panel.contains("Nisi več v sobi #projekt"));
    assert!(!panel.contains("data-room-name=\"projekt\""));
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .filter(room_member::Column::ClientId.eq(member.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/messages",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/projekt/panel",
            "",
            Some(&member_cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("še nimaš dostopa"));

    let response = app
        .clone()
        .oneshot(form_request(
            "DELETE",
            "/rooms/projekt",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(Soba::find_by_id(room.id).one(&db).await.unwrap().is_none());
    assert_eq!(
        RoomMember::find()
            .filter(room_member::Column::SobaId.eq(room.id))
            .count(&db)
            .await
            .unwrap(),
        0
    );
}
