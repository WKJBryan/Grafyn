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
    #[serde(default)]
    pub items: Vec<ParsedConversation>,
    #[serde(default)]
    pub total_items: usize,
}

/// Reviewable semantic wikilink suggestion from local import linking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportLinkSuggestion {
    pub from_title: String,
    pub to_title: String,
    pub reason: String,
}

/// Result of applying an import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub note_ids: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub semantic_link_suggestions: Vec<ImportLinkSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_link_error: Option<String>,
    pub message: String,
}
