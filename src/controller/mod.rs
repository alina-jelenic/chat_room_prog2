pub mod cli;
pub mod tipi;
pub mod web;

// pub use cli::run_tcp;
// pub use web::run_websocket;

#[cfg(test)]
mod tests {
    use super::tipi::ServerState;
    use crate::podatkovni_tipi::user::Client;

    #[test]
    fn test_create_room() {
        let mut state = ServerState::new_test();
        state.create_room("general");
        assert!(state.sobe.contains_key("general"));
    }

    #[test]
    fn test_add_user() {
        let mut state = ServerState::new_test();
        let user = Client::new(1, "alina");
        state.add_user(user);
        assert!(state.uporabniki.contains_key(&1));
    }
}
