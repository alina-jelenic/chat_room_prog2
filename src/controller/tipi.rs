use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub type SharedState = Arc<Mutex<ServerState>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomAccessRevoked {
    pub room_id: i32,
    pub user_id: i32,
}

pub struct ServerState {
    pub soba_tx: HashMap<i32, broadcast::Sender<String>>,
    pub db: DatabaseConnection,
    pub jwt_secret: String,
    room_access_revoked_tx: broadcast::Sender<RoomAccessRevoked>,
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

    pub fn subscribe_to_room_access_revocations(&self) -> broadcast::Receiver<RoomAccessRevoked> {
        self.room_access_revoked_tx.subscribe()
    }

    pub fn revoke_room_access(&self, room_id: i32, user_id: i32) {
        let _ = self
            .room_access_revoked_tx
            .send(RoomAccessRevoked { room_id, user_id });
    }

    pub fn new(db: DatabaseConnection, jwt_secret: String) -> SharedState {
        let (room_access_revoked_tx, _) = broadcast::channel(64);

        Arc::new(Mutex::new(Self {
            soba_tx: HashMap::new(),
            db,
            jwt_secret,
            room_access_revoked_tx,
        }))
    }
}
