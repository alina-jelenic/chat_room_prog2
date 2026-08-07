use crate::common::{body_text, form_request, register_and_login, test_app};
use axum::http::StatusCode;
use chat_room_prog2::entities::{
    client, message,
    prelude::{Client, Soba},
    soba,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tower::ServiceExt;

#[tokio::test]
async fn message_search_is_room_scoped_authorized_and_html_escaped() {
    let (app, db) = test_app().await;
    let owner_cookie = register_and_login(&app, "alina").await;
    let outsider_cookie = register_and_login(&app, "jovan").await;
    let owner = Client::find()
        .filter(client::Column::Username.eq("alina"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    app.clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=iskanje",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    let room = Soba::find()
        .filter(soba::Column::Name.eq("iskanje"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let general = Soba::find()
        .filter(soba::Column::Name.eq("general"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    for (content, room_id) in [
        ("Rust vsebuje <oznako>&", room.id),
        ("neujemajoče sporočilo", room.id),
        ("rust iz druge sobe", general.id),
    ] {
        message::ActiveModel {
            sender_id: Set(Some(owner.id as i64)),
            content: Set(content.to_string()),
            timestamp: Set(1),
            soba_id: Set(room_id),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/iskanje/messages/search?q=rust",
            "",
            Some(&owner_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let results = body_text(response).await;
    assert!(results.contains("Rust vsebuje &lt;oznako&gt;&amp;"));
    assert!(!results.contains("neujemajoče sporočilo"));
    assert!(!results.contains("rust iz druge sobe"));
    assert!(results.contains("Rezultati za »rust«"));
    assert!(results.contains("id=\"search-msg-"));

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms/iskanje/messages/search?q=rust",
            "",
            Some(&outsider_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let panel = body_text(
        app.clone()
            .oneshot(form_request(
                "GET",
                "/rooms/iskanje/panel",
                "",
                Some(&owner_cookie),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(panel.contains("Išči po zgodovini"));
    assert!(panel.contains("/rooms/iskanje/messages/search"));
}
