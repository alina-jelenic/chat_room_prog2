use tokio::net::TcpListener;
use std::sync::{Arc, Mutex};

use crate::{podatkovni_tipi::user, server::{connection::Connection, state::ServerState}};

mod server;
mod podatkovni_tipi;


#[tokio::main]
async fn main() -> tokio::io::Result<()>{
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let state = Arc::new(Mutex::new(ServerState::new()));

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