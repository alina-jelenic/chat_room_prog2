use crate::common::{assert_login_redirect, body_text, form_request, test_app};
use axum::http::{StatusCode, header::SET_COOKIE};

use tower::ServiceExt;

#[tokio::test]
async fn protected_http_endpoints_require_a_valid_session() {
    let (app, _db) = test_app().await;

    for request in [
        form_request("GET", "/me", "", None),
        form_request("GET", "/rooms", "", None),
        form_request("GET", "/rooms/general/panel", "", None),
        form_request("GET", "/rooms/general/messages", "", None),
        form_request("POST", "/rooms", "name=skrivna", None),
        form_request("POST", "/rooms/join", "id=123456", None),
        form_request("DELETE", "/rooms/skrivna/membership", "", None),
        form_request("DELETE", "/rooms/skrivna", "", None),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_login_redirect(&response);
    }

    let response = app
        .clone()
        .oneshot(form_request(
            "GET",
            "/rooms",
            "",
            Some("chat_session=ponarejen-token"),
        ))
        .await
        .unwrap();
    assert_login_redirect(&response);
}

#[tokio::test]
async fn login_me_and_logout_manage_the_session_cookie() {
    let (app, _db) = test_app().await;

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=neobstaja&password=napacno",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Napačno ime ali geslo"));

    let register_body = "username=Alina&password=skrivnost1&confirm=skrivnost1";
    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/register", register_body, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=alina&password=napacno",
            None,
        ))
        .await
        .unwrap();
    assert!(body_text(response).await.contains("Napačno ime ali geslo"));

    let response = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/api/login",
            "username=ALINA&password=skrivnost1",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.headers()["HX-Redirect"], "/index.html");
    let set_cookie = response.headers()[SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("chat_session="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Path=/"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(form_request("GET", "/me", "", Some(&cookie)))
        .await
        .unwrap();
    let me = body_text(response).await;
    assert!(me.contains("id=\"user-display\">alina</span>"));
    assert!(me.contains("id=\"user-avatar\">a</div>"));

    let response = app
        .clone()
        .oneshot(form_request("POST", "/api/logout", "", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.headers()["HX-Redirect"], "/authorisation.html");
    let removal = response.headers()[SET_COOKIE].to_str().unwrap();
    assert!(removal.contains("chat_session="));
    assert!(removal.contains("Max-Age=0"));
    assert!(removal.contains("Path=/"));
}
