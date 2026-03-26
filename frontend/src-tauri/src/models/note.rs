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

/// Extraction mode for distillation
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractionMode {
    /// Algorithm: heading heuristic (≥2 H2 → rules splitting, else TextTiling) + YAKE
    #[default]
    Algorithm,
    /// LLM: structured JSON extraction via OpenRouter (falls back to Algorithm)
    Llm,
}

/// Hub creation policy during distillation
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HubCreatePolicy {
    /// Create hub when a tag appears 3+ times across candidates
    #[default]
    Auto,
    /// Always create hub for every candidate with a suggested hub
    Always,
    /// Never create hubs
    Never,
}

/// What to do when a candidate's title matches an existing note
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeduplicationAction {
    /// Create new note regardless of duplicates
    Create,
    /// Merge content into existing note with matching title
    Merge,
    /// Skip candidates that match existing notes (default)
    #[default]
    Skip,
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
    /// Typed wikilinks with relationship information
    #[serde(default)]
    pub parsed_links: Vec<ParsedLink>,
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
            parsed_links: Vec::new(),
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
    #[serde(default)]
    pub extraction_mode: ExtractionMode,
    #[serde(default)]
    pub hub_policy: HubCreatePolicy,
    #[serde(default)]
    pub dedup_action: DeduplicationAction,
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
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub skipped_duplicates: usize,
    #[serde(default)]
    pub merged_into: Vec<String>,
}

/// Hub update information from distillation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubUpdate {
    pub hub_id: String,
    pub hub_title: String,
    pub action: String,
    pub atomic_ids_added: Vec<String>,
}

/// Semantic relationship type between linked notes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    #[default]
    Related,
    Supports,
    Contradicts,
    Expands,
    Questions,
    Answers,
    Example,
    PartOf,
    /// Bare [[wikilink]] with no type annotation
    Untyped,
}

impl RelationType {
    /// Get the reverse relation for backlinks
    pub fn reverse(&self) -> Self {
        match self {
            RelationType::Supports => RelationType::Related,
            RelationType::Contradicts => RelationType::Contradicts,
            RelationType::Expands => RelationType::Related,
            RelationType::Questions => RelationType::Answers,
            RelationType::Answers => RelationType::Questions,
            RelationType::Example => RelationType::Related,
            RelationType::PartOf => RelationType::Related,
            other => other.clone(),
        }
    }

    /// Parse from string (case-insensitive)
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "related" => RelationType::Related,
            "supports" => RelationType::Supports,
            "contradicts" => RelationType::Contradicts,
            "expands" => RelationType::Expands,
            "questions" => RelationType::Questions,
            "answers" => RelationType::Answers,
            "example" => RelationType::Example,
            "part_of" | "partof" => RelationType::PartOf,
            _ => RelationType::Untyped,
        }
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationType::Related => write!(f, "related"),
            RelationType::Supports => write!(f, "supports"),
            RelationType::Contradicts => write!(f, "contradicts"),
            RelationType::Expands => write!(f, "expands"),
            RelationType::Questions => write!(f, "questions"),
            RelationType::Answers => write!(f, "answers"),
            RelationType::Example => write!(f, "example"),
            RelationType::PartOf => write!(f, "part_of"),
            RelationType::Untyped => write!(f, "untyped"),
        }
    }
}

/// A parsed wikilink with optional relationship type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedLink {
    pub target_title: String,
    pub relation: RelationType,
}

/// A typed edge in the note graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target_id: String,
    pub relation: RelationType,
}

/// Direction of a link (outgoing or backlink)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkDirection {
    Outgoing,
    Backlink,
}

/// Graph neighbor information with relationship type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNeighbor {
    pub note: NoteMeta,
    pub direction: LinkDirection,
    pub relation: RelationType,
}

// Keep old LinkType as alias for backward compat with frontend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    Outgoing,
    Backlink,
}

// ── Chunk-level retrieval types ───────────────────────────────────────────

/// A chunk-level search result: a segment of a note with parent context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResult {
    pub chunk_id: String,
    pub parent_note_id: String,
    pub parent_title: String,
    pub text: String,
    pub start_char: usize,
    pub end_char: usize,
    pub depth_score: f64,
    pub search_score: f32,
    /// Approximate token count (words * 4/3)
    pub token_estimate: usize,
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
    #[serde(default)]
    pub link_ids: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<ZettelLinkCandidate>,
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
