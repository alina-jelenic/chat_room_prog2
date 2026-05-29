use std::collections::HashMap;
use tokio::sync::broadcast;

use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::io::{BufReader, AsyncBufReadExt, AsyncWriteExt};


use crate::podatkovni_tipi::{soba::Soba, user::Client};
// v serverstate dod novo spremenljivo za bazo podatkov, ki bo shranjevala sobe in uporabnike, namesto sob in uporabnikov
// tx se bo spremenil v HashMap<String, broadcast::Sender<String>>, kjer bo ključ ime sobe, vrednost pa kanal za to sobo
pub struct ServerState {
    pub sobe: HashMap<String, Soba>,
    pub uporabniki: HashMap<u64, Client>,
    pub tx: broadcast::Sender<String>,
}

impl ServerState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel::<String>(64);
        Self {
            sobe: HashMap::new(),
            uporabniki: HashMap::new(),
            tx: tx,
        }
    }

    pub fn create_room(&mut self, name: &str) {
        self.sobe.insert(name.to_string(), Soba::new(name));
    }

    pub fn add_user(&mut self, user: Client) {
        self.uporabniki.insert(user.id, user);
    }

    pub fn get_room_mut(&mut self, name: &str) -> Option<&mut Soba> {
        self.sobe.get_mut(name)
    }
}


pub struct Connection<S> {
    pub username: String,
    pub stream: S,
    pub state: Arc<Mutex<ServerState>>,
}


