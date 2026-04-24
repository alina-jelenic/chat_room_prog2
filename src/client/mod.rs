pub struct Client {
    pub username: String,
}

impl Client {
    pub fn new(username: String) -> Self {
        Self { username }
    }

    pub async fn connect(&self) {
        
    }
}