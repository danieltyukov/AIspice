use serde::{Deserialize, Serialize};

/// A single message in the chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// An event emitted from the AI streaming pipeline to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event_type: String,
    pub data: String,
}

/// Metadata about a single change the AI made (returned after edits).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeInfo {
    pub component: Option<String>,
    pub description: String,
    pub filename: String,
}
