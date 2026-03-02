use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Note status workflow: draft → evidence → canonical
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NoteStatus {
    #[default]
    Draft,
    Evidence,
    Canonical,
}

impl std::fmt::Display for NoteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteStatus::Draft => write!(f, "draft"),
            NoteStatus::Evidence => write!(f, "evidence"),
            NoteStatus::Canonical => write!(f, "canonical"),
        }
    }
}

impl std::str::FromStr for NoteStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "draft" => Ok(NoteStatus::Draft),
            "evidence" => Ok(NoteStatus::Evidence),
            "canonical" => Ok(NoteStatus::Canonical),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

/// Full note object with all fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub content: String,
    pub status: NoteStatus,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub wikilinks: Vec<String>,
    /// Additional frontmatter properties
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

impl Default for Note {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: String::new(),
            title: String::new(),
            content: String::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            wikilinks: Vec::new(),
            properties: HashMap::new(),
        }
    }
}

/// Minimal note metadata for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    pub status: NoteStatus,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Note> for NoteMeta {
    fn from(note: &Note) -> Self {
        Self {
            id: note.id.clone(),
            title: note.title.clone(),
            status: note.status.clone(),
            tags: note.tags.clone(),
            created_at: note.created_at,
            updated_at: note.updated_at,
        }
    }
}

/// Request to create a new note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreate {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub status: NoteStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// Request to update an existing note
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteUpdate {
    pub title: Option<String>,
    pub content: Option<String>,
    pub status: Option<NoteStatus>,
    pub tags: Option<Vec<String>>,
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

/// YAML frontmatter structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteFrontmatter {
    pub title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    /// Catch-all for additional frontmatter fields
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Search result with score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub note: NoteMeta,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// Request for distilling a container note into atomic notes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillRequest {
    #[serde(default = "default_auto")]
    pub mode: String,
    #[serde(default = "default_auto")]
    pub extraction_method: String,
}

fn default_auto() -> String {
    "auto".to_string()
}

/// Response from distilling a note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillResponse {
    #[serde(default)]
    pub created_note_ids: Vec<String>,
    #[serde(default)]
    pub hub_updates: Vec<HubUpdate>,
    #[serde(default)]
    pub container_updated: bool,
    pub message: String,
    #[serde(default)]
    pub extraction_method_used: String,
    #[serde(default = "default_completed")]
    pub status: String,
}

fn default_completed() -> String {
    "completed".to_string()
}

/// Hub update information from distillation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubUpdate {
    pub hub_id: String,
    pub hub_title: String,
    pub action: String,
    pub atomic_ids_added: Vec<String>,
}

/// Graph neighbor information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighbor {
    pub note: NoteMeta,
    pub link_type: LinkType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    Outgoing,
    Backlink,
}

// ── Zettelkasten link discovery types ─────────────────────────────────────

/// A candidate link discovered between notes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZettelLinkCandidate {
    pub target_id: String,
    pub target_title: String,
    pub link_type: String,
    pub confidence: f64,
    pub reason: String,
}

/// Response from the discover_links command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverLinksResponse {
    pub note_id: String,
    pub links: Vec<ZettelLinkCandidate>,
}

/// Request to apply discovered links
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyLinksRequest {
    pub link_ids: Vec<String>,
}

/// Response from applying discovered links
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyLinksResponse {
    pub note_id: String,
    pub links_created: usize,
    pub links_attempted: usize,
}

/// Response from creating a single link
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLinkResponse {
    pub status: String,
    pub source: String,
    pub target: String,
    pub link_type: String,
}
