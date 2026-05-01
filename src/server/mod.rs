pub mod connection;
pub mod state;

use tokio::net::TcpListener;
use std::sync::{Arc, Mutex};
use connection::Connection;
use state::ServerState;

// run skrbi za strežnik
pub async fn run() {
    let poslusalec = TcpListener::bind("127.0.0.1:8080").await.unwrap();
    let state = Arc::new(Mutex::new(ServerState::new()));

    loop {
        let (stream, _) = poslusalec.accept().await.unwrap();

        let conn = Connection::new(
            "gost".to_string(),
            stream,
            state.clone(),
        );

        tokio::spawn(async move {
            conn.handle().await;
        });
    }
}


#[cfg(test)]
mod tests {
    use super::state::ServerState;
    use crate::podatkovni_tipi::{soba::Soba, user::Client};

    #[test]
    fn test_create_room() {
        let mut state = ServerState::new();
        state.create_room("general");

        assert!(state.sobe.contains_key("general"));
    }

    #[test]
    fn test_add_user() {
        let mut state = ServerState::new();
        let user = Client::new(1, "alina");

        state.add_user(user);

        assert!(state.uporabniki.contains_key(&1));
    }
}
