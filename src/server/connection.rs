use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use crate::server::state::ServerState;

pub struct Connection {
    pub username: String,
    pub stream: TcpStream,
    pub state: Arc<Mutex<ServerState>>,
}

impl Connection {
    pub fn new(username: String, stream: TcpStream, state: Arc<Mutex<ServerState>>) -> Self {
        Self { username, stream, state }
    }

    // skrbi za klienta
    pub async fn handle(mut self) {
        // kasneje: branje sporočil, dodajanje v sobe, broadcast ...
    }
}
