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

impl Default for PromotionState {
    fn default() -> Self {
        PromotionState::Candidate
    }
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
    NoteCreated,
    NoteUpdated,
    NoteCanonicalPromoted,
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
    DecisionEpisodeCreated,
    ReflectionCardRecorded,
    MemoryDigestReviewed,
    OutcomeFollowUpRecorded,
    ConstitutionSetupSaved,
    ConstitutionItemReviewed,
    ActionGapReviewed,
    ConstitutionInferenceRun,
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
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub speaker_role: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinInferenceRunSummary {
    pub inference_version: String,
    pub scanned_traces: usize,
    pub scanned_events: usize,
    pub created_records: usize,
    pub updated_records: usize,
    pub auto_promoted_records: usize,
    pub candidate_records: usize,
    pub skipped_rejected_records: usize,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedEvidenceRef {
    pub trace_id: String,
    pub event_id: String,
    pub session_id: String,
    #[serde(default)]
    pub tile_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    pub event_type: TraceEventType,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub prompt_excerpt: Option<String>,
    #[serde(default)]
    pub response_excerpt: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub speaker_role: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinReviewRecord {
    pub record: UserRecord,
    pub evidence_count: usize,
    #[serde(default)]
    pub latest_evidence: Option<ResolvedEvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwinContextRecord {
    pub id: String,
    pub kind: UserRecordKind,
    pub content: String,
    pub confidence: f32,
    pub promotion_state: PromotionState,
    pub evidence_count: usize,
    #[serde(default)]
    pub source_label: Option<String>,
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
pub struct PrimitiveDecisionAssessment {
    #[serde(default)]
    pub stakes: Option<String>,
    #[serde(default)]
    pub reversibility: Option<String>,
    #[serde(default)]
    pub time_horizon: Option<String>,
    #[serde(default)]
    pub uncertainty: Option<String>,
    #[serde(default)]
    pub agency: Option<String>,
    #[serde(default)]
    pub value_tension: Option<String>,
    #[serde(default)]
    pub constraint_pressure: Option<String>,
    #[serde(default)]
    pub taste_aesthetic_pull: Option<String>,
    #[serde(default)]
    pub somatic_signal: Option<String>,
    #[serde(default)]
    pub action_gap_risk: Option<String>,
    #[serde(default)]
    pub outcome_feedback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEpisode {
    pub id: String,
    pub session_id: String,
    pub tile_id: String,
    pub decision: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub stakes: Option<String>,
    #[serde(default)]
    pub initial_leaning: Option<String>,
    #[serde(default)]
    pub selected_response: Option<CanvasResponseRef>,
    #[serde(default)]
    pub chosen_option: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub review_date: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub regret_score: Option<u8>,
    #[serde(default)]
    pub lesson: Option<String>,
    #[serde(default)]
    pub missed_something: Option<String>,
    #[serde(default)]
    pub primitive_assessment: PrimitiveDecisionAssessment,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEpisodeCreate {
    pub id: String,
    pub session_id: String,
    pub tile_id: String,
    pub decision: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub stakes: Option<String>,
    #[serde(default)]
    pub initial_leaning: Option<String>,
    #[serde(default)]
    pub review_date: Option<String>,
    #[serde(default)]
    pub primitive_assessment: PrimitiveDecisionAssessment,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionOutcomeUpdate {
    #[serde(default)]
    pub selected_response: Option<CanvasResponseRef>,
    #[serde(default)]
    pub chosen_option: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub review_date: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub regret_score: Option<u8>,
    #[serde(default)]
    pub lesson: Option<String>,
    #[serde(default)]
    pub missed_something: Option<String>,
    #[serde(default)]
    pub primitive_assessment: Option<PrimitiveDecisionAssessment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecisionMirrorPreset {
    #[default]
    Balanced,
    EvidenceStrict,
    InsightSearch,
    ActionBias,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionMirrorWeights {
    #[serde(default = "default_weight")]
    pub notes_weight: f32,
    #[serde(default = "default_weight")]
    pub approved_records_weight: f32,
    #[serde(default = "default_candidate_record_weight")]
    pub candidate_records_weight: f32,
    #[serde(default = "default_constitution_weight")]
    pub constitution_weight: f32,
    #[serde(default = "default_action_gap_weight")]
    pub action_gaps_weight: f32,
    #[serde(default = "default_recency_weight")]
    pub recency_weight: f32,
    #[serde(default = "default_weight")]
    pub evidence_count_weight: f32,
    #[serde(default = "default_weight")]
    pub outcome_history_weight: f32,
    #[serde(default = "default_contradiction_weight")]
    pub contradiction_weight: f32,
    #[serde(default = "default_weight")]
    pub breadth_weight: f32,
    #[serde(default = "default_weight")]
    pub depth_weight: f32,
    #[serde(default = "default_evidence_grounding_weight")]
    pub evidence_grounding_weight: f32,
    #[serde(default = "default_weight")]
    pub blind_spot_weight: f32,
    #[serde(default = "default_weight")]
    pub counter_position_weight: f32,
    #[serde(default = "default_weight")]
    pub actionability_weight: f32,
    #[serde(default = "default_weight")]
    pub uncertainty_weight: f32,
    #[serde(default = "default_privacy_weight")]
    pub privacy_weight: f32,
    #[serde(default = "default_unsupported_penalty_weight")]
    pub unsupported_penalty_weight: f32,
}

impl Default for DecisionMirrorWeights {
    fn default() -> Self {
        Self {
            notes_weight: 1.0,
            approved_records_weight: 1.0,
            candidate_records_weight: 0.6,
            constitution_weight: 1.25,
            action_gaps_weight: 1.2,
            recency_weight: 0.5,
            evidence_count_weight: 1.0,
            outcome_history_weight: 1.0,
            contradiction_weight: 1.15,
            breadth_weight: 1.0,
            depth_weight: 1.0,
            evidence_grounding_weight: 1.25,
            blind_spot_weight: 1.0,
            counter_position_weight: 1.0,
            actionability_weight: 1.0,
            uncertainty_weight: 1.0,
            privacy_weight: 1.5,
            unsupported_penalty_weight: 1.5,
        }
    }
}

impl DecisionMirrorWeights {
    pub fn for_preset(preset: &DecisionMirrorPreset) -> Self {
        match preset {
            DecisionMirrorPreset::Balanced => Self::default(),
            DecisionMirrorPreset::EvidenceStrict => Self {
                candidate_records_weight: 0.35,
                constitution_weight: 1.35,
                action_gaps_weight: 1.25,
                evidence_count_weight: 1.35,
                evidence_grounding_weight: 1.8,
                privacy_weight: 2.0,
                unsupported_penalty_weight: 2.0,
                ..Self::default()
            },
            DecisionMirrorPreset::InsightSearch => Self {
                candidate_records_weight: 0.85,
                constitution_weight: 1.2,
                action_gaps_weight: 1.35,
                contradiction_weight: 1.5,
                depth_weight: 1.25,
                blind_spot_weight: 1.45,
                counter_position_weight: 1.25,
                unsupported_penalty_weight: 1.25,
                ..Self::default()
            },
            DecisionMirrorPreset::ActionBias => Self {
                action_gaps_weight: 1.45,
                outcome_history_weight: 1.3,
                blind_spot_weight: 1.15,
                counter_position_weight: 1.1,
                actionability_weight: 1.8,
                uncertainty_weight: 0.85,
                ..Self::default()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionMirrorConfig {
    #[serde(default)]
    pub preset: DecisionMirrorPreset,
    #[serde(default)]
    pub weights: DecisionMirrorWeights,
    #[serde(default)]
    pub advanced_enabled: bool,
}

impl Default for DecisionMirrorConfig {
    fn default() -> Self {
        let preset = DecisionMirrorPreset::Balanced;
        Self {
            weights: DecisionMirrorWeights::for_preset(&preset),
            preset,
            advanced_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionMirrorConfigUpdate {
    #[serde(default)]
    pub preset: Option<DecisionMirrorPreset>,
    #[serde(default)]
    pub weights: Option<DecisionMirrorWeights>,
    #[serde(default)]
    pub advanced_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEvidenceSource {
    pub source_type: String,
    pub id: String,
    pub label: String,
    pub weight: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionEvidencePacket {
    #[serde(default)]
    pub selected_sources: Vec<DecisionEvidenceSource>,
    #[serde(default)]
    pub excluded_private_count: usize,
    #[serde(default)]
    pub excluded_rejected_count: usize,
    #[serde(default)]
    pub excluded_no_train_count: usize,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub config_snapshot: Option<DecisionMirrorConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReflectionScores {
    #[serde(default)]
    pub breadth_score: f32,
    #[serde(default)]
    pub depth_score: f32,
    #[serde(default)]
    pub evidence_grounding_score: f32,
    #[serde(default)]
    pub blind_spot_score: f32,
    #[serde(default)]
    pub actionability_score: f32,
    #[serde(default)]
    pub counterargument_score: f32,
    #[serde(default)]
    pub uncertainty_score: f32,
    #[serde(default)]
    pub privacy_score: f32,
    #[serde(default)]
    pub unsupported_claim_count: u32,
    #[serde(default)]
    pub overall_score: f32,
    #[serde(default)]
    pub weighted_breakdown: HashMap<String, f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionCard {
    pub id: String,
    pub decision_episode_id: String,
    pub session_id: String,
    pub tile_id: String,
    pub model_id: String,
    pub content: String,
    #[serde(default)]
    pub cited_note_ids: Vec<String>,
    #[serde(default)]
    pub cited_user_record_ids: Vec<String>,
    #[serde(default)]
    pub cited_constitution_item_ids: Vec<String>,
    #[serde(default)]
    pub cited_action_gap_ids: Vec<String>,
    pub scores: ReflectionScores,
    #[serde(default)]
    pub evidence_packet: DecisionEvidencePacket,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionCardCreate {
    pub decision_episode_id: String,
    pub session_id: String,
    pub tile_id: String,
    pub model_id: String,
    pub content: String,
    #[serde(default)]
    pub cited_note_ids: Vec<String>,
    #[serde(default)]
    pub cited_user_record_ids: Vec<String>,
    #[serde(default)]
    pub cited_constitution_item_ids: Vec<String>,
    #[serde(default)]
    pub cited_action_gap_ids: Vec<String>,
    #[serde(default)]
    pub evidence_packet: Option<DecisionEvidencePacket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEpisodeWithReflections {
    pub episode: DecisionEpisode,
    #[serde(default)]
    pub reflection_cards: Vec<ReflectionCard>,
    #[serde(default)]
    pub feedback_events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConstitutionStatus {
    #[default]
    Candidate,
    Active,
    Softened,
    NotMe,
    Private,
    NoTrain,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionItem {
    pub id: String,
    pub claim: String,
    pub dimension: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default = "default_record_confidence")]
    pub priority: f32,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub status: ConstitutionStatus,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    pub tensions: Vec<String>,
    #[serde(default)]
    pub linked_record_ids: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionItemCreate {
    pub claim: String,
    pub dimension: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default = "default_record_confidence")]
    pub priority: f32,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub status: ConstitutionStatus,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    pub tensions: Vec<String>,
    #[serde(default)]
    pub linked_record_ids: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConstitutionItemUpdate {
    #[serde(default)]
    pub claim: Option<String>,
    #[serde(default)]
    pub dimension: Option<String>,
    #[serde(default)]
    pub scope: Option<Vec<String>>,
    #[serde(default)]
    pub priority: Option<f32>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub status: Option<ConstitutionStatus>,
    #[serde(default)]
    pub tensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionGap {
    pub id: String,
    pub stated_value: String,
    pub revealed_behavior: String,
    #[serde(default)]
    pub driver_hypothesis: Option<String>,
    #[serde(default)]
    pub somatic_taste_signal: Option<String>,
    pub decision_risk: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    pub linked_record_ids: Vec<String>,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub status: ConstitutionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionGapCreate {
    pub stated_value: String,
    pub revealed_behavior: String,
    #[serde(default)]
    pub driver_hypothesis: Option<String>,
    #[serde(default)]
    pub somatic_taste_signal: Option<String>,
    pub decision_risk: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    pub linked_record_ids: Vec<String>,
    #[serde(default = "default_record_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub status: ConstitutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionReviewRequest {
    pub action: MemoryDigestAction,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConstitutionSetup {
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub tastes: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub somatic_cues: Vec<String>,
    #[serde(default)]
    pub action_tendencies: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionInferenceSummary {
    pub scanned_records: usize,
    pub scanned_decisions: usize,
    pub created_constitution_items: usize,
    pub created_action_gaps: usize,
    #[serde(default)]
    pub scanned_behavior_events: usize,
    #[serde(default)]
    pub scanned_notes: usize,
    #[serde(default)]
    pub scanned_interviews: usize,
    #[serde(default)]
    pub auto_active_items: usize,
    #[serde(default)]
    pub review_candidate_items: usize,
    #[serde(default)]
    pub skipped_domain_claims: usize,
    #[serde(default)]
    pub extracted_research_findings: usize,
    #[serde(default)]
    pub pruned_stale_constitution_items: usize,
    #[serde(default)]
    pub pruned_stale_records: usize,
    #[serde(default)]
    pub updated_setup_entries: usize,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDigestState {
    #[default]
    Pending,
    Kept,
    Softened,
    NotMe,
    Private,
    NoTrain,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDigestAction {
    Keep,
    Soften,
    NotMe,
    Private,
    NoTrain,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDigestItem {
    pub id: String,
    pub pattern: String,
    pub evidence_count: usize,
    pub confidence: f32,
    pub trigger_reason: String,
    #[serde(default)]
    pub latest_evidence: Option<ResolvedEvidenceRef>,
    #[serde(default)]
    pub record_ids: Vec<String>,
    #[serde(default)]
    pub state: MemoryDigestState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDigestReviewRequest {
    pub action: MemoryDigestAction,
    #[serde(default)]
    pub rationale: Option<String>,
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
    pub approved_user_records: ExportFileSummary,
    pub candidate_user_records: ExportFileSummary,
    pub rejected_user_records: ExportFileSummary,
    pub decision_mirror_benchmark: ExportFileSummary,
    pub constitution_items: ExportFileSummary,
    pub action_gaps: ExportFileSummary,
    pub manifest_path: String,
    pub included_records: usize,
    pub excluded_records: usize,
}

pub fn default_record_confidence() -> f32 {
    0.8
}

fn default_weight() -> f32 {
    1.0
}

fn default_candidate_record_weight() -> f32 {
    0.6
}

fn default_constitution_weight() -> f32 {
    1.25
}

fn default_action_gap_weight() -> f32 {
    1.2
}

fn default_recency_weight() -> f32 {
    0.5
}

fn default_contradiction_weight() -> f32 {
    1.15
}

fn default_evidence_grounding_weight() -> f32 {
    1.25
}

fn default_privacy_weight() -> f32 {
    1.5
}

fn default_unsupported_penalty_weight() -> f32 {
    1.5
}
