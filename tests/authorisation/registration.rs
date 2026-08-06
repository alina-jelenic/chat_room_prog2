use crate::integration::{body_text, form_request, register_and_login, test_app};

use chat_room_prog2::{
    controller::forms::normalize_username,
    entities::{client, prelude::Client},
};

use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait, Set};

use tower::ServiceExt;

#[tokio::test]
async fn invalid_and_case_insensitive_duplicate_usernames_are_rejected() {
    let (app, db) = test_app().await;

    for body in [
        "username=alina&password=kratko&confirm=drugo",
        "username=alina&password=12345&confirm=12345",
    ] {
        let response = app
            .clone()
            .oneshot(form_request("POST", "/api/register", body, None))
            .await
            .unwrap();
        assert!(body_text(response).await.contains("server-msg error"));
    }
    assert_eq!(Client::find().count(&db).await.unwrap(), 0);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/register",
            "username=%3Cscript%3E&password=skrivnost1&confirm=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Uporabniško ime"));
    assert_eq!(Client::find().count(&db).await.unwrap(), 0);

    register_and_login(&app, "Alina").await;
    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/register",
            "username=ALINA&password=skrivnost1&confirm=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("že zasedeno"));
    assert_eq!(Client::find().count(&db).await.unwrap(), 1);

    // Unikatnost ni samo preverba v handlerju, temveč tudi omejitev baze.
    let duplicate = client::ActiveModel {
        username: Set("alina".to_string()),
        geslo: Set("ni-pomembno".to_string()),
        ..Default::default()
    }
    .insert(&db)
    .await;
    assert!(duplicate.is_err());
}

#[test]
fn username_rules_are_deterministic() {
    assert_eq!(normalize_username("  Alina_2  ").unwrap(), "alina_2");
    assert!(normalize_username("ab").is_err());
    assert!(normalize_username("2alina").is_err());
    assert!(normalize_username("ime s presledkom").is_err());
    assert!(normalize_username("<script>").is_err());
    assert!(normalize_username(&"a".repeat(25)).is_err());
}
