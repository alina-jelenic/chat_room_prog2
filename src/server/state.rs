use std::collections::HashMap;
use tokio::sync::broadcast;

use crate::podatkovni_tipi::{soba::Soba, user::Client};

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
