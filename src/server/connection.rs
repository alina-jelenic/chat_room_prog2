pub struct Connection {
    pub username: String,
}

impl Connection {
    pub fn new(username: String) -> Self {
        Self { username }
    }

    pub async fn handle(&self) {
        
    }
}