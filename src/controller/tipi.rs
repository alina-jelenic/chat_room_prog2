use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub type SharedState = Arc<Mutex<ServerState>>;

pub struct ServerState {
    pub soba_tx: HashMap<i32, broadcast::Sender<String>>,
    pub db: DatabaseConnection,
    pub jwt_secret: String,
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

    pub fn new(db: DatabaseConnection, jwt_secret: String) -> SharedState {
        Arc::new(Mutex::new(Self {
            soba_tx: HashMap::new(),
            db,
            jwt_secret,
        }))
    }
}
