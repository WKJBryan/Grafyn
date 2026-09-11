use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    #[default]
    ProductProject,
    Everyday,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Tentative,
    Confirmed,
    Rejected,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    TargetStatement,
    OtherPersonStatement,
    ModelOutput,
    DomainKnowledge,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    #[default]
    Active,
    Paused,
    Achieved,
    Abandoned,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    #[default]
    Queued,
    Processing,
    Completed,
    NeedsReview,
    Failed,
    Superseded,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    #[default]
    Related,
    Equivalent,
    Conflicts,
    Supports,
    Contradicts,
    Expands,
    Questions,
    Answers,
    Example,
    PartOf,
    Enables,
    Inhibits,
    Requires,
    ContributesTo,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceNodeKind {
    #[default]
    Action,
    Consequence,
    Constraint,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceNode {
    pub id: String,
    pub subject_id: String,
    pub kind: EvidenceNodeKind,
    pub label: String,
    pub receipts: Vec<Receipt>,
    pub review_status: ReviewStatus,
    pub recorded_at: String,
    pub invalidated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InterviewDraft {
    pub id: String,
    pub subject_id: String,
    pub subject_name: String,
    pub domain: Domain,
    pub step: usize,
    pub situation: String,
    pub options: Vec<String>,
    pub wanted: String,
    pub expected: String,
    pub chosen: String,
    pub rejected: Vec<String>,
    pub actual: String,
    pub rationale: String,
    pub constraints: Vec<String>,
    pub goal_ids: Vec<String>,
    pub updated_at: String,
    pub expected_goal_relation: Option<RelationshipKind>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceInput {
    pub id: String,
    pub note_id: String,
    pub title: String,
    /// Only attributable source body, never generated navigation or frontmatter.
    pub text: String,
    pub subject_id: String,
    pub role: EvidenceRole,
    pub restricted: bool,
    pub held_out: bool,
    pub mapping_revision: u64,
    pub source_group: String,
    pub observed_at: Option<String>,
    pub structured_case: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    #[serde(flatten)]
    pub input: SourceInput,
    pub revision: u64,
    pub content_hash: String,
    pub recorded_at: String,
    pub deleted: bool,
    /// Interview sources are managed by this store, not the note inventory.
    pub interview: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Receipt {
    pub source_id: String,
    pub source_revision: u64,
    pub start: usize,
    pub end: usize,
    pub quote: String,
    pub locator: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionCase {
    pub id: String,
    pub subject_id: String,
    pub domain: Domain,
    pub situation: String,
    pub options: Vec<String>,
    pub wanted: String,
    pub expected: String,
    pub chosen: String,
    pub rejected: Vec<String>,
    pub actual: String,
    pub rationale: String,
    pub constraints: Vec<String>,
    pub goal_revisions: Vec<GoalReference>,
    pub receipts: Vec<Receipt>,
    pub review_status: ReviewStatus,
    pub provenance: String,
    pub case_kind: String,
    pub recorded_at: String,
    pub invalidated: bool,
    pub conflict: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalCriterion {
    pub id: String,
    pub metric: Option<String>,
    pub counting_rule: Option<String>,
    pub unit: Option<String>,
    pub population: Option<String>,
    pub denominator: Option<String>,
    pub baseline: Option<f64>,
    pub baseline_observed_at: Option<String>,
    pub comparator: Option<String>,
    pub target: Option<f64>,
    pub upper_target: Option<f64>,
    pub start_anchor: Option<String>,
    pub deadline: Option<String>,
    pub duration: Option<String>,
    pub sustained_period: Option<String>,
    pub measurement_source: Option<String>,
    pub observation_schedule: Option<String>,
    pub observed_progress: Option<f64>,
    pub is_proxy: bool,
    pub field_provenance: std::collections::BTreeMap<String, String>,
    pub receipts: Vec<Receipt>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalInput {
    pub id: String,
    pub subject_id: String,
    pub label: String,
    pub definition: String,
    pub beneficiary: String,
    pub scope: String,
    pub status: GoalStatus,
    pub criteria: Vec<GoalCriterion>,
    pub constraints: Vec<String>,
    pub competing_goal_ids: Vec<String>,
    pub contextual_priority: Option<String>,
    pub effective_at: Option<String>,
    pub reason: String,
    pub receipts: Vec<Receipt>,
    pub review_status: ReviewStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRevision {
    #[serde(flatten)]
    pub input: GoalInput,
    pub revision: u64,
    pub recorded_at: String,
    pub invalidated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalReference {
    pub goal_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Relationship {
    pub id: String,
    pub subject_id: String,
    pub from_id: String,
    pub to_id: String,
    pub relation: RelationshipKind,
    pub directed: bool,
    pub provenance: String,
    pub similarity: Option<f32>,
    pub model_version: Option<String>,
    pub review_status: ReviewStatus,
    pub explanation: String,
    pub from_receipt: Receipt,
    pub to_receipt: Receipt,
    /// target_stated_belief / extracted_hypothesis / empirical_claim; never upgraded by review.
    pub causal_basis: Option<String>,
    pub conditions: Vec<String>,
    pub delay: Option<String>,
    pub magnitude: Option<String>,
    pub goal_criterion_id: Option<String>,
    pub invalidated: bool,
    pub recorded_at: String,
    pub assessment: Option<super::PairAssessment>,
    pub reviewed_context_hash: Option<String>,
    pub review_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingJob {
    pub id: String,
    pub source_id: String,
    pub source_revision: u64,
    pub mapping_revision: u64,
    pub extractor_version: String,
    pub vault_id: String,
    pub status: JobStatus,
    pub error: Option<String>,
    pub attempts: u32,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractionOutput {
    pub statements: Vec<PersonalEvidence>,
    pub nodes: Vec<EvidenceNode>,
    pub cases: Vec<DecisionCase>,
    pub goals: Vec<GoalInput>,
    pub relationships: Vec<Relationship>,
    pub needs_review: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceSnapshot {
    pub statements: Vec<PersonalEvidence>,
    pub nodes: Vec<EvidenceNode>,
    pub schema_version: u32,
    pub subject_id: String,
    pub subject_name: String,
    pub interview_draft: Option<InterviewDraft>,
    pub sources: Vec<SourceRecord>,
    pub cases: Vec<DecisionCase>,
    pub goals: Vec<GoalRevision>,
    pub relationships: Vec<Relationship>,
    pub jobs: Vec<ProcessingJob>,
    pub embedding_status: String,
    pub assessment_status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextRequest {
    pub query: String,
    pub subject_id: String,
    pub as_of: Option<String>,
    pub excluded_source_groups: Vec<String>,
    pub max_cases: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextPacket {
    pub statements: Vec<PersonalEvidence>,
    pub nodes: Vec<EvidenceNode>,
    pub subject_id: String,
    pub assembled_at: String,
    pub cases: Vec<DecisionCase>,
    pub goals: Vec<GoalRevision>,
    pub relationships: Vec<Relationship>,
    pub source_revisions: Vec<Receipt>,
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonalEvidence {
    pub id: String,
    pub subject_id: String,
    pub statement: String,
    /// statement / preference / constraint / decision_procedure.
    pub kind: String,
    pub receipts: Vec<Receipt>,
    pub review_status: ReviewStatus,
    pub provenance: String,
    pub recorded_at: String,
    pub invalidated: bool,
}
