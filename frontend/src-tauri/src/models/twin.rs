use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserRecordKind {
    #[default]
    Fact,
    Preference,
    ReasoningPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordOrigin {
    #[default]
    User,
    Synthetic,
    Inferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromotionState {
    Candidate,
    AutoPromoted,
    Endorsed,
    Rejected,
    Private,
    NoTrain,
}

impl PromotionState {
    pub fn default_for_origin(origin: &RecordOrigin) -> Self {
        match origin {
            RecordOrigin::User => PromotionState::AutoPromoted,
            RecordOrigin::Synthetic | RecordOrigin::Inferred => PromotionState::Candidate,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordLinkType {
    Supports,
    Contradicts,
    Supersedes,
    DerivedFrom,
    UserDimension,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordLink {
    pub relation: RecordLinkType,
    pub target_record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceEventType {
    SessionCreated,
    SessionUpdated,
    SessionDeleted,
    PromptSubmitted,
    ResponseCompleted,
    ResponseErrored,
    ModelsAdded,
    ResponseRegenerated,
    DebateStarted,
    DebateContinued,
    NoteExported,
    FeedbackRecorded,
    RankingRecorded,
    InsightCaptured,
    TileDeleted,
    ResponseDeleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceRef {
    pub trace_id: String,
    pub event_id: String,
    pub session_id: String,
    #[serde(default)]
    pub tile_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub id: String,
    pub event_type: TraceEventType,
    pub created_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrace {
    pub id: String,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub events: Vec<TraceEvent>,
}

impl SessionTrace {
    pub fn new(session_id: &str) -> Self {
        let now = Utc::now();
        Self {
            id: session_id.to_string(),
            session_id: session_id.to_string(),
            created_at: now,
            updated_at: now,
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub kind: UserRecordKind,
    pub content: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub origin: RecordOrigin,
    pub promotion_state: PromotionState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub links: Vec<RecordLink>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecordCreate {
    pub kind: UserRecordKind,
    pub content: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub origin: RecordOrigin,
    #[serde(default)]
    pub promotion_state: Option<PromotionState>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub links: Vec<RecordLink>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserRecordUpdate {
    pub content: Option<String>,
    pub confidence: Option<f32>,
    pub promotion_state: Option<PromotionState>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub links: Option<Vec<RecordLink>>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanvasFeedbackType {
    Accept,
    Reject,
    Ranking,
    Correction,
    Insight,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanvasResponseRef {
    pub tile_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasFeedbackRequest {
    pub feedback_type: CanvasFeedbackType,
    #[serde(default)]
    pub response: Option<CanvasResponseRef>,
    #[serde(default)]
    pub ranked_responses: Vec<CanvasResponseRef>,
    #[serde(default)]
    pub kind: Option<UserRecordKind>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub links: Vec<RecordLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasFeedbackResult {
    pub trace_event_id: String,
    #[serde(default)]
    pub created_record_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TwinExportRequest {
    #[serde(default)]
    pub eval_percentage: Option<u8>,
    #[serde(default)]
    pub holdout_percentage: Option<u8>,
    #[serde(default)]
    pub bundle_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFileSummary {
    pub path: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub output_dir: String,
    pub train: ExportFileSummary,
    pub eval: ExportFileSummary,
    pub holdout: ExportFileSummary,
    pub manifest_path: String,
    pub included_records: usize,
    pub excluded_records: usize,
}

pub fn default_record_confidence() -> f32 {
    0.8
}
