// Obdelava prijave in registracije.
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::Deserialize;

use crate::controller::auth::{create_jwt, session_cookie};
use crate::controller::tipi::SharedState;
use crate::controller::web::AppError;
use crate::controller::web::is_unique_violation;
use crate::entities::{client, prelude::Client};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
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
pub const PASSWORD_MIN_LENGTH: usize = 6;

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
        return Err("Uporabniško ime lahko vsebuje samo črke, številke, '-' in '_'.".to_string());
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
            .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?;
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
            )));
        }
    };

    if form.password != form.confirm {
        return Ok(Html(
            r#"<div id="register-msg" class="server-msg error">Gesli se ne ujemata.</div>"#
                .to_string(),
        ));
    }

    if form.password.len() < PASSWORD_MIN_LENGTH {
        return Ok(Html(format!(
            r#"<div id="register-msg" class="server-msg error">Geslo mora imeti vsaj {PASSWORD_MIN_LENGTH} znakov.</div>"#
        )));
    }

    let db = state
        .lock()
        .map_err(|_| AppError("Napaka: zaklenjeno stanje strežnika.".to_string()))?
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

    // Geslo zgoščujemo, da ga v bazi ne hranimo v čisti obliki.
    // Naključna sol omogoča varno uporabo enakih gesel pri različnih uporabnikih.
    let hashed = hash_password(&form.password).map_err(AppError)?;

    let insert_result = client::ActiveModel {
        username: Set(username),
        geslo: Set(hashed),
        ..Default::default()
    }
    .insert(&db)
    .await;

    if let Err(e) = insert_result {
        return if is_unique_violation(&e) {
            Ok(Html(
                r#"<div id="register-msg" class="server-msg error">Uporabniško ime je že zasedeno.</div>"#
                    .to_string(),
            ))
        } else {
            Err(e.into())
        };
    }

    Ok(Html(
        r#"<div id="register-msg" class="server-msg"></div>
    <div id="register-kartica" hx-swap-oob="innerHTML" style="text-align:center;">
      <div class="server-msg success" style="display:block; margin-bottom:16px;">
        ✓ Račun je ustvarjen!<br>
      </div>
      <label for="login-tab" class="btn-outline" style="display:inline-block; text-decoration:none; text-align:center; width:100%;">
        Pojdi na prijavo
      </label>
    </div>"#
            .to_string(),
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
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(hash) => hash,
        Err(_) => return Ok(false),
    };

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Administratorsko ponastavi samo račun iz stare baze, ki še nima veljavnega
/// gesla. Funkcija ni HTTP endpoint: uporablja jo lokalni CLI, zato obiskovalec
/// aplikacije ne more prevzeti tujega računa samo s poznavanjem uporabniškega
/// imena.
pub async fn admin_reset_legacy_password(
    db: &DatabaseConnection,
    legacy_username: &str,
    new_username: Option<&str>,
    new_password: &str,
) -> Result<client::Model, String> {
    if new_password.len() < PASSWORD_MIN_LENGTH {
        return Err(format!(
            "Geslo mora imeti vsaj {PASSWORD_MIN_LENGTH} znakov."
        ));
    }

    let legacy_username = legacy_username.trim();
    if legacy_username.is_empty() {
        return Err("Vnesi uporabniško ime legacy računa.".to_string());
    }

    let user = Client::find()
        .filter(client::Column::Username.eq(legacy_username))
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Račun '{legacy_username}' ne obstaja."))?;

    if !user.geslo.is_empty() {
        return Err("Ta račun že ima geslo; CLI je namenjen samo legacy računom.".to_string());
    }

    // Staro ime obenem uskladimo z današnjimi pravili. Če je bilo staro ime
    // neveljavno, lahko administrator kot drugi argument poda novo veljavno ime.
    let normalized_username = normalize_username(new_username.unwrap_or(&user.username))?;
    let name_owner = Client::find()
        .filter(client::Column::Username.eq(&normalized_username))
        .one(db)
        .await
        .map_err(|e| e.to_string())?;
    if name_owner
        .as_ref()
        .is_some_and(|existing| existing.id != user.id)
    {
        return Err(format!(
            "Uporabniško ime '{normalized_username}' je že zasedeno."
        ));
    }

    let hashed = hash_password(new_password)?;
    let mut active: client::ActiveModel = user.into();
    active.username = Set(normalized_username);
    active.geslo = Set(hashed);
    active.update(db).await.map_err(|e| e.to_string())
}
