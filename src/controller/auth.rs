use crate::controller::web::AppError;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use time::Duration;

pub const SESSION_COOKIE: &str = "chat_session";
const SESSION_DURATION_SECONDS: usize = 60 * 60 * 24; // 24 ur

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i32,
    pub username: String,
    pub exp: usize,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i32,
    pub username: String,
}

pub fn create_jwt(user_id: i32, username: &str) -> Result<String, AppError> {
    let now = now_as_usize()?;
    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        exp: now + SESSION_DURATION_SECONDS,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret()?.as_bytes()),
    )
    .map_err(|e| AppError(format!("Napaka pri ustvarjanju JWT tokena: {e}")))
}

pub fn verify_jwt(token: &str) -> Option<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret().ok()?.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

pub fn auth_user_from_jar(jar: &CookieJar) -> Option<AuthUser> {
    let token = jar.get(SESSION_COOKIE)?.value();
    let claims = verify_jwt(token)?;

    Some(AuthUser {
        id: claims.sub,
        username: claims.username,
    })
}

pub fn require_auth(jar: &CookieJar) -> Result<AuthUser, Response> {
    auth_user_from_jar(jar).ok_or_else(redirect_to_login)
}

pub fn session_cookie(token: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, token);
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_max_age(Duration::seconds(SESSION_DURATION_SECONDS as i64));
    cookie
}

pub fn removal_cookie() -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, "");
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_max_age(Duration::seconds(0));
    cookie
}

pub fn redirect_to_login() -> Response {
    ([ ("HX-Redirect", "/authorisation.html") ], Html("")).into_response()
}

pub async fn me_handler(jar: CookieJar) -> Response {
    match auth_user_from_jar(&jar) {
        Some(user) => Html(render_user_pill(&user.username)).into_response(),
        None => redirect_to_login(),
    }
}

pub async fn logout_handler(jar: CookieJar) -> Response {
    let jar = jar.remove(removal_cookie());
    (jar, [ ("HX-Redirect", "/authorisation.html") ], Html("")).into_response()
}

fn render_user_pill(username: &str) -> String {
    let safe_username = html_escape(username);
    let initial_raw = username.trim().chars().next().unwrap_or('?').to_string();
    let initial = html_escape(&initial_raw);

    format!(
        r#"<div class="user-pill" id="user-pill">
      <div class="avatar" id="user-avatar">{initial}</div>
      <span class="user-name" id="user-display">{safe_username}</span>
    </div>"#,
        initial = initial,
        safe_username = safe_username,
    )
}

fn jwt_secret() -> Result<String, AppError> {
    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError("JWT_SECRET ni nastavljen v .env".to_string()))?;

    if secret.trim().len() < 32 {
        return Err(AppError(
            "JWT_SECRET mora biti dolg vsaj 32 znakov.".to_string(),
        ));
    }

    Ok(secret)
}

fn now_as_usize() -> Result<usize, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as usize)
        .map_err(|_| AppError("Sistemski čas ni veljaven.".to_string()))
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[allow(dead_code)]
pub fn unauthorized_response() -> Response {
    StatusCode::UNAUTHORIZED.into_response()
}
