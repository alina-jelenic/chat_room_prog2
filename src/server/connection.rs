use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::io::{BufReader, AsyncBufReadExt, AsyncWriteExt};

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
    pub async fn handle(self) {
        // kasneje: branje sporočil, dodajanje v sobe, broadcast ...
        // zaklene stanje da drugi ne morajo dostopat, shranim si kam bom pisal pa od kje bom bral
        let (tx, mut rx) = {
            let s = self.state.lock().unwrap();
            (s.tx.clone(), s.tx.subscribe())
        };

        let (reader, mut writer) = self.stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        let user = self.username;

        let _ = tx.send(format!("*** {user} se je pridružil *** "));
        
        loop{
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            let _ = tx.send(format!("{user}: {l}"));
                        }
                        _ => break,
                        
                    }
                }
                msg = rx.recv() => {
                    if let Ok(m) = msg {
                        if writer.write_all(format!("{m}\n").as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }

        let _ = tx.send(format!("*** {user} je odsel ***"));
    }
}
