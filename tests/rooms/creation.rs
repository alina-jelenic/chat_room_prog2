use crate::integration::{body_text, form_request, register_and_login, test_app};
use axum::http::StatusCode;
use chat_room_prog2::entities::{prelude::Soba, soba};

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use tower::ServiceExt;

#[tokio::test]
async fn invalid_and_duplicate_room_names_do_not_create_rooms() {
    let (app, db) = test_app().await;
    let cookie = register_and_login(&app, "alina").await;

    for body in [
        "name=",
        "name=ime+s+presledkom",
        "name=%3Cscript%3E",
        "name=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let response = app
            .clone()
            .oneshot(form_request("POST", "/rooms", body, Some(&cookie)))
            .await
            .unwrap();
        assert!(
            body_text(response)
                .await
                .contains("room-action-message error")
        );
    }
    assert_eq!(Soba::find().count(&db).await.unwrap(), 1);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=general",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že obstaja"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=Rust_Chat",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("#rust_chat"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/rooms",
            "name=RUST_CHAT",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že obstaja"));
    assert_eq!(Soba::find().count(&db).await.unwrap(), 2);
}

#[tokio::test]
async fn concurrent_room_creation_with_same_name_only_creates_one_room() {
    let (app, db) = test_app().await;
    let cookie_a = register_and_login(&app, "prva").await;
    let cookie_b = register_and_login(&app, "druga").await;

    let (response_a, response_b) = tokio::join!(
        app.clone().oneshot(form_request(
            "POST",
            "/rooms",
            "name=dirka",
            Some(&cookie_a)
        )),
        app.clone().oneshot(form_request(
            "POST",
            "/rooms",
            "name=dirka",
            Some(&cookie_b)
        )),
    );
    let response_a = response_a.unwrap();
    let response_b = response_b.unwrap();

    // Oba zahtevka morata dobiti smiseln odgovor, ne surovega strežniškega 500.
    assert_eq!(response_a.status(), StatusCode::OK);
    assert_eq!(response_b.status(), StatusCode::OK);

    let text_a = body_text(response_a).await;
    let text_b = body_text(response_b).await;

    let successes = [&text_a, &text_b]
        .into_iter()
        .filter(|t| t.contains("je ustvarjena"))
        .count();
    assert_eq!(successes, 1, "natanko en zahtevek bi moral uspeti");

    assert_eq!(
        Soba::find()
            .filter(soba::Column::Name.eq("dirka"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
}
