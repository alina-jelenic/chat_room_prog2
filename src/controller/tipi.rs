use crate::podatkovni_tipi::{soba::Soba, user::Client};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub type SharedState = Arc<Mutex<ServerState>>;

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
            tx,
        }
    }

    pub fn shared() -> SharedState {
        Arc::new(Mutex::new(Self::new()))
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
    pub state: SharedState,
}