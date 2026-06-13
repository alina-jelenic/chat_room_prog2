//tukaj koda za obdelavo login in sign up
use axum::{extract::{Form, State}, response::{Html, IntoResponse, Response}};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use crate::entities::{client, prelude::Client};
use crate::controller::tipi::SharedState;
use crate::controller::web::AppError;
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

pub async fn login_handler(State(state): State<SharedState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError>{
    // obdelaj login formo
    let db = state.lock().map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?.db.clone();
    // poiščeš uporabnika v bazi in preveriš geslo
    let user = Client::find().filter(client::Column::Username.eq(&form.username)).one(&db).await?;

    match user {
        None => Ok(Html(
            r#"<div id="login-msg" class="server-msg error">Napačno ime ali geslo.</div>"#
        ).into_response()),
        Some(u) => {
            let ok = verify_password(&form.password, &u.geslo)
                .map_err(|e| AppError(e))?;
            if ok {
                Ok((
                    [("HX-Redirect", "/index.html")],
                    Html(""),
                ).into_response())
            } else {
                Ok(Html(
                    r#"<div id="login-msg" class="server-msg error">Napačno ime ali geslo.</div>"#
                ).into_response())
            }
        }
    }
    
}

pub async fn register_handler(State(state): State<SharedState>,
    Form(form): Form<RegisterForm>,
) -> Result<Html<String>, AppError> {
    // obdelaj register formo
    if form.password != form.confirm {
        return Ok(Html(
            r#"<div class="server-msg error">Gesli se ne ujemata.</div>"#.to_string()
        ));
    }

    if form.password.len() < 6 {
        return Ok(Html(
            r#"<div class="server-msg error">Geslo mora imeti vsaj 6 znakov.</div>"#.to_string()
        ));
    }

    let db = state.lock()
        .map_err(|_| AppError("Napaka: zaklenjen state".to_string()))?
        .db
        .clone();

    let existing = Client::find()
        .filter(client::Column::Username.eq(&form.username))
        .one(&db)
        .await?;

    if existing.is_some() {
        return Ok(Html(
            r#"<div class="server-msg error">Uporabniško ime je že zasedeno.</div>"#.to_string()
        ));
    }

    // hashas zato ker ne želimo samo texta v bazi ( lahko ukradejo) plus dve osebi lahko enako geslo.
    let hashed = hash_password(&form.password)
        .map_err(|e| AppError(e))?;

    client::ActiveModel {
        username: Set(form.username),
        geslo: Set(hashed),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    Ok(Html(
        r#"<div class="server-msg success">Račun ustvarjen! <a href="/authorisation.html">Prijavi se</a></div>"#.to_string()
    ))
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
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| e.to_string())?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}