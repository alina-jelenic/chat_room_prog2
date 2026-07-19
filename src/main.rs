use chat_room_prog2::controller::{
    auth::validate_jwt_secret,
    rooms::{ensure_default_room, prepare_database_schema},
    tipi::ServerState,
    web::run_websocket,
};
use sea_orm::Database;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Preberemo .env, če obstaja. To omogoča, da URL baze zamenjamo brez spremembe kode.
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://./chat.db?mode=rwc".to_string());
    let jwt_secret = env::var("JWT_SECRET")
        .map_err(|_| "JWT_SECRET ni nastavljen. Kopiraj .env.example v .env.")?;
    validate_jwt_secret(&jwt_secret)?;

    let db = Database::connect(&database_url).await?;

    // Ključni popravek: migracije morajo teči pred ServerState::new in pred zagonom routerja.
    // S tem se tabele client, soba, message in seaql_migrations ustvarijo samodejno.
    prepare_database_schema(&db).await?;

    // Frontend privzeto odpre sobo #general, zato jo pripravimo že ob zagonu.
    ensure_default_room(&db).await?;

    let state = ServerState::new(db, jwt_secret);

    run_websocket(state).await?;

    Ok(())
}
