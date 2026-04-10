use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub status: NoteStatus,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub migration_source: Option<String>,
    #[serde(default)]
    pub optimizer_managed: bool,
    pub wikilinks: Vec<String>,
    /// Typed wikilinks with relationship information
    #[serde(default)]
    pub parsed_links: Vec<ParsedLink>,
    /// Additional frontmatter properties
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

pub const PROP_IS_TOPIC_HUB: &str = "is_topic_hub";
pub const PROP_TOPIC_KEY: &str = "topic_key";
pub const PROP_TOPIC_HUB_IDS: &str = "topic_hub_ids";
pub const PROP_TOPIC_ALIASES: &str = "topic_aliases";
pub const PROP_INFERRED_LINK_IDS: &str = "inferred_link_ids";
pub const PROP_AUTO_INSERTED_LINK_IDS: &str = "auto_inserted_link_ids";
pub const CURRENT_NOTE_SCHEMA_VERSION: u32 = 2;

impl Default for Note {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: String::new(),
            title: String::new(),
            content: String::new(),
            relative_path: String::new(),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: HashMap::new(),
        }
    }
}

impl Note {
    pub fn is_topic_hub(&self) -> bool {
        self.properties
            .get(PROP_IS_TOPIC_HUB)
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || self.title.starts_with("Hub: ")
            || self.tags.iter().any(|tag| tag == "hub")
    }

    pub fn topic_key(&self) -> Option<String> {
        self.properties
            .get(PROP_TOPIC_KEY)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub fn topic_hub_ids(&self) -> Vec<String> {
        self.properties
            .get(PROP_TOPIC_HUB_IDS)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn topic_aliases(&self) -> Vec<String> {
        self.properties
            .get(PROP_TOPIC_ALIASES)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn inferred_link_ids(&self) -> Vec<String> {
        self.properties
            .get(PROP_INFERRED_LINK_IDS)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn auto_inserted_link_ids(&self) -> Vec<String> {
        self.properties
            .get(PROP_AUTO_INSERTED_LINK_IDS)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn set_inferred_link_ids(&mut self, values: Vec<String>) {
        set_optional_string_array(&mut self.properties, PROP_INFERRED_LINK_IDS, values);
    }

    pub fn set_auto_inserted_link_ids(&mut self, values: Vec<String>) {
        set_optional_string_array(&mut self.properties, PROP_AUTO_INSERTED_LINK_IDS, values);
    }

    pub fn set_topic_hub_metadata(
        &mut self,
        is_topic_hub: bool,
        topic_key: Option<String>,
        topic_hub_ids: Vec<String>,
        topic_aliases: Vec<String>,
    ) {
        set_optional_bool(&mut self.properties, PROP_IS_TOPIC_HUB, is_topic_hub);
        set_optional_string(&mut self.properties, PROP_TOPIC_KEY, topic_key);
        set_optional_string_array(&mut self.properties, PROP_TOPIC_HUB_IDS, topic_hub_ids);
        set_optional_string_array(&mut self.properties, PROP_TOPIC_ALIASES, topic_aliases);
    }
}

fn set_optional_bool(properties: &mut HashMap<String, Value>, key: &str, value: bool) {
    if value {
        properties.insert(key.to_string(), Value::Bool(true));
    } else {
        properties.remove(key);
    }
}

fn set_optional_string(properties: &mut HashMap<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        properties.insert(key.to_string(), Value::String(value));
    } else {
        properties.remove(key);
    }
}

fn set_optional_string_array(
    properties: &mut HashMap<String, Value>,
    key: &str,
    values: Vec<String>,
) {
    let cleaned = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    if cleaned.is_empty() {
        properties.remove(key);
    } else {
        properties.insert(
            key.to_string(),
            Value::Array(cleaned.into_iter().map(Value::String).collect()),
        );
    }
}

/// Minimal note metadata for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub status: NoteStatus,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub migration_source: Option<String>,
    #[serde(default)]
    pub optimizer_managed: bool,
}

impl From<&Note> for NoteMeta {
    fn from(note: &Note) -> Self {
        Self {
            id: note.id.clone(),
            title: note.title.clone(),
            relative_path: note.relative_path.clone(),
            aliases: note.aliases.clone(),
            status: note.status.clone(),
            tags: note.tags.clone(),
            created_at: note.created_at,
            updated_at: note.updated_at,
            schema_version: note.schema_version,
            migration_source: note.migration_source.clone(),
            optimizer_managed: note.optimizer_managed,
        }
    }
}

/// Request to create a new note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreate {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub status: NoteStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_note_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub migration_source: Option<String>,
    #[serde(default)]
    pub optimizer_managed: bool,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// Request to update an existing note
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteUpdate {
    pub title: Option<String>,
    pub content: Option<String>,
    pub relative_path: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub status: Option<NoteStatus>,
    pub tags: Option<Vec<String>>,
    pub schema_version: Option<u32>,
    pub migration_source: Option<String>,
    pub optimizer_managed: Option<bool>,
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

/// YAML frontmatter structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteFrontmatter {
    #[serde(default)]
    pub note_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_note_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub migration_source: Option<String>,
    #[serde(default)]
    pub optimizer_managed: bool,
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
    #[serde(default)]
    pub target_path: Option<String>,
    pub relation: RelationType,
}

/// A typed edge in the note graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target_id: String,
    pub relation: RelationType,
    #[serde(default = "default_graph_edge_provenance")]
    pub provenance: GraphEdgeProvenance,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Note,
    TopicHub,
}

impl std::fmt::Display for GraphNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphNodeKind::Note => write!(f, "note"),
            GraphNodeKind::TopicHub => write!(f, "topic_hub"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    NoteLink,
    TopicMembership,
    TopicRelated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeProvenance {
    #[default]
    Explicit,
    Inferred,
    Topic,
    AutoInserted,
}

impl std::fmt::Display for GraphEdgeProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphEdgeProvenance::Explicit => write!(f, "explicit"),
            GraphEdgeProvenance::Inferred => write!(f, "inferred"),
            GraphEdgeProvenance::Topic => write!(f, "topic"),
            GraphEdgeProvenance::AutoInserted => write!(f, "auto_inserted"),
        }
    }
}

impl std::fmt::Display for GraphEdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphEdgeKind::NoteLink => write!(f, "note_link"),
            GraphEdgeKind::TopicMembership => write!(f, "topic_membership"),
            GraphEdgeKind::TopicRelated => write!(f, "topic_related"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopicHubCandidate {
    pub hub_id: String,
    pub hub_title: String,
    pub topic_key: String,
    #[serde(default)]
    pub membership_source: String,
}

// Keep old LinkType as alias for backward compat with frontend
#[allow(dead_code)]
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
    #[serde(default)]
    pub exploratory_links: Vec<ZettelLinkCandidate>,
    #[serde(default)]
    pub topic_hubs: Vec<TopicHubCandidate>,
    #[serde(default)]
    pub cached_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub is_stale: bool,
    #[serde(default = "default_discovery_response_source")]
    pub source: String,
}

fn default_discovery_response_source() -> String {
    "fresh".to_string()
}

fn default_note_schema_version() -> u32 {
    CURRENT_NOTE_SCHEMA_VERSION
}

fn default_graph_edge_provenance() -> GraphEdgeProvenance {
    GraphEdgeProvenance::Explicit
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
