use crate::models::twin_event::{
    BoundedSummary, ClaimAssertion, EventId, Governance, Identifier, RelationshipAssertion,
    RelationshipDirection,
};
use chrono::{DateTime, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub const BASIS_POINTS_MAX: u16 = 10_000;
pub const MAX_PROJECTED_ITEMS: usize = 16_384;
pub const MAX_STATE_LINKS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct AttentionScore(u16);

impl AttentionScore {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(BASIS_POINTS_MAX);

    pub fn new(value: u16) -> Result<Self, StateModelError> {
        if value > BASIS_POINTS_MAX {
            return Err(StateModelError::ScoreOutOfRange(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for AttentionScore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u16::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateModelError {
    ScoreOutOfRange(u16),
    InvalidDigest,
    InvalidRelationshipVariant,
    CollectionTooLarge(&'static str),
}

impl std::fmt::Display for StateModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScoreOutOfRange(value) => {
                write!(
                    formatter,
                    "attention score {value} exceeds 10000 basis points"
                )
            }
            Self::InvalidDigest => formatter.write_str("snapshot ID must be lowercase SHA-256 hex"),
            Self::InvalidRelationshipVariant => {
                formatter.write_str("relationship variant must be sorted and unique")
            }
            Self::CollectionTooLarge(name) => write!(formatter, "{name} exceeds its item limit"),
        }
    }
}

impl std::error::Error for StateModelError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionComponent {
    Relevance,
    Confidence,
    Recency,
    Recurrence,
    Novelty,
    RelationshipMatch,
    GoalMatch,
    Contradiction,
    ConfidenceGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionVector {
    pub relevance: AttentionScore,
    pub confidence: AttentionScore,
    pub recency: AttentionScore,
    pub recurrence: AttentionScore,
    pub novelty: AttentionScore,
    pub relationship_match: AttentionScore,
    pub goal_match: AttentionScore,
    pub contradiction: AttentionScore,
}

impl AttentionVector {
    pub fn component(&self, component: AttentionComponent) -> AttentionScore {
        match component {
            AttentionComponent::Relevance => self.relevance,
            AttentionComponent::Confidence => self.confidence,
            AttentionComponent::Recency => self.recency,
            AttentionComponent::Recurrence => self.recurrence,
            AttentionComponent::Novelty => self.novelty,
            AttentionComponent::RelationshipMatch => self.relationship_match,
            AttentionComponent::GoalMatch => self.goal_match,
            AttentionComponent::Contradiction => self.contradiction,
            AttentionComponent::ConfidenceGap => {
                AttentionScore::new(BASIS_POINTS_MAX.saturating_sub(self.confidence.get()))
                    .expect("bounded confidence gap")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionProfile {
    Recall,
    Decision,
    Simulation,
    Reflection,
    CaptureReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionWeight {
    pub component: AttentionComponent,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionContribution {
    pub component: AttentionComponent,
    pub component_score: AttentionScore,
    pub weight: u16,
    pub weighted_basis_points: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionExplanation {
    pub profile: AttentionProfile,
    pub profile_version: u16,
    pub vector: AttentionVector,
    pub contributions: Vec<AttentionContribution>,
    pub final_score: AttentionScore,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipKey {
    pub subject_id: crate::models::twin_event::EntityId,
    pub predicate: crate::models::twin_event::RelationshipPredicate,
    pub object_id: crate::models::twin_event::EntityId,
    pub direction: RelationshipDirection,
}

impl From<&RelationshipAssertion> for RelationshipKey {
    fn from(value: &RelationshipAssertion) -> Self {
        Self {
            subject_id: value.subject_id.clone(),
            predicate: value.predicate.clone(),
            object_id: value.object_id.clone(),
            direction: value.direction.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RelationshipVariant {
    pub relationships: Vec<RelationshipKey>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelationshipVariant {
    relationships: Vec<RelationshipKey>,
}

impl<'de> Deserialize<'de> for RelationshipVariant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRelationshipVariant::deserialize(deserializer)?;
        let value = Self {
            relationships: raw.relationships,
        };
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

impl RelationshipVariant {
    pub fn new(mut relationships: Vec<RelationshipKey>) -> Self {
        relationships.sort();
        relationships.dedup();
        Self { relationships }
    }

    pub fn global() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<(), StateModelError> {
        if self.relationships.len() > MAX_STATE_LINKS {
            return Err(StateModelError::CollectionTooLarge("relationship_variant"));
        }
        let mut normalized = self.relationships.clone();
        normalized.sort();
        normalized.dedup();
        if normalized != self.relationships {
            return Err(StateModelError::InvalidRelationshipVariant);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedItemKind {
    Observation,
    PendingProposal,
    ReviewedMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedStateItem {
    pub item_id: Identifier,
    pub kind: ProjectedItemKind,
    pub claim: ClaimAssertion,
    #[serde(default)]
    pub summary: Option<BoundedSummary>,
    #[serde(default)]
    pub proposal_event_id: Option<EventId>,
    pub review_event_ids: Vec<EventId>,
    pub governance: Governance,
    pub relationship_variant: RelationshipVariant,
    pub evidence_event_ids: Vec<EventId>,
    pub support_count: u16,
    pub opposition_count: u16,
    pub prior_exact_support_count: u16,
    pub last_confirmed_at: DateTime<Utc>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
    pub superseded_by: Vec<EventId>,
    pub goals: Vec<String>,
    pub tags: Vec<String>,
}

impl ProjectedStateItem {
    pub fn is_temporally_active(&self, reference_time: DateTime<Utc>) -> bool {
        self.superseded_by.is_empty()
            && self.valid_from.is_none_or(|from| reference_time >= from)
            && self.valid_to.is_none_or(|to| reference_time <= to)
    }

    pub fn validate(&self) -> Result<(), StateModelError> {
        self.relationship_variant.validate()?;
        for (name, len) in [
            ("review_event_ids", self.review_event_ids.len()),
            ("evidence_event_ids", self.evidence_event_ids.len()),
            ("superseded_by", self.superseded_by.len()),
            ("goals", self.goals.len()),
            ("tags", self.tags.len()),
        ] {
            if len > MAX_STATE_LINKS {
                return Err(StateModelError::CollectionTooLarge(name));
            }
        }
        for value in self.goals.iter().chain(&self.tags) {
            if value.len() > 256 || value.trim().is_empty() || value.chars().any(char::is_control) {
                return Err(StateModelError::CollectionTooLarge("context value"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalRule {
    Contradiction,
    RepeatedExactClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalDraft {
    pub memory_id: Identifier,
    pub claim: ClaimAssertion,
    pub relationship_variant: RelationshipVariant,
    pub evidence_event_ids: Vec<EventId>,
    pub rules: Vec<ProposalRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContradictionCluster {
    pub cluster_id: Identifier,
    pub relationship_variant: RelationshipVariant,
    pub memory_ids: Vec<Identifier>,
    pub claims: Vec<ClaimAssertion>,
    pub evidence_event_ids: Vec<EventId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalDraftSet {
    pub rule_version: u16,
    pub drafts: Vec<ProposalDraft>,
    pub contradiction_clusters: Vec<ContradictionCluster>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineState {
    Observed,
    Pending,
    Accepted,
    Rejected,
    Superseded,
    Expired,
    PendingConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalStateEntry {
    pub item_id: Identifier,
    pub source_event_id: EventId,
    pub state: TimelineState,
    pub effective_at: DateTime<Utc>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipVariantState {
    pub relationship_variant: RelationshipVariant,
    pub reviewed_memory_ids: Vec<Identifier>,
    pub pending_memory_ids: Vec<Identifier>,
    pub observation_event_ids: Vec<EventId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn parse(value: impl Into<String>) -> Result<Self, StateModelError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(StateModelError::InvalidDigest);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SnapshotId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionSnapshot {
    pub schema_version: u16,
    pub projection_version: u16,
    pub attention_profile_version: u16,
    pub reference_time: DateTime<Utc>,
    pub applied_event_ids: Vec<EventId>,
    pub reviewed_memories: Vec<ProjectedStateItem>,
    pub pending_proposals: Vec<ProjectedStateItem>,
    pub relationship_variants: Vec<RelationshipVariantState>,
    pub timeline: Vec<TemporalStateEntry>,
    pub contradiction_clusters: Vec<ContradictionCluster>,
    pub recent_observations: Vec<ProjectedStateItem>,
    pub snapshot_id: SnapshotId,
}

impl ProjectionSnapshot {
    pub fn validate(&self) -> Result<(), StateModelError> {
        for (name, len) in [
            ("applied_event_ids", self.applied_event_ids.len()),
            ("reviewed_memories", self.reviewed_memories.len()),
            ("pending_proposals", self.pending_proposals.len()),
            ("relationship_variants", self.relationship_variants.len()),
            ("timeline", self.timeline.len()),
            ("contradiction_clusters", self.contradiction_clusters.len()),
            ("recent_observations", self.recent_observations.len()),
        ] {
            if len > MAX_PROJECTED_ITEMS {
                return Err(StateModelError::CollectionTooLarge(name));
            }
        }
        for variant in &self.relationship_variants {
            variant.relationship_variant.validate()?;
        }
        for item in self
            .reviewed_memories
            .iter()
            .chain(&self.pending_proposals)
            .chain(&self.recent_observations)
        {
            item.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionDestination {
    Local,
    Network,
    Export,
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionRequest {
    pub query: String,
    pub relationship_variant: RelationshipVariant,
    pub goals: Vec<String>,
    pub reference_time: DateTime<Utc>,
    pub destination: SelectionDestination,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionCandidate {
    pub item: ProjectedStateItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReasonCode {
    AllowedUse,
    Sensitivity,
    Visibility,
    Review,
    Authority,
    Temporal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedSelection {
    pub item_id: Identifier,
    pub reason: ExclusionReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSelection {
    pub item_id: Identifier,
    pub attention: AttentionExplanation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionTrace {
    pub snapshot_id: SnapshotId,
    pub profile: AttentionProfile,
    pub profile_version: u16,
    pub selected: Vec<RankedSelection>,
    pub excluded: Vec<ExcludedSelection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_scores_are_bounded_basis_points() {
        assert_eq!(AttentionScore::new(10_000).unwrap().get(), 10_000);
        assert_eq!(AttentionScore::MAX.get(), 10_000);
        assert!(AttentionScore::new(10_001).is_err());
        assert!(serde_json::from_str::<AttentionScore>("10001").is_err());
    }

    #[test]
    fn snapshot_and_state_dtos_reject_unknown_json_fields() {
        let value = r#"{"relationships":[],"unknown":true}"#;
        assert!(serde_json::from_str::<RelationshipVariant>(value).is_err());
        let id =
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap();
        assert_eq!(id.as_str().len(), 64);
    }

    #[test]
    fn relationship_variants_are_sorted_and_deduplicated() {
        use crate::models::twin_event::{EntityId, RelationshipPredicate};
        let key = RelationshipKey {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-a").unwrap(),
            direction: RelationshipDirection::Directed,
        };
        assert_eq!(
            RelationshipVariant::new(vec![key.clone(), key])
                .relationships
                .len(),
            1
        );
    }
}
