use serde::{Deserialize, Serialize};

/// A recalled note with relevance score and graph context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub note_id: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub tags: Vec<String>,
    pub graph_boost: f32,
    pub total_score: f32,
}

/// A potential contradiction between notes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub note_id: String,
    pub title: String,
    pub snippet: String,
    pub similarity_score: f32,
    pub conflict_type: String,
    pub details: String,
}

/// A claim/decision extracted from a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedClaim {
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub claim_type: String,
    pub confidence: f32,
}

/// Request to recall relevant notes
#[derive(Debug, Clone, Deserialize)]
pub struct RecallRequest {
    pub query: String,
    #[serde(default)]
    pub context_note_ids: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    5
}

/// Request to extract claims from conversation
#[derive(Debug, Clone, Deserialize)]
pub struct ExtractRequest {
    pub messages: Vec<ConversationMessage>,
}

/// A conversation message
#[derive(Debug, Clone, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}
