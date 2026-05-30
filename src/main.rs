use tokio::net::TcpListener;
use std::sync::{Arc, Mutex};
use sea_orm::Database;

use crate::{controller::{tipi::Connection, tipi::ServerState}};

mod controller;
mod podatkovni_tipi;
mod entities;


#[tokio::main]
async fn main() -> tokio::io::Result<()>{
    let db = Database::connect("sqlite://./chat.db?mode=rwc")
        .await
        .expect("Ne morem se povezati z bazo");

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let state = Arc::new(Mutex::new(ServerState::new(db).await));

    loop{
        // socket - stream kjer se pogovarjal, addr - moj naslov
        let (socket, addr) = listener.accept().await?;
        // kloniramo naš state
        let state = Arc::clone(&state);

        let username = addr.to_string();
        let conn = Connection::new(username, socket, state);

        tokio::spawn(async move {
            conn.handle().await;
        });

    }

}