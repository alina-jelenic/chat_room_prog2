use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub type SharedState = Arc<Mutex<ServerState>>;

/// En uporabnik lahko pošlje največ eno sporočilo v tem časovnem razmiku.
/// Omejitev je skupna vsem njegovim zavihkom, povezavam in sobam.
pub const MESSAGE_COOLDOWN: Duration = Duration::from_millis(750);
pub const REACTION_COOLDOWN: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomAccessRevokedReason {
    Left,
    Kicked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomAccessRevoked {
    pub room_id: i32,
    pub user_id: i64,
    pub reason: RoomAccessRevokedReason,
}

pub struct ServerState {
    pub soba_tx: HashMap<i32, broadcast::Sender<String>>,
    pub db: DatabaseConnection,
    pub jwt_secret: String,
    room_access_revoked_tx: broadcast::Sender<RoomAccessRevoked>,
    last_message_at: HashMap<i64, Instant>,
    last_reaction_at: HashMap<i64, Instant>,
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

    pub fn revoke_room_access(&self, room_id: i32, user_id: i64, reason: RoomAccessRevokedReason) {
        let _ = self.room_access_revoked_tx.send(RoomAccessRevoked {
            room_id,
            user_id,
            reason,
        });
    }

    /// Rezervira naslednje pošiljanje. `false` pomeni, da uporabnikov prejšnji
    /// zapis še ni dovolj star. Ker se preverba izvede pod skupnim kratkim
    /// mutexom, omejitve ni mogoče obiti z drugim zavihkom ali drugo sobo.
    pub fn reserve_message_send(&mut self, user_id: i64) -> bool {
        Self::reserve_action(&mut self.last_message_at, user_id, MESSAGE_COOLDOWN)
    }

    pub fn reserve_reaction(&mut self, user_id: i64) -> bool {
        Self::reserve_action(&mut self.last_reaction_at, user_id, REACTION_COOLDOWN)
    }

    fn reserve_action(
        last_action_at: &mut HashMap<i64, Instant>,
        user_id: i64,
        cooldown: Duration,
    ) -> bool {
        let now = Instant::now();

        if last_action_at
            .get(&user_id)
            .is_some_and(|last| now.saturating_duration_since(*last) < cooldown)
        {
            return false;
        }

        last_action_at.insert(user_id, now);
        true
    }

    pub fn new(db: DatabaseConnection, jwt_secret: String) -> SharedState {
        let (room_access_revoked_tx, _) = broadcast::channel(64);

        Arc::new(Mutex::new(Self {
            soba_tx: HashMap::new(),
            db,
            jwt_secret,
            room_access_revoked_tx,
            last_message_at: HashMap::new(),
            last_reaction_at: HashMap::new(),
        }))
    }
}
