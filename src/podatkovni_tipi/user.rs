#[derive(Debug, Clone, PartialEq, Eq)]

pub struct Client {
    pub id: u64,
    pub username: String,
}

impl Client {
    pub fn new(id: u64, username: impl Into<String>) -> Self {
        Self { id, username: username.into() }
    }

    pub async fn connect(&self) {
        
    }
}