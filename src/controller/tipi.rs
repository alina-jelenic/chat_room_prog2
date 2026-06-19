use crate::podatkovni_tipi::{soba::Soba, user::Client};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use sea_orm::DatabaseConnection;

pub type SharedState = Arc<Mutex<ServerState>>;

pub struct ServerState {
    pub sobe: HashMap<String, Soba>,
    pub uporabniki: HashMap<u64, Client>,
    pub soba_tx: HashMap<i32, broadcast::Sender<String>>,
    pub db: DatabaseConnection,
}

impl ServerState {

        pub fn get_or_create_room_tx(&mut self, soba_id: i32) -> broadcast::Sender<String> {
        self.soba_tx
        .entry(soba_id)
        .or_insert_with(|| {
            let (tx, _) = broadcast::channel::<String>(64);
            tx
        })
        .clone()
}
        pub async fn new(db: DatabaseConnection) -> SharedState {
        let (tx, _) = broadcast::channel::<String>(64);
        Arc::new(Mutex::new(Self {
            sobe: HashMap::new(),
            uporabniki: HashMap::new(),
            soba_tx: HashMap::new(),
            db,
        }))
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

    #[cfg(test)]
    pub fn new_test() -> Self {
        let (tx, _) = broadcast::channel::<String>(64);
        // ustvarimo dummy db connection za teste
        use sea_orm::{DatabaseBackend, MockDatabase};
        let db = MockDatabase::new(DatabaseBackend::Sqlite).into_connection();
        Self {
            sobe: HashMap::new(),
            uporabniki: HashMap::new(),
            soba_tx: HashMap::new(),
            db,
        }
}
}

pub struct Connection<S> {
    pub username: String,
    pub stream: S,
    pub state: SharedState,
}