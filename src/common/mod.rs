#[derive(Debug, Clone)]
pub struct Message {
    pub sender: String,
    pub content: String,
}

impl Message {
    pub fn new(sender: String, content: String) -> Self {
        Self { sender, content }
    }

    pub fn format(&self) -> String {
        format!("[{}]: {}", self.sender, self.content)
    }
}