use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single message in a parsed conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedMessage {
    pub index: usize,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Metadata about a parsed conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMetadata {
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    pub message_count: usize,
    #[serde(default)]
    pub model_info: Vec<String>,
}

/// A fully parsed conversation ready for import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedConversation {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub messages: Vec<ParsedMessage>,
    pub metadata: ConversationMetadata,
    #[serde(default)]
    pub suggested_tags: Vec<String>,
}

/// Preview of an import file before applying
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreview {
    pub conversations: Vec<ParsedConversation>,
    pub platform: String,
    pub total_conversations: usize,
}

/// Result of applying an import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub note_ids: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    pub message: String,
}
