pub mod soba;
pub mod sporocila;
pub mod user;

//pub use soba::*;
//pub use sporocila::*;
//pub use user::*;



#[cfg(test)]
mod tests {
    use crate::podatkovni_tipi::{user::Client, soba::Soba, sporocila::Message};

    // za uporabnika
    #[test]
    fn test_user_creation() {
        let u = Client::new(1, "alina");
        assert_eq!(u.username, "alina");
        assert_eq!(u.id, 1);
    }

    #[test]
    fn test_user_equality() {
        let u1 = Client::new(1, "alina");
        let u2 = Client::new(1, "alina");
        assert_eq!(u1, u2);
    }

    #[test]
    fn test_user_inequality() {
        let u1 = Client::new(1, "alina");
        let u2 = Client::new(2, "bob");
        assert_ne!(u1, u2);
    }

    // za sobe
    #[test]
    fn test_soba_add_user() {
        let mut r = Soba::new("general");
        r.dodaj_uporabnika(Client::new(1, "alina"));
        assert_eq!(r.users.len(), 1);
    }

    #[test]
    fn test_soba_prazno() {
        let mut r = Soba::new("general");
        assert!(r.users.is_empty());
        assert!(r.history.is_empty());
    }

    // za sporočila
    #[test]
    fn test_message_add() {
        let mut r = Soba::new("general");
        let msg = Message::new(1, "hello", 123);
        r.dodaj_sporocilo(msg);
        assert_eq!(r.history.len(), 1);
    }

    #[test]
    fn test_message_add_multiple() {
        let mut r = Soba::new("general");
        r.dodaj_sporocilo(Message::new(1, "hello", 100));
        r.dodaj_sporocilo(Message::new(2, "world", 200));
        r.dodaj_sporocilo(Message::new(1, "bye", 300));
        assert_eq!(r.history.len(), 3);
    }

    #[test]
    fn test_message_order_preserved() {
        let mut r = Soba::new("general");
        r.dodaj_sporocilo(Message::new(1, "first", 100));
        r.dodaj_sporocilo(Message::new(1, "second", 200));
        assert_eq!(r.history[0].content, "first");
        assert_eq!(r.history[1].content, "second");
    }
}

// ali želimo da lahko nekdo pošlje prazno sporočilo?
