use tokio::net::TcpListener;
use std::sync::{Arc, Mutex};
use sea_orm::Database;

use crate::controller::{tipi::{Connection, ServerState}, web::run_websocket};

mod controller;
mod podatkovni_tipi;
mod entities;


#[tokio::main]
async fn main(){
    let db = Database::connect("sqlite://./chat.db?mode=rwc")
        .await
        .expect("Ne morem se povezati z bazo");

    //let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let state = ServerState::new(db).await;

    // loop{
    //     // socket - stream kjer se pogovarjal, addr - moj naslov
    //     let (socket, addr) = listener.accept().await?;
    //     // kloniramo naš state
    //     let state = Arc::clone(&state);

    //     let username = addr.to_string();
    //     let conn = Connection::new(username, socket, state);

    //     tokio::spawn(async move {
    //         conn.handle().await;
    //     });

    // }

    // zagnemo websocket strežnik
    // preveri če jevredu error handling
    if let Err(e) = run_websocket(state).await{
        eprintln!("Napaka pri zagonu websocket strežnika: {}", e);
    }

}