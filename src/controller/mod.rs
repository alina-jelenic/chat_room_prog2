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

    #[test]
    fn test_create_duplicate_room() {
        let mut state = ServerState::new_test();
        state.create_room("general");
        state.create_room("general");
        assert_eq!(state.sobe.len(), 1);
    }

    #[test]
    fn test_create_multiple_rooms() {
        let mut state = ServerState::new_test();
        state.create_room("general");
        state.create_room("help");
        state.create_room("friends");
        assert_eq!(state.sobe.len(), 3);
    }

    #[test]
    fn test_room_exists() {
        let mut state = ServerState::new_test();
        state.create_room("general");
        assert!(state.get_room_mut("general").is_some()); 
    }

    #[test]
    fn test_no_room() {
        let mut state= ServerState::new_test();
        assert!(state.get_room_mut("genneral").is_none())
    }

    #[test]
    fn test_multiple_users() {
        let mut state = ServerState::new_test();
        state.add_user(Client::new(1, "alina"));
        state.add_user(Client::new(2, "jovan"));
        state.add_user(Client::new(3, "tinkara"));
        assert_eq!(state.uporabniki.len(), 3)
    }

    #[test]
    fn test_message_filter_own() {
        let username = "127.0.0.1:43532";
        let message = format!("{username}: hello");
        // own message should be filtered out
        assert!(message.starts_with(&format!("{username}:")));
    }

    #[test]
    fn test_message_filter_other() {
        let my_username = "127.0.0.1:43532";
        let message = "127.0.0.1:99999: hello".to_string();
        // other user's message should pass through
        assert!(!message.starts_with(&format!("{my_username}:")));
    }

    #[test]
    fn test_message_filter_system() {
        let username = "127.0.0.1:43532";
        let message = "*** 127.0.0.1:99999 se je pridružil ***".to_string();
        // system messages should always pass through
        assert!(!message.starts_with(&format!("{username}:")));
    }


}
// treba preveriti, kaj se zgodi če dod novo ime pod že znan id