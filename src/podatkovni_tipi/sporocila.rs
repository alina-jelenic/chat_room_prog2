#[derive(Debug, Clone)]
pub struct Message {
    pub sender: u64,
    pub content: String,
    pub timestamp: u64
}

impl Message {
    pub fn new(sender: u64, content: impl Into<String>, timestamp: u64) -> Self {
        Self { sender, content: content.into(), timestamp }
    }

    pub fn format(&self) -> String {
        format!("[{}, {}]: {}", self.sender,self.timestamp,  self.content)
    }
}