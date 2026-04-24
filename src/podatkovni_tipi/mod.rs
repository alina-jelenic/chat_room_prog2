mod soba;
mod sporocila;
mod user;

//pub use soba::*;
//pub use sporocila::*;
//pub use user::*;



#[cfg(test)]
mod tests {
    use crate::podatkovni_tipi::{user::Client, soba::Soba, sporocila::Message};

    #[test]
    fn test_user_creation() {
        let u = Client::new(1, "alina");
        assert_eq!(u.username, "alina");
    }

    #[test]
    fn test_soba_add_user() {
        let mut r = Soba::new("general");
        r.dodaj_uporabnika(Client::new(1, "alina"));
        assert_eq!(r.users.len(), 1);
    }

    #[test]
    fn test_message_add() {
        let mut r = Soba::new("general");
        let msg = Message::new(1, "hello", 123);
        r.dodaj_sporocilo(msg);
        assert_eq!(r.history.len(), 1);
    }
}
