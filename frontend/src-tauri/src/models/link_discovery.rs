use crate::models::note::ZettelLinkCandidate;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkSuggestionQueueEntry {
    pub note_id: String,
    pub note_title: String,
    #[serde(default)]
    pub links: Vec<ZettelLinkCandidate>,
    #[serde(default)]
    pub exploratory_links: Vec<ZettelLinkCandidate>,
    #[serde(default)]
    pub cached_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub is_stale: bool,
    #[serde(default = "default_queue_source")]
    pub source: String,
    #[serde(default = "default_queue_status")]
    pub status: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub pending_count: usize,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

fn default_queue_source() -> String {
    "cache".to_string()
}

fn default_queue_status() -> String {
    "pending".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkDiscoveryStatus {
    pub enabled: bool,
    pub llm_enabled: bool,
    pub is_running: bool,
    pub queue_size: usize,
    pub pending_notes: usize,
    pub pending_suggestions: usize,
    pub stale_notes: usize,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub current_note_id: Option<String>,
    #[serde(default)]
    pub current_note_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DismissLinkSuggestionResponse {
    pub note_id: String,
    pub removed: bool,
    pub remaining: usize,
}
