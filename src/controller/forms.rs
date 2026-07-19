// tukaj koda za obdelavo login in sign up
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;

use crate::controller::auth::{create_jwt, session_cookie};
use crate::controller::tipi::SharedState;
use crate::controller::web::AppError;
use crate::entities::{client, prelude::Client};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct RegisterForm {
    username: String,
    password: String,
    confirm: String,
}

pub const USERNAME_MIN_LENGTH: usize = 3;
pub const USERNAME_MAX_LENGTH: usize = 24;

/// Uporabniško ime poenotimo na male črke. Začne se s črko, nato pa
/// dovolimo še številke, vezaj in podčrtaj. Tako se `Alina` in `alina`
/// ne obravnavata kot dva različna uporabnika.
pub fn normalize_username(username: &str) -> Result<String, String> {
    let clean = username.trim().to_ascii_lowercase();
    let length = clean.chars().count();

    if !(USERNAME_MIN_LENGTH..=USERNAME_MAX_LENGTH).contains(&length) {
        return Err(format!(
            "Uporabniško ime mora imeti od {USERNAME_MIN_LENGTH} do {USERNAME_MAX_LENGTH} znakov."
        ));
    }

    let mut chars = clean.chars();
    let first = chars
        .next()
        .ok_or_else(|| "Uporabniško ime ne sme biti prazno.".to_string())?;

    if !first.is_ascii_alphabetic() {
        return Err("Uporabniško ime se mora začeti s črko.".to_string());
    }

    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
        return Err(
            "Uporabniško ime lahko vsebuje samo črke, številke, '-' in '_'.".to_string(),
        );
    }

    Ok(clean)
}

pub async fn login_handler(
    jar: CookieJar,
    State(state): State<SharedState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let username = match normalize_username(&form.username) {
        Ok(username) => username,
        Err(_) => return Ok(invalid_login_response()),
    };
    let (db, jwt_secret) = {
        let state = state
            .lock()
            .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?;
        (state.db.clone(), state.jwt_secret.clone())
    };
    let user = Client::find()
        .filter(client::Column::Username.eq(&username))
        .one(&db)
        .await?;

    match user {
        None => Ok(invalid_login_response()),
        Some(u) => {
            let ok = verify_password(&form.password, &u.geslo).map_err(AppError)?;
            if ok {
                let token = create_jwt(u.id, &u.username, &jwt_secret)?;
                let jar = jar.add(session_cookie(token));

                Ok((jar, [("HX-Redirect", "/index.html")], Html("")).into_response())
            } else {
                Ok(invalid_login_response())
            }
        }
    }
}

pub async fn register_handler(
    State(state): State<SharedState>,
    Form(form): Form<RegisterForm>,
) -> Result<Html<String>, AppError> {
    let username = match normalize_username(&form.username) {
        Ok(username) => username,
        Err(message) => {
            return Ok(Html(format!(
                r#"<div id="register-msg" class="server-msg error">{message}</div>"#
            )))
        }
    };

    if form.password != form.confirm {
        return Ok(Html(
            r#"<div id="register-msg" class="server-msg error">Gesli se ne ujemata.</div>"#.to_string(),
        ));
    }

    if form.password.len() < 6 {
        return Ok(Html(
            r#"<div id="register-msg" class="server-msg error">Geslo mora imeti vsaj 6 znakov.</div>"#.to_string(),
        ));
    }

    let db = state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?
        .db
        .clone();

    let existing = Client::find()
        .filter(client::Column::Username.eq(&username))
        .one(&db)
        .await?;

    if existing.is_some() {
        return Ok(Html(
            r#"<div id="register-msg" class="server-msg error">Uporabniško ime je že zasedeno.</div>"#.to_string(),
        ));
    }

    // hashas zato ker ne želimo samo texta v bazi ( lahko ukradejo) plus dve osebi lahko enako geslo.
    let hashed = hash_password(&form.password).map_err(AppError)?;

    client::ActiveModel {
        username: Set(username),
        geslo: Set(hashed),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    Ok(Html(
        r#"<div id="register-msg" class="server-msg success">Račun ustvarjen! <a href="/authorisation.html">Prijavi se</a></div>"#.to_string(),
    ))
}

fn invalid_login_response() -> Response {
    Html(r#"<div id="login-msg" class="server-msg error">Napačno ime ali geslo.</div>"#)
        .into_response()
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| e.to_string())?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}
