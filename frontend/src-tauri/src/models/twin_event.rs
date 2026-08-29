use chrono::{DateTime, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::fmt;

pub const MAX_CAUSAL_PARENTS: usize = 64;
pub const MAX_EVENT_LINKS: usize = 64;
pub const MAX_EVIDENCE_REFS: usize = 64;
pub const MAX_CONTEXT_ENTITIES: usize = 64;
pub const MAX_CONTEXT_RELATIONSHIPS: usize = 64;
pub const MAX_CONTEXT_VALUES: usize = 64;
pub const MAX_RELATIONSHIP_EVIDENCE: usize = 32;
pub const MAX_CLAIMS: usize = 64;
pub const MAX_DECISION_OPTIONS: usize = 32;
pub const MAX_LABEL_BYTES: usize = 256;
pub const MAX_ROLE_BYTES: usize = 128;
pub const MAX_COST_DECIMAL_BYTES: usize = 64;

fn validate_identifier(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value == "." || value == ".." {
        return Err("identifier must contain 1..=256 safe characters".into());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@'))
    {
        return Err("identifier contains control, whitespace, or path characters".into());
    }
    Ok(())
}

macro_rules! identifier_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier_type!(Identifier);
identifier_type!(ActorId);
identifier_type!(DeviceId);
identifier_type!(EntityId);
identifier_type!(SourceChannel);
identifier_type!(EntityType);
identifier_type!(ClaimPredicate);
identifier_type!(RelationshipPredicate);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || value.contains('\\')
            || value.chars().any(|character| {
                character.is_whitespace()
                    || character.is_control()
                    || !(character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '.' | ':' | '/' | '@'))
            })
            || value
                .split(['/', ':'])
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        {
            return Err("model id contains unsafe or unsupported path material".into());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ModelId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

fn validate_text(value: &str, limit: usize, name: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(format!(
            "{name} must be nonempty bounded text without control characters"
        ));
    }
    Ok(())
}

macro_rules! validated_text_type {
    ($name:ident, $limit:expr, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_text(&value, $limit, $label)?;
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

validated_text_type!(ClaimObject, 4096, "claim object");
validated_text_type!(BoundedSummary, 1024, "summary");
validated_text_type!(BoundedLabel, MAX_LABEL_BYTES, "label");
validated_text_type!(BoundedRole, MAX_ROLE_BYTES, "role or kind");
validated_text_type!(ProvenanceLabel, 512, "provenance");

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DecimalCost(String);

impl DecimalCost {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() > MAX_COST_DECIMAL_BYTES {
            return Err("cost exceeds its byte limit".into());
        }
        validate_decimal(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for DecimalCost {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BoundedContent(String);

impl BoundedContent {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.trim().is_empty()
            || value.len() > 32_768
            || value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(
                "event content must be nonempty bounded text without unsafe controls".into(),
            );
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BoundedContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != 64
            || !value
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err("event id must be a lowercase SHA-256 hex digest".into());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for EventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        EventId::parse(value.clone())
            .map_err(|_| "content digest must be lowercase SHA-256 hex")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContentDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TwinEventType {
    ObservationRecorded,
    NoteChanged,
    ConversationTurnRecorded,
    CanvasResponseRecorded,
    MemoryProposed,
    MemoryReviewed,
    DecisionRecorded,
    DecisionOutcomeRecorded,
    FeedbackRecorded,
    RelationshipContextObserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwinEvent {
    pub schema_version: u16,
    pub event_id: EventId,
    pub event_type: TwinEventType,
    pub actor_id: ActorId,
    pub device_id: DeviceId,
    pub device_sequence: u64,
    pub causal_parents: Vec<EventId>,
    pub recorded_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    #[serde(deserialize_with = "required_option")]
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "required_option")]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "required_option")]
    pub valid_to: Option<DateTime<Utc>>,
    pub supersedes: Vec<EventId>,
    pub reinforces: Vec<EventId>,
    pub context: EventContext,
    pub evidence: Vec<EvidenceRef>,
    pub governance: Governance,
    pub payload: TwinEventPayload,
}

impl TwinEvent {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported Twin event schema version".into());
        }
        if self.device_sequence == 0 {
            return Err("device sequence starts at 1".into());
        }
        if self.event_type != self.payload.event_type() {
            return Err("event type does not match payload variant".into());
        }
        if self
            .valid_from
            .zip(self.valid_to)
            .is_some_and(|(from, to)| from > to)
        {
            return Err("valid_from must not follow valid_to".into());
        }
        if matches!(
            self.payload,
            TwinEventPayload::RelationshipContextObserved(_)
        ) && self.context.relationships.is_empty()
        {
            return Err("relationship context observation requires a relationship".into());
        }
        validate_max_len(&self.causal_parents, MAX_CAUSAL_PARENTS, "causal_parents")?;
        validate_max_len(&self.supersedes, MAX_EVENT_LINKS, "supersedes")?;
        validate_max_len(&self.reinforces, MAX_EVENT_LINKS, "reinforces")?;
        validate_max_len(
            &self.context.entities,
            MAX_CONTEXT_ENTITIES,
            "context.entities",
        )?;
        validate_max_len(
            &self.context.relationships,
            MAX_CONTEXT_RELATIONSHIPS,
            "context.relationships",
        )?;
        validate_max_len(
            &self.context.environments,
            MAX_CONTEXT_VALUES,
            "context.environments",
        )?;
        validate_max_len(
            &self.context.activities,
            MAX_CONTEXT_VALUES,
            "context.activities",
        )?;
        validate_max_len(&self.context.goals, MAX_CONTEXT_VALUES, "context.goals")?;
        validate_max_len(&self.context.tags, MAX_CONTEXT_VALUES, "context.tags")?;
        validate_max_len(&self.evidence, MAX_EVIDENCE_REFS, "evidence")?;
        for value in self
            .context
            .environments
            .iter()
            .chain(&self.context.activities)
            .chain(&self.context.goals)
            .chain(&self.context.tags)
        {
            validate_text(value, 256, "context value")?;
        }
        for relationship in &self.context.relationships {
            validate_max_len(
                &relationship.evidence,
                MAX_RELATIONSHIP_EVIDENCE,
                "relationship.evidence",
            )?;
            if relationship
                .valid_from
                .zip(relationship.valid_to)
                .is_some_and(|(from, to)| from > to)
            {
                return Err("relationship valid_from must not follow valid_to".into());
            }
        }
        validate_unique(&self.causal_parents, "causal_parents")?;
        validate_unique(&self.supersedes, "supersedes")?;
        validate_unique(&self.reinforces, "reinforces")?;
        validate_unique(&self.context.entities, "context.entities")?;
        validate_unique(&self.context.relationships, "context.relationships")?;
        validate_unique(&self.context.environments, "context.environments")?;
        validate_unique(&self.context.activities, "context.activities")?;
        validate_unique(&self.context.goals, "context.goals")?;
        validate_unique(&self.context.tags, "context.tags")?;
        validate_unique(&self.evidence, "evidence")?;
        self.payload.validate()
    }

    pub fn normalize(&mut self) {
        sort_dedup(&mut self.causal_parents);
        sort_dedup(&mut self.supersedes);
        sort_dedup(&mut self.reinforces);
        sort_dedup(&mut self.context.entities);
        for relationship in &mut self.context.relationships {
            sort_dedup(&mut relationship.evidence);
        }
        sort_dedup(&mut self.context.relationships);
        sort_dedup(&mut self.context.environments);
        sort_dedup(&mut self.context.activities);
        sort_dedup(&mut self.context.goals);
        sort_dedup(&mut self.context.tags);
        sort_dedup(&mut self.evidence);
        self.payload.normalize();
    }
}

fn validate_max_len<T>(items: &[T], limit: usize, name: &str) -> Result<(), String> {
    if items.len() > limit {
        return Err(format!("{name} exceeds its {limit}-item limit"));
    }
    Ok(())
}

fn validate_unique<T: Ord + Clone>(items: &[T], name: &str) -> Result<(), String> {
    let mut normalized = items.to_vec();
    sort_dedup(&mut normalized);
    if normalized.len() != items.len() {
        return Err(format!("{name} contains duplicates"));
    }
    Ok(())
}

fn sort_dedup<T: Ord>(items: &mut Vec<T>) {
    items.sort();
    items.dedup();
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventContext {
    pub entities: Vec<ContextEntity>,
    pub relationships: Vec<RelationshipAssertion>,
    pub environments: Vec<String>,
    pub activities: Vec<String>,
    pub goals: Vec<String>,
    pub source_channel: SourceChannel,
    pub tags: Vec<String>,
}

impl Default for EventContext {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
            relationships: Vec::new(),
            environments: Vec::new(),
            activities: Vec::new(),
            goals: Vec::new(),
            source_channel: SourceChannel::parse("system").expect("static source channel"),
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEntity {
    pub entity_id: EntityId,
    pub entity_type: EntityType,
    #[serde(deserialize_with = "required_option")]
    pub display_label: Option<BoundedLabel>,
    #[serde(deserialize_with = "required_option")]
    pub role_in_event: Option<BoundedRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipAssertion {
    pub subject_id: EntityId,
    pub predicate: RelationshipPredicate,
    pub object_id: EntityId,
    pub direction: RelationshipDirection,
    #[serde(deserialize_with = "required_option")]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "required_option")]
    pub valid_to: Option<DateTime<Utc>>,
    pub evidence: Vec<EvidenceRef>,
    pub governance: Governance,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RelationshipDirection {
    Directed,
    Bidirectional,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_type: EvidenceType,
    pub source_id: Identifier,
    #[serde(deserialize_with = "required_option")]
    pub digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceType {
    Note,
    Conversation,
    CanvasSession,
    CanvasResponse,
    TwinRecord,
    Decision,
    Feedback,
    Import,
    Event,
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Governance {
    pub review: ReviewState,
    pub authority: AuthorityClass,
    pub sensitivity: Sensitivity,
    pub visibility: Visibility,
    pub allowed_uses: AllowedUses,
}

impl Governance {
    pub fn direct_observation() -> Self {
        Self {
            review: ReviewState::NotApplicable,
            authority: AuthorityClass::EvidenceObservation,
            sensitivity: Sensitivity::Standard,
            visibility: Visibility::SyncedVault,
            allowed_uses: AllowedUses {
                recall: true,
                twin_advisor: true,
                twin_simulation: false,
                export: true,
                training: false,
                sync: true,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewState {
    NotApplicable,
    Pending,
    Accepted,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "class", deny_unknown_fields)]
pub enum AuthorityClass {
    EvidenceObservation,
    ReviewedMemory,
    CanonicalUserRule,
    DeterministicallyVerified { method: VerificationMethod },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VerificationMethod {
    HumanReview,
    SourceChecksum,
    SignedImport,
    RecordedOutcomeMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Sensitivity {
    Standard,
    Sensitive,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Visibility {
    LocalOnly,
    SyncedVault,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedUses {
    pub recall: bool,
    pub twin_advisor: bool,
    pub twin_simulation: bool,
    pub export: bool,
    pub training: bool,
    pub sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TwinEventPayload {
    ObservationRecorded(ObservationRecorded),
    NoteChanged(NoteChanged),
    ConversationTurnRecorded(ConversationTurnRecorded),
    CanvasResponseRecorded(CanvasResponseRecorded),
    MemoryProposed(MemoryProposed),
    MemoryReviewed(MemoryReviewed),
    DecisionRecorded(DecisionRecorded),
    DecisionOutcomeRecorded(DecisionOutcomeRecorded),
    FeedbackRecorded(FeedbackRecorded),
    RelationshipContextObserved(RelationshipContextObserved),
}

impl TwinEventPayload {
    pub fn event_type(&self) -> TwinEventType {
        match self {
            Self::ObservationRecorded(_) => TwinEventType::ObservationRecorded,
            Self::NoteChanged(_) => TwinEventType::NoteChanged,
            Self::ConversationTurnRecorded(_) => TwinEventType::ConversationTurnRecorded,
            Self::CanvasResponseRecorded(_) => TwinEventType::CanvasResponseRecorded,
            Self::MemoryProposed(_) => TwinEventType::MemoryProposed,
            Self::MemoryReviewed(_) => TwinEventType::MemoryReviewed,
            Self::DecisionRecorded(_) => TwinEventType::DecisionRecorded,
            Self::DecisionOutcomeRecorded(_) => TwinEventType::DecisionOutcomeRecorded,
            Self::FeedbackRecorded(_) => TwinEventType::FeedbackRecorded,
            Self::RelationshipContextObserved(_) => TwinEventType::RelationshipContextObserved,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::ObservationRecorded(value) => {
                validate_max_len(&value.claims, MAX_CLAIMS, "observation.claims")
            }
            Self::DecisionRecorded(value) => {
                validate_max_len(&value.options, MAX_DECISION_OPTIONS, "decision.options")
            }
            _ => Ok(()),
        }
    }

    fn normalize(&mut self) {
        if let Self::ObservationRecorded(value) = self {
            sort_dedup(&mut value.claims);
        }
    }
}

fn validate_decimal(value: &str) -> Result<(), String> {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        || parts.next().is_some()
    {
        return Err("cost must be unsigned decimal text".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimAssertion {
    pub subject_id: EntityId,
    pub predicate: ClaimPredicate,
    pub object: ClaimObject,
    pub polarity: ClaimPolarity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimPolarity {
    Affirmed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRecorded {
    pub observation_id: Identifier,
    pub claims: Vec<ClaimAssertion>,
    #[serde(deserialize_with = "required_option")]
    pub summary: Option<BoundedSummary>,
    #[serde(deserialize_with = "required_option")]
    pub content_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteChanged {
    pub note_id: Identifier,
    pub change: NoteChangeKind,
    #[serde(deserialize_with = "required_option")]
    pub content_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteChangeKind {
    Created,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnRecorded {
    pub conversation_id: Identifier,
    pub turn_id: Identifier,
    pub role: BoundedRole,
    pub content: BoundedContent,
    #[serde(deserialize_with = "required_option")]
    pub model_id: Option<ModelId>,
    #[serde(deserialize_with = "required_option")]
    pub provenance: Option<ProvenanceLabel>,
    #[serde(deserialize_with = "required_option")]
    pub content_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasResponseRecorded {
    pub session_id: Identifier,
    pub tile_id: Identifier,
    pub response_id: Identifier,
    pub prompt: BoundedContent,
    pub response: BoundedContent,
    pub model_id: ModelId,
    #[serde(deserialize_with = "required_option")]
    pub provider: Option<Identifier>,
    #[serde(deserialize_with = "required_option")]
    pub provenance: Option<ProvenanceLabel>,
    #[serde(deserialize_with = "required_option")]
    pub tokens_used: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    pub cost_usd_decimal: Option<DecimalCost>,
    #[serde(deserialize_with = "required_option")]
    pub prompt_digest: Option<ContentDigest>,
    #[serde(deserialize_with = "required_option")]
    pub response_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposed {
    pub memory_id: Identifier,
    pub claim: ClaimAssertion,
    #[serde(deserialize_with = "required_option")]
    pub summary: Option<BoundedSummary>,
    pub proposal_source: ProvenanceLabel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryReviewed {
    pub memory_id: Identifier,
    pub decision: MemoryReviewDecision,
    #[serde(deserialize_with = "required_option")]
    pub reviewed_claim: Option<ClaimAssertion>,
    #[serde(deserialize_with = "required_option")]
    pub rationale: Option<BoundedContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryReviewDecision {
    Accept,
    Reject,
    Supersede,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRecorded {
    pub decision_id: Identifier,
    pub decision: BoundedContent,
    pub options: Vec<BoundedContent>,
    #[serde(deserialize_with = "required_option")]
    pub stakes: Option<BoundedContent>,
    #[serde(deserialize_with = "required_option")]
    pub initial_leaning: Option<BoundedContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionOutcomeRecorded {
    pub decision_id: Identifier,
    pub outcome: BoundedContent,
    #[serde(deserialize_with = "required_option")]
    pub chosen_option: Option<BoundedContent>,
    #[serde(deserialize_with = "required_option")]
    pub regret_score: Option<u8>,
    #[serde(deserialize_with = "required_option")]
    pub lesson: Option<BoundedContent>,
    #[serde(deserialize_with = "required_option")]
    pub missed_something: Option<BoundedContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackRecorded {
    pub feedback_id: Identifier,
    pub target_id: Identifier,
    pub kind: BoundedRole,
    pub content: BoundedContent,
    #[serde(deserialize_with = "required_option")]
    pub rationale: Option<BoundedContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipContextObserved {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_round_trips_without_collapsing_orthogonal_fields() {
        let governance = Governance {
            review: ReviewState::Accepted,
            authority: AuthorityClass::DeterministicallyVerified {
                method: VerificationMethod::SignedImport,
            },
            sensitivity: Sensitivity::Restricted,
            visibility: Visibility::LocalOnly,
            allowed_uses: AllowedUses {
                recall: true,
                twin_advisor: false,
                twin_simulation: false,
                export: false,
                training: false,
                sync: false,
            },
        };
        let encoded = serde_json::to_string(&governance).unwrap();
        let decoded: Governance = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, governance);
    }

    #[test]
    fn relationship_observation_requires_relationship_context() {
        let event = crate::services::twin_events::test_support::event_for_payload(
            TwinEventPayload::RelationshipContextObserved(RelationshipContextObserved {}),
        );
        assert!(event.validate().unwrap_err().contains("relationship"));
    }

    #[test]
    fn every_approved_payload_has_a_dedicated_matching_event_type() {
        let payloads = crate::services::twin_events::test_support::all_payloads();
        assert_eq!(
            payloads
                .iter()
                .map(TwinEventPayload::event_type)
                .collect::<Vec<_>>(),
            vec![
                TwinEventType::ObservationRecorded,
                TwinEventType::NoteChanged,
                TwinEventType::ConversationTurnRecorded,
                TwinEventType::CanvasResponseRecorded,
                TwinEventType::MemoryProposed,
                TwinEventType::MemoryReviewed,
                TwinEventType::DecisionRecorded,
                TwinEventType::DecisionOutcomeRecorded,
                TwinEventType::FeedbackRecorded,
                TwinEventType::RelationshipContextObserved,
            ]
        );
        for payload in payloads {
            let mut event = crate::services::twin_events::test_support::event_for_payload(payload);
            event.event_type = TwinEventType::ObservationRecorded;
            if event.payload.event_type() != TwinEventType::ObservationRecorded {
                assert!(event.validate().is_err());
            }
        }
    }

    #[test]
    fn missing_required_envelope_keys_are_rejected() {
        let event = crate::services::twin_events::test_support::valid_event(1, Vec::new());
        let mut value = serde_json::to_value(event).unwrap();
        for key in [
            "recorded_at",
            "observed_at",
            "occurred_at",
            "valid_from",
            "valid_to",
            "supersedes",
            "reinforces",
            "context",
            "evidence",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<TwinEvent>(missing).is_err(),
                "{key}"
            );
        }
        assert!(value.get_mut("event_id").is_some());
    }

    #[test]
    fn source_payloads_preserve_bounded_meaningful_content_without_raw_context_or_secrets() {
        let payloads = crate::services::twin_events::test_support::all_payloads();
        let json = serde_json::to_string(&payloads).unwrap();
        assert!(json.contains("prompt text"));
        assert!(json.contains("response text"));
        assert!(json.contains("conversation content"));
        assert!(json.contains("feedback content"));
        assert!(!json.contains("api_key"));
        assert!(!json.contains("unselected_context"));
        let multiline = BoundedContent::parse("line one\nline two\titem\r\n").unwrap();
        let round_trip: BoundedContent =
            serde_json::from_str(&serde_json::to_string(&multiline).unwrap()).unwrap();
        assert_eq!(round_trip, multiline);
        assert!(BoundedContent::parse("unsafe\0content").is_err());
        assert!(Identifier::parse("../unsafe").is_err());
        assert!(EventId::parse("ABC").is_err());
    }

    #[test]
    fn model_ids_accept_provider_paths_but_reject_unsafe_path_material() {
        let valid = ModelId::parse("anthropic/claude-3.5-haiku").unwrap();
        assert_eq!(valid.as_str(), "anthropic/claude-3.5-haiku");
        assert!(ModelId::parse("openrouter:anthropic/claude-3.5-haiku").is_ok());
        for invalid in [
            "",
            "anthropic model",
            "anthropic\\model",
            "anthropic//model",
            ":anthropic/model",
            "anthropic:/model",
            "anthropic::model",
            "anthropic/../model",
            "anthropic/./model",
            "anthropic/model\0",
        ] {
            assert!(ModelId::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(ModelId::parse("a".repeat(257)).is_err());
    }

    #[test]
    fn unknown_json_is_rejected_at_envelope_context_governance_and_payload_boundaries() {
        let event = crate::services::twin_events::test_support::valid_event(1, Vec::new());
        let baseline = serde_json::to_value(&event).unwrap();

        let mut top_level = baseline.clone();
        top_level["api_key"] = serde_json::json!("must-not-pass");
        assert!(serde_json::from_value::<TwinEvent>(top_level).is_err());

        let mut context = baseline.clone();
        context["context"]["unselected_context"] = serde_json::json!({"raw": true});
        assert!(serde_json::from_value::<TwinEvent>(context).is_err());

        let mut governance = baseline.clone();
        governance["governance"]["api_key"] = serde_json::json!("must-not-pass");
        assert!(serde_json::from_value::<TwinEvent>(governance).is_err());

        let mut payload_wrapper = baseline.clone();
        payload_wrapper["payload"]["api_key"] = serde_json::json!("must-not-pass");
        assert!(serde_json::from_value::<TwinEvent>(payload_wrapper).is_err());

        let mut payload_data = baseline;
        payload_data["payload"]["data"]["unselected_context"] = serde_json::json!({"raw": true});
        assert!(serde_json::from_value::<TwinEvent>(payload_data).is_err());

        let empty_payload = crate::services::twin_events::test_support::event_for_payload(
            TwinEventPayload::RelationshipContextObserved(RelationshipContextObserved {}),
        );
        let mut empty_payload = serde_json::to_value(empty_payload).unwrap();
        empty_payload["payload"]["data"]["api_key"] = serde_json::json!("must-not-pass");
        assert!(serde_json::from_value::<TwinEvent>(empty_payload).is_err());
    }

    #[test]
    fn bounded_labels_roles_and_decimal_costs_accept_the_limit_and_reject_overflow() {
        assert!(BoundedLabel::parse("a".repeat(MAX_LABEL_BYTES)).is_ok());
        assert!(BoundedLabel::parse("a".repeat(MAX_LABEL_BYTES + 1)).is_err());
        assert!(BoundedLabel::parse("unsafe\nlabel").is_err());
        assert!(BoundedRole::parse("r".repeat(MAX_ROLE_BYTES)).is_ok());
        assert!(BoundedRole::parse("r".repeat(MAX_ROLE_BYTES + 1)).is_err());
        assert!(DecimalCost::parse("0.000125").is_ok());
        assert!(DecimalCost::parse("1".repeat(MAX_COST_DECIMAL_BYTES + 1)).is_err());
        assert!(DecimalCost::parse("1\n0").is_err());
    }

    #[test]
    fn event_lists_accept_their_limit_and_reject_one_over() {
        let ids = (0..=MAX_CAUSAL_PARENTS)
            .map(|index| EventId::parse(format!("{index:064x}")).unwrap())
            .collect::<Vec<_>>();
        let mut event = crate::services::twin_events::test_support::valid_event(1, Vec::new());
        event.causal_parents = ids[..MAX_CAUSAL_PARENTS].to_vec();
        assert!(event.validate().is_ok());
        event.causal_parents = ids;
        assert!(event.validate().is_err());

        let evidence = (0..=MAX_EVIDENCE_REFS)
            .map(|index| EvidenceRef {
                evidence_type: EvidenceType::Note,
                source_id: Identifier::parse(format!("evidence-{index}")).unwrap(),
                digest: None,
            })
            .collect::<Vec<_>>();
        let mut event = crate::services::twin_events::test_support::valid_event(1, Vec::new());
        event.evidence = evidence[..MAX_EVIDENCE_REFS].to_vec();
        assert!(event.validate().is_ok());
        event.evidence = evidence;
        assert!(event.validate().is_err());

        let context_values = (0..=MAX_CONTEXT_VALUES)
            .map(|index| format!("context-{index}"))
            .collect::<Vec<_>>();
        let mut event = crate::services::twin_events::test_support::valid_event(1, Vec::new());
        event.context.environments = context_values[..MAX_CONTEXT_VALUES].to_vec();
        assert!(event.validate().is_ok());
        event.context.environments = context_values;
        assert!(event.validate().is_err());

        let claims = (0..=MAX_CLAIMS)
            .map(|index| ClaimAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: ClaimPredicate::parse("prefers").unwrap(),
                object: ClaimObject::parse(format!("claim {index}")).unwrap(),
                polarity: ClaimPolarity::Affirmed,
            })
            .collect::<Vec<_>>();
        let observation = |claims| {
            TwinEventPayload::ObservationRecorded(ObservationRecorded {
                observation_id: Identifier::parse("observation-1").unwrap(),
                claims,
                summary: None,
                content_digest: None,
            })
        };
        let event = crate::services::twin_events::test_support::event_for_payload(observation(
            claims[..MAX_CLAIMS].to_vec(),
        ));
        assert!(event.validate().is_ok());
        let event =
            crate::services::twin_events::test_support::event_for_payload(observation(claims));
        assert!(event.validate().is_err());

        let options = (0..=MAX_DECISION_OPTIONS)
            .map(|index| BoundedContent::parse(format!("option {index}")).unwrap())
            .collect::<Vec<_>>();
        let payload = |options| {
            TwinEventPayload::DecisionRecorded(DecisionRecorded {
                decision_id: Identifier::parse("decision-1").unwrap(),
                decision: BoundedContent::parse("choose").unwrap(),
                options,
                stakes: None,
                initial_leaning: None,
            })
        };
        let event = crate::services::twin_events::test_support::event_for_payload(payload(
            options[..MAX_DECISION_OPTIONS].to_vec(),
        ));
        assert!(event.validate().is_ok());
        let event = crate::services::twin_events::test_support::event_for_payload(payload(options));
        assert!(event.validate().is_err());
    }
}
