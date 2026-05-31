use crate::podatkovni_tipi::user::Client;
use crate::podatkovni_tipi::sporocila::Message;
#[derive(Debug)]
pub struct Soba {
    pub name: String,
    pub users: Vec<Client>,
    pub history: Vec<Message>,
}

impl Soba {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            users: vec![],
            history: vec![],
        }
    }

    pub fn dodaj_uporabnika(&mut self, user: Client) {
        self.users.push(user);
    }

    pub fn dodaj_sporocilo(&mut self, msg: Message) {
        self.history.push(msg);
    }

    pub fn odstrani_uporabnika(&mut self, user_id: u64) {
    self.users.retain(|u| u.id != user_id);
}
}
