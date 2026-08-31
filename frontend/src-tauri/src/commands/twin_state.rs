use crate::models::note::{Note, NoteCreate, NoteStatus, CURRENT_NOTE_SCHEMA_VERSION};
use crate::models::twin_event::{
    AuthorityClass, BoundedContent, BoundedLabel, BoundedRole, ClaimAssertion, ClaimPolarity,
    ContentDigest, ContextEntity, EntityId, EntityType, EventContext, EventId, EvidenceRef,
    EvidenceType, Governance, Identifier, MemoryProposed, MemoryReviewDecision, MemoryReviewed,
    ProvenanceLabel, RelationshipAssertion, RelationshipDirection, RelationshipPredicate,
    ReviewState, Sensitivity, SourceChannel, TwinEvent, TwinEventPayload,
    MAX_CONTEXT_RELATIONSHIPS,
};
use crate::models::twin_state::{
    AttentionCandidate, AttentionProfile, AttentionRequest, ProjectedItemKind, ProjectedStateItem,
    ProjectionSnapshot, RelationshipKey, RelationshipVariant, SelectionDestination, SelectionTrace,
    SnapshotId, TemporalStateEntry,
};
use crate::services::twin_events::{
    local_capture_governance, project, rank, standard_capture_governance,
    CompanionObservationInput, MutationError, MutationOrigin, MutationPlan, TwinEventDraft,
};
use crate::AppState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tauri::State;

const TWIN_STATE_COMMAND_SCHEMA_VERSION: u16 = 1;
const MAX_PAGE_LIMIT: u16 = 100;
const MAX_FILTER_VALUES: usize = 64;
const MAX_CONTEXT_VALUE_BYTES: usize = 256;
const MAX_CURSOR_BYTES: usize = 8_192;
const FILTER_DIGEST_DOMAIN: &[u8] = b"grafyn.twin-state-filter.v1";
const CURSOR_VERSION: u16 = 1;
const COMPANION_PERSON_ID_DOMAIN: &[u8] = b"grafyn.companion-person.v1";
const MAX_COMPANION_TITLE_CHARACTERS: usize = 80;
const MAX_COMPANION_ATTACHMENTS: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CompanionCaptureKind {
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CompanionSyncPolicy {
    Inherit,
    LocalOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionCaptureContextInput {
    #[serde(default)]
    pub person: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub relationship: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub activity: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCompanionCaptureRequest {
    pub content: String,
    pub capture_kind: CompanionCaptureKind,
    pub context: CompanionCaptureContextInput,
    #[serde(default)]
    pub attachment_digests: Vec<ContentDigest>,
    pub grafyn_sync: CompanionSyncPolicy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCompanionCaptureResponse {
    pub note: Note,
    pub observation_event_id: EventId,
}

fn companion_horizontal_rule(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    compact.len() >= 3
        && compact
            .chars()
            .next()
            .is_some_and(|marker| matches!(marker, '-' | '*' | '_'))
        && compact
            .chars()
            .all(|character| Some(character) == compact.chars().next())
}

fn strip_companion_block_prefix(mut line: &str) -> &str {
    loop {
        line = line.trim_start();
        let previous = line;
        if let Some(rest) = line.strip_prefix('>') {
            line = rest;
        } else {
            let heading_length = line
                .chars()
                .take_while(|character| *character == '#')
                .count();
            if (1..=6).contains(&heading_length)
                && line[heading_length..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                line = &line[heading_length..];
            } else if ["- ", "+ ", "* "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
                .is_some()
            {
                line = &line[2..];
            } else {
                let digits = line.bytes().take_while(u8::is_ascii_digit).count();
                let suffix = line.get(digits..).unwrap_or_default();
                if digits > 0 && (suffix.starts_with(". ") || suffix.starts_with(") ")) {
                    line = &suffix[2..];
                }
            }
        }
        line = line.trim_start();
        if line.starts_with("[ ] ") || line.starts_with("[x] ") || line.starts_with("[X] ") {
            line = &line[4..];
        }
        if line == previous {
            return line;
        }
    }
}

fn strip_companion_inline_markdown(line: &str) -> String {
    let characters = line.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '\\' if index + 1 < characters.len() => {
                output.push(characters[index + 1]);
                index += 2;
            }
            '!' if characters.get(index + 1) == Some(&'[') => {
                index += 1;
            }
            '[' => {
                if let Some(close_offset) = characters[index + 1..]
                    .iter()
                    .position(|character| *character == ']')
                {
                    let close = index + 1 + close_offset;
                    for character in &characters[index + 1..close] {
                        if !matches!(character, '*' | '_' | '~' | '`') {
                            output.push(*character);
                        }
                    }
                    index = close + 1;
                    if characters.get(index) == Some(&'(') {
                        if let Some(target_end) = characters[index + 1..]
                            .iter()
                            .position(|character| *character == ')')
                        {
                            index += target_end + 2;
                        }
                    }
                } else {
                    index += 1;
                }
            }
            '<' => {
                if let Some(close_offset) = characters[index + 1..]
                    .iter()
                    .position(|character| *character == '>')
                {
                    index += close_offset + 2;
                } else {
                    index += 1;
                }
            }
            '*' | '_' | '~' | '`' => index += 1,
            character => {
                output.push(character);
                index += 1;
            }
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn derive_companion_capture_title(content: &str, captured_at: DateTime<Utc>) -> String {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || companion_horizontal_rule(line) {
            continue;
        }
        let title = strip_companion_inline_markdown(strip_companion_block_prefix(line));
        if !title.is_empty() {
            return title.chars().take(MAX_COMPANION_TITLE_CHARACTERS).collect();
        }
    }
    captured_at.format("Capture %Y-%m-%d %H:%M UTC").to_string()
}

fn normalize_companion_context_value(
    value: Option<String>,
    name: &str,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_CONTEXT_VALUE_BYTES || value.chars().any(char::is_control) {
        return Err(format!(
            "Companion capture {name} must be at most 256 bytes without controls"
        ));
    }
    Ok(Some(value.split_whitespace().collect::<Vec<_>>().join(" ")))
}

fn companion_person_id(label: &str) -> Result<EntityId, String> {
    let normalized = label
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update((COMPANION_PERSON_ID_DOMAIN.len() as u64).to_be_bytes());
    hasher.update(COMPANION_PERSON_ID_DOMAIN);
    hasher.update((normalized.len() as u64).to_be_bytes());
    hasher.update(normalized.as_bytes());
    EntityId::parse(format!("person-{:x}", hasher.finalize()))
}

fn companion_relationship_predicate(value: &str) -> Result<RelationshipPredicate, String> {
    let mut normalized = String::new();
    let mut pending_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else if !normalized.is_empty() {
            pending_separator = true;
        }
    }
    if normalized.is_empty() {
        return Err("Companion capture relationship must contain a letter or number".into());
    }
    RelationshipPredicate::parse(normalized)
}

fn plan_companion_capture(
    request: CreateCompanionCaptureRequest,
    captured_at: DateTime<Utc>,
) -> Result<(NoteCreate, CompanionObservationInput), String> {
    if request.content.trim().is_empty() {
        return Err("Companion capture content cannot be blank".into());
    }
    if request.attachment_digests.len() > MAX_COMPANION_ATTACHMENTS {
        return Err("Companion capture cannot cite more than 63 attachments".into());
    }

    let mut attachment_digests = request.attachment_digests;
    attachment_digests.sort();
    attachment_digests.dedup();
    let person = normalize_companion_context_value(request.context.person, "person")?;
    let role = normalize_companion_context_value(request.context.role, "role")?;
    let relationship =
        normalize_companion_context_value(request.context.relationship, "relationship")?;
    let environment =
        normalize_companion_context_value(request.context.environment, "environment")?;
    let activity = normalize_companion_context_value(request.context.activity, "activity")?;
    let goal = normalize_companion_context_value(request.context.goal, "goal")?;
    if person.is_none() && (role.is_some() || relationship.is_some()) {
        return Err("Companion capture role and relationship require a person".into());
    }

    let governance = match request.grafyn_sync {
        CompanionSyncPolicy::Inherit => standard_capture_governance(),
        CompanionSyncPolicy::LocalOnly => local_capture_governance(Sensitivity::Standard),
    };
    let source_channel = SourceChannel::parse("companion_capture")?;
    let mut context = EventContext {
        source_channel,
        tags: vec!["inbox".into()],
        environments: environment.into_iter().collect(),
        activities: activity.into_iter().collect(),
        goals: goal.into_iter().collect(),
        ..EventContext::default()
    };
    if let Some(person) = person {
        let person_id = companion_person_id(&person)?;
        context.entities.push(ContextEntity {
            entity_id: person_id.clone(),
            entity_type: EntityType::parse("person")?,
            display_label: Some(BoundedLabel::parse(person)?),
            role_in_event: role.map(BoundedRole::parse).transpose()?,
        });
        if let Some(relationship) = relationship {
            context.relationships.push(RelationshipAssertion {
                subject_id: EntityId::parse("owner")?,
                predicate: companion_relationship_predicate(&relationship)?,
                object_id: person_id,
                direction: RelationshipDirection::Directed,
                valid_from: Some(captured_at),
                valid_to: None,
                evidence: Vec::new(),
                governance: governance.clone(),
            });
        }
    }

    let grafyn_sync = match request.grafyn_sync {
        CompanionSyncPolicy::Inherit => "inherit",
        CompanionSyncPolicy::LocalOnly => "local_only",
    };
    let mut properties = HashMap::new();
    properties.insert("capture_kind".into(), serde_json::json!("text"));
    properties.insert(
        "attachment_digests".into(),
        serde_json::json!(attachment_digests
            .iter()
            .map(ContentDigest::as_str)
            .collect::<Vec<_>>()),
    );
    properties.insert("grafyn_sync".into(), serde_json::json!(grafyn_sync));
    let note = NoteCreate {
        title: derive_companion_capture_title(&request.content, captured_at),
        content: request.content,
        relative_path: None,
        aliases: Vec::new(),
        status: NoteStatus::Draft,
        tags: vec!["inbox".into()],
        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: None,
        optimizer_managed: false,
        properties,
    };
    let observation = CompanionObservationInput {
        observed_at: captured_at,
        context,
        attachment_digests,
        governance,
    };
    Ok((note, observation))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinRelationshipFilter {
    pub subject_id: EntityId,
    pub predicate: RelationshipPredicate,
    pub object_id: EntityId,
    pub direction: RelationshipDirection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinStateFilter {
    pub relationships: Vec<TwinRelationshipFilter>,
    pub goals: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinStatePageRequest {
    pub reference_time: DateTime<Utc>,
    pub filter: TwinStateFilter,
    pub cursor: Option<String>,
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinProjectionRequest {
    pub reference_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinRelationshipVariantInput {
    pub relationships: Vec<TwinRelationshipFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinAttentionRankRequest {
    pub reference_time: DateTime<Utc>,
    pub profile: AttentionProfile,
    pub query: String,
    pub relationship_variant: TwinRelationshipVariantInput,
    pub goals: Vec<String>,
    pub destination: SelectionDestination,
    pub filter: TwinStateFilter,
    pub limit: u16,
    #[serde(default)]
    pub candidate_note_ids: Vec<Identifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TwinProposalReviewDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewTwinProposalRequest {
    pub memory_id: Identifier,
    pub decision: TwinProposalReviewDecision,
    pub reviewed_claim: Option<ClaimAssertion>,
    pub rationale: Option<String>,
    pub expected_snapshot_id: SnapshotId,
    pub snapshot_reference_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwinStatePage<T> {
    pub schema_version: u16,
    pub snapshot_id: SnapshotId,
    pub reference_time: DateTime<Utc>,
    pub filter_digest: String,
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

pub type TwinObservationPage = TwinStatePage<ProjectedStateItem>;
pub type TwinProposalPage = TwinStatePage<ProjectedStateItem>;
pub type TwinTimelinePage = TwinStatePage<TemporalStateEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwinAttentionRankResponse {
    pub schema_version: u16,
    pub snapshot_id: SnapshotId,
    pub reference_time: DateTime<Utc>,
    pub filter_digest: String,
    pub trace: SelectionTrace,
    pub note_bindings: Vec<TwinAttentionNoteBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TwinAttentionNoteBinding {
    pub item_id: Identifier,
    pub note_id: Identifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTwinProposalResponse {
    pub schema_version: u16,
    pub memory_id: Identifier,
    pub decision: TwinProposalReviewDecision,
    pub proposal_event_id: EventId,
    pub review_event_id: EventId,
    pub reference_time: DateTime<Utc>,
    pub snapshot: ProjectionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NormalizedFilter {
    relationships: Vec<RelationshipKey>,
    goals: Vec<String>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TwinStateCursorV1 {
    version: u16,
    kind: String,
    snapshot_id: String,
    filter_digest: String,
    last_key: String,
}

fn normalize_context_value(value: &str) -> Result<String, String> {
    if value.len() > MAX_CONTEXT_VALUE_BYTES
        || value.trim().is_empty()
        || value.chars().any(char::is_control)
    {
        return Err(
            "Twin state filters require nonempty values of at most 256 bytes without controls"
                .into(),
        );
    }
    Ok(value
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" "))
}

fn normalize_values(values: &[String], name: &str) -> Result<Vec<String>, String> {
    if values.len() > MAX_FILTER_VALUES {
        return Err(format!("Twin state {name} filters exceed 64 values"));
    }
    let mut normalized = values
        .iter()
        .map(|value| normalize_context_value(value))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_filter(filter: &TwinStateFilter) -> Result<NormalizedFilter, String> {
    if filter.relationships.len() > MAX_FILTER_VALUES {
        return Err("Twin state relationship filters exceed 64 values".into());
    }
    let mut relationships = filter
        .relationships
        .iter()
        .map(|relationship| RelationshipKey {
            subject_id: relationship.subject_id.clone(),
            predicate: relationship.predicate.clone(),
            object_id: relationship.object_id.clone(),
            direction: relationship.direction.clone(),
        })
        .collect::<Vec<_>>();
    relationships.sort();
    relationships.dedup();
    Ok(NormalizedFilter {
        relationships,
        goals: normalize_values(&filter.goals, "goal")?,
        tags: normalize_values(&filter.tags, "tag")?,
    })
}

fn filter_digest(filter: &NormalizedFilter) -> Result<String, String> {
    let encoded = serde_json::to_vec(filter).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update((FILTER_DIGEST_DOMAIN.len() as u64).to_be_bytes());
    hasher.update(FILTER_DIGEST_DOMAIN);
    hasher.update((encoded.len() as u64).to_be_bytes());
    hasher.update(encoded);
    Ok(format!("{:x}", hasher.finalize()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err("Twin state cursor is malformed".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or_else(|| "Twin state cursor is malformed".to_string())?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or_else(|| "Twin state cursor is malformed".to_string())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn encode_cursor(cursor: &TwinStateCursorV1) -> Result<String, String> {
    let bytes = serde_json::to_vec(cursor).map_err(|error| error.to_string())?;
    let encoded = encode_hex(&bytes);
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err("Twin state cursor exceeds its size bound".into());
    }
    Ok(encoded)
}

fn decode_cursor(value: &str) -> Result<TwinStateCursorV1, String> {
    if value.len() > MAX_CURSOR_BYTES {
        return Err("Twin state cursor exceeds its size bound".into());
    }
    let bytes = decode_hex(value)?;
    serde_json::from_slice(&bytes).map_err(|_| "Twin state cursor is malformed".into())
}

fn validate_limit(limit: u16) -> Result<(), String> {
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err("Twin state limit must be between 1 and 100".into());
    }
    Ok(())
}

fn validate_page_request(request: &TwinStatePageRequest) -> Result<NormalizedFilter, String> {
    validate_limit(request.limit)?;
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > MAX_CURSOR_BYTES)
    {
        return Err("Twin state cursor exceeds its size bound".into());
    }
    normalize_filter(&request.filter)
}

fn page_by_key<T: Clone>(
    kind: &str,
    snapshot_id: &SnapshotId,
    filter_digest: &str,
    items: Vec<T>,
    cursor: Option<&str>,
    limit: u16,
    key: impl Fn(&T) -> String,
) -> Result<(Vec<T>, Option<String>), String> {
    validate_limit(limit)?;
    let start = if let Some(cursor) = cursor {
        let cursor = decode_cursor(cursor)?;
        if cursor.version != CURSOR_VERSION
            || cursor.kind != kind
            || cursor.snapshot_id != snapshot_id.as_str()
            || cursor.filter_digest != filter_digest
        {
            return Err("Twin state cursor is stale or belongs to another query".into());
        }
        items
            .iter()
            .position(|item| key(item) == cursor.last_key)
            .map(|index| index + 1)
            .ok_or_else(|| "Twin state cursor no longer identifies an item".to_string())?
    } else {
        0
    };
    let end = start.saturating_add(usize::from(limit)).min(items.len());
    let page = items[start..end].to_vec();
    let next_cursor = if end < items.len() {
        let last_key = page
            .last()
            .map(&key)
            .ok_or_else(|| "Twin state page could not advance its cursor".to_string())?;
        Some(encode_cursor(&TwinStateCursorV1 {
            version: CURSOR_VERSION,
            kind: kind.to_string(),
            snapshot_id: snapshot_id.to_string(),
            filter_digest: filter_digest.to_string(),
            last_key,
        })?)
    } else {
        None
    };
    Ok((page, next_cursor))
}

fn normalized_item_values(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .filter_map(|value| normalize_context_value(value).ok())
        .collect()
}

fn item_matches_filter(item: &ProjectedStateItem, filter: &NormalizedFilter) -> bool {
    let relationships = item
        .relationship_variant
        .relationships
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let goals = normalized_item_values(&item.goals);
    let tags = normalized_item_values(&item.tags);
    filter
        .relationships
        .iter()
        .all(|relationship| relationships.contains(relationship))
        && filter.goals.iter().all(|goal| goals.contains(goal))
        && filter.tags.iter().all(|tag| tags.contains(tag))
}

fn companion_note_id_from_item(item: &ProjectedStateItem) -> Option<Identifier> {
    if item.kind != ProjectedItemKind::Observation
        || !item.item_id.as_str().starts_with("observation:")
        || !item.item_id.as_str().ends_with(":capture")
        || item.claim.subject_id.as_str() != "owner"
        || item.claim.predicate.as_str() != "recorded_note"
        || item.claim.polarity != ClaimPolarity::Affirmed
    {
        return None;
    }
    Identifier::parse(item.claim.object.as_str()).ok()
}

fn event_matches_filter(event: &TwinEvent, filter: &NormalizedFilter) -> bool {
    let relationships = event
        .context
        .relationships
        .iter()
        .map(RelationshipKey::from)
        .collect::<BTreeSet<_>>();
    let goals = normalized_item_values(&event.context.goals);
    let tags = normalized_item_values(&event.context.tags);
    filter
        .relationships
        .iter()
        .all(|relationship| relationships.contains(relationship))
        && filter.goals.iter().all(|goal| goals.contains(goal))
        && filter.tags.iter().all(|tag| tags.contains(tag))
}

fn build_snapshot(
    state: &AppState,
    reference_time: DateTime<Utc>,
) -> Result<(Vec<TwinEvent>, ProjectionSnapshot), String> {
    let events = state
        .twin_event_store
        .ordered_events()
        .map_err(|error| error.to_string())?;
    let snapshot = project(&events, reference_time).map_err(|error| error.to_string())?;
    Ok((events, snapshot))
}

async fn finish_root_read<T>(
    state: &AppState,
    ticket: crate::commands::RootReadTicket,
    result: Result<T, String>,
) -> Result<T, String> {
    match result {
        Ok(value) => {
            ticket.finish(state).await?;
            Ok(value)
        }
        Err(error) => {
            ticket.finish(state).await?;
            Err(error)
        }
    }
}

async fn list_twin_observations_inner(
    state: &AppState,
    request: TwinStatePageRequest,
) -> Result<TwinObservationPage, String> {
    let filter = validate_page_request(&request)?;
    let digest = filter_digest(&filter)?;
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = (|| {
        let (_, snapshot) = build_snapshot(state, request.reference_time)?;
        let items = snapshot
            .recent_observations
            .iter()
            .filter(|item| item_matches_filter(item, &filter))
            .cloned()
            .collect::<Vec<_>>();
        let (items, next_cursor) = page_by_key(
            "observations",
            &snapshot.snapshot_id,
            &digest,
            items,
            request.cursor.as_deref(),
            request.limit,
            |item| item.item_id.to_string(),
        )?;
        Ok(TwinStatePage {
            schema_version: TWIN_STATE_COMMAND_SCHEMA_VERSION,
            snapshot_id: snapshot.snapshot_id,
            reference_time: snapshot.reference_time,
            filter_digest: digest,
            items,
            next_cursor,
        })
    })();
    finish_root_read(state, ticket, result).await
}

async fn list_twin_proposals_inner(
    state: &AppState,
    request: TwinStatePageRequest,
) -> Result<TwinProposalPage, String> {
    let filter = validate_page_request(&request)?;
    let digest = filter_digest(&filter)?;
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = (|| {
        let (_, snapshot) = build_snapshot(state, request.reference_time)?;
        let items = snapshot
            .pending_proposals
            .iter()
            .filter(|item| item_matches_filter(item, &filter))
            .cloned()
            .collect::<Vec<_>>();
        let (items, next_cursor) = page_by_key(
            "proposals",
            &snapshot.snapshot_id,
            &digest,
            items,
            request.cursor.as_deref(),
            request.limit,
            |item| item.item_id.to_string(),
        )?;
        Ok(TwinStatePage {
            schema_version: TWIN_STATE_COMMAND_SCHEMA_VERSION,
            snapshot_id: snapshot.snapshot_id,
            reference_time: snapshot.reference_time,
            filter_digest: digest,
            items,
            next_cursor,
        })
    })();
    finish_root_read(state, ticket, result).await
}

async fn get_twin_state_projection_inner(
    state: &AppState,
    request: TwinProjectionRequest,
) -> Result<ProjectionSnapshot, String> {
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = build_snapshot(state, request.reference_time).map(|(_, snapshot)| snapshot);
    finish_root_read(state, ticket, result).await
}

async fn rank_twin_attention_inner(
    state: &AppState,
    request: TwinAttentionRankRequest,
) -> Result<TwinAttentionRankResponse, String> {
    validate_limit(request.limit)?;
    if request.candidate_note_ids.len() > usize::from(MAX_PAGE_LIMIT) {
        return Err("Twin attention candidate note IDs exceed 100 values".into());
    }
    let requested_note_ids = request
        .candidate_note_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if request.relationship_variant.relationships.len() > MAX_FILTER_VALUES {
        return Err("Twin attention relationship context exceeds 64 values".into());
    }
    let goals = normalize_values(&request.goals, "goal")?;
    let filter = normalize_filter(&request.filter)?;
    let digest = filter_digest(&filter)?;
    let relationship_variant = RelationshipVariant::new(
        request
            .relationship_variant
            .relationships
            .iter()
            .map(|relationship| RelationshipKey {
                subject_id: relationship.subject_id.clone(),
                predicate: relationship.predicate.clone(),
                object_id: relationship.object_id.clone(),
                direction: relationship.direction.clone(),
            })
            .collect(),
    );
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = (|| {
        let (_, snapshot) = build_snapshot(state, request.reference_time)?;
        let items = snapshot
            .reviewed_memories
            .iter()
            .chain(&snapshot.pending_proposals)
            .chain(&snapshot.recent_observations)
            .filter(|item| item_matches_filter(item, &filter))
            .cloned()
            .collect::<Vec<_>>();
        let mut note_ids_by_item = BTreeMap::new();
        let mut candidates = Vec::new();
        for item in items {
            let note_id = companion_note_id_from_item(&item);
            if !requested_note_ids.is_empty()
                && note_id
                    .as_ref()
                    .is_none_or(|note_id| !requested_note_ids.contains(note_id))
            {
                continue;
            }
            if let Some(note_id) = note_id {
                note_ids_by_item.insert(item.item_id.clone(), note_id);
            }
            candidates.push(AttentionCandidate { item });
        }
        candidates.sort_by(|left, right| left.item.item_id.cmp(&right.item.item_id));
        candidates.dedup_by(|left, right| left.item.item_id == right.item.item_id);
        let trace = rank(
            snapshot.snapshot_id.clone(),
            &candidates,
            request.profile,
            &AttentionRequest {
                query: request.query,
                relationship_variant,
                goals,
                reference_time: request.reference_time,
                destination: request.destination,
                limit: request.limit,
            },
        )
        .map_err(|error| error.to_string())?;
        let note_bindings = trace
            .selected
            .iter()
            .filter_map(|selection| {
                note_ids_by_item
                    .get(&selection.item_id)
                    .cloned()
                    .map(|note_id| TwinAttentionNoteBinding {
                        item_id: selection.item_id.clone(),
                        note_id,
                    })
            })
            .collect();
        Ok(TwinAttentionRankResponse {
            schema_version: TWIN_STATE_COMMAND_SCHEMA_VERSION,
            snapshot_id: snapshot.snapshot_id,
            reference_time: snapshot.reference_time,
            filter_digest: digest,
            trace,
            note_bindings,
        })
    })();
    finish_root_read(state, ticket, result).await
}

async fn get_twin_event_timeline_inner(
    state: &AppState,
    request: TwinStatePageRequest,
) -> Result<TwinTimelinePage, String> {
    let filter = validate_page_request(&request)?;
    let digest = filter_digest(&filter)?;
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = (|| {
        let (events, snapshot) = build_snapshot(state, request.reference_time)?;
        let projected = snapshot
            .reviewed_memories
            .iter()
            .chain(&snapshot.pending_proposals)
            .chain(&snapshot.recent_observations)
            .map(|item| (item.item_id.clone(), item))
            .collect::<BTreeMap<_, _>>();
        let events = events
            .iter()
            .map(|event| (event.event_id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        let items = snapshot
            .timeline
            .iter()
            .filter(|entry| {
                projected
                    .get(&entry.item_id)
                    .is_some_and(|item| item_matches_filter(item, &filter))
                    || events
                        .get(&entry.source_event_id)
                        .is_some_and(|event| event_matches_filter(event, &filter))
            })
            .cloned()
            .collect::<Vec<_>>();
        let (items, next_cursor) = page_by_key(
            "timeline",
            &snapshot.snapshot_id,
            &digest,
            items,
            request.cursor.as_deref(),
            request.limit,
            |entry| serde_json::to_string(entry).expect("timeline DTO serialization is infallible"),
        )?;
        Ok(TwinStatePage {
            schema_version: TWIN_STATE_COMMAND_SCHEMA_VERSION,
            snapshot_id: snapshot.snapshot_id,
            reference_time: snapshot.reference_time,
            filter_digest: digest,
            items,
            next_cursor,
        })
    })();
    finish_root_read(state, ticket, result).await
}

fn review_event_context(
    item: &ProjectedStateItem,
    relationships: &[RelationshipAssertion],
    source_channel: &SourceChannel,
) -> EventContext {
    EventContext {
        relationships: relationships.to_vec(),
        goals: item.goals.clone(),
        source_channel: source_channel.clone(),
        tags: item.tags.clone(),
        ..EventContext::default()
    }
}

fn relationship_key(relationship: &RelationshipAssertion) -> RelationshipKey {
    RelationshipKey {
        subject_id: relationship.subject_id.clone(),
        predicate: relationship.predicate.clone(),
        object_id: relationship.object_id.clone(),
        direction: relationship.direction.clone(),
    }
}

fn review_relationships(
    item: &ProjectedStateItem,
    events: &[TwinEvent],
    reference_time: DateTime<Utc>,
) -> Result<Vec<RelationshipAssertion>, String> {
    if item.relationship_variant.relationships.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(proposal_id) = item.proposal_event_id.as_ref() {
        let proposal = events
            .iter()
            .find(|event| &event.event_id == proposal_id)
            .ok_or_else(|| "Twin proposal context source is missing".to_string())?;
        let relationships = proposal.context.relationships.clone();
        let variant = RelationshipVariant::new(
            relationships
                .iter()
                .map(relationship_key)
                .collect::<Vec<_>>(),
        );
        if variant != item.relationship_variant {
            return Err("Twin proposal relationship context changed since projection".into());
        }
        return Ok(relationships);
    }

    let eligible = events
        .iter()
        .filter(|event| event.recorded_at <= reference_time)
        .collect::<Vec<_>>();
    let superseded = eligible
        .iter()
        .flat_map(|event| event.supersedes.iter().cloned())
        .collect::<BTreeSet<_>>();
    let keys = item
        .relationship_variant
        .relationships
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let sources = eligible
        .into_iter()
        .filter(|event| {
            if superseded.contains(&event.event_id)
                || event.governance.sensitivity == Sensitivity::Restricted
                || matches!(
                    event.governance.review,
                    ReviewState::Rejected | ReviewState::Superseded
                )
                || event.valid_from.is_some_and(|from| reference_time < from)
                || event.valid_to.is_some_and(|to| reference_time > to)
            {
                return false;
            }
            let variant = RelationshipVariant::new(
                event
                    .context
                    .relationships
                    .iter()
                    .map(relationship_key)
                    .collect::<Vec<_>>(),
            );
            let TwinEventPayload::ObservationRecorded(observation) = &event.payload else {
                return false;
            };
            variant == item.relationship_variant && observation.claims.contains(&item.claim)
        })
        .collect::<Vec<_>>();
    if u16::try_from(sources.len()).unwrap_or(u16::MAX) != item.support_count {
        return Err("Twin proposal relationship support changed since projection".into());
    }
    let relationships = sources
        .iter()
        .flat_map(|event| &event.context.relationships)
        .filter(|relationship| keys.contains(&relationship_key(relationship)))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if relationships.len() > MAX_CONTEXT_RELATIONSHIPS {
        return Err("Twin proposal relationship provenance exceeds its event bound".into());
    }
    let variant = RelationshipVariant::new(
        relationships
            .iter()
            .map(relationship_key)
            .collect::<Vec<_>>(),
    );
    if variant != item.relationship_variant {
        return Err("Twin proposal relationship provenance is missing".into());
    }
    Ok(relationships)
}

fn review_evidence(item: &ProjectedStateItem) -> Result<Vec<EvidenceRef>, String> {
    item.evidence_event_ids
        .iter()
        .map(|event_id| {
            Ok(EvidenceRef {
                evidence_type: EvidenceType::Event,
                source_id: Identifier::parse(event_id.as_str())?,
                digest: None,
            })
        })
        .collect()
}

fn review_draft(
    item: &ProjectedStateItem,
    relationships: &[RelationshipAssertion],
    governance: Governance,
    source_channel: &SourceChannel,
    recorded_at: DateTime<Utc>,
    causal_parents: Vec<EventId>,
    payload: TwinEventPayload,
) -> Result<TwinEventDraft, String> {
    Ok(TwinEventDraft {
        actor_id: None,
        causal_parents,
        recorded_at,
        observed_at: recorded_at,
        occurred_at: None,
        valid_from: item.valid_from,
        valid_to: item.valid_to,
        supersedes: Vec::new(),
        reinforces: item.evidence_event_ids.clone(),
        context: review_event_context(item, relationships, source_channel),
        evidence: review_evidence(item)?,
        governance,
        payload,
    })
}

fn build_review_drafts(
    item: &ProjectedStateItem,
    events: &[TwinEvent],
    decision: TwinProposalReviewDecision,
    reviewed_claim: Option<ClaimAssertion>,
    rationale: Option<BoundedContent>,
    recorded_at: DateTime<Utc>,
) -> Result<Vec<TwinEventDraft>, String> {
    if !item.review_event_ids.is_empty() {
        return Err(
            "Twin proposal has already entered review; refresh before reviewing again".into(),
        );
    }
    if decision == TwinProposalReviewDecision::Reject && reviewed_claim.is_some() {
        return Err("reviewedClaim is only valid when accepting a Twin proposal".into());
    }
    let source_channel = SourceChannel::parse("twin_review")?;
    let relationships = review_relationships(item, events, recorded_at)?;
    let mut drafts = Vec::with_capacity(if item.proposal_event_id.is_some() {
        1
    } else {
        2
    });
    if item.proposal_event_id.is_none() {
        drafts.push(review_draft(
            item,
            &relationships,
            item.governance.clone(),
            &source_channel,
            recorded_at,
            Vec::new(),
            TwinEventPayload::MemoryProposed(MemoryProposed {
                memory_id: item.item_id.clone(),
                claim: item.claim.clone(),
                summary: item.summary.clone(),
                proposal_source: ProvenanceLabel::parse("deterministic_projection_v1")?,
            }),
        )?);
    }
    let mut review_governance = item.governance.clone();
    let review_decision = match decision {
        TwinProposalReviewDecision::Accept => {
            review_governance.review = ReviewState::Accepted;
            review_governance.authority = AuthorityClass::ReviewedMemory;
            MemoryReviewDecision::Accept
        }
        TwinProposalReviewDecision::Reject => {
            review_governance.review = ReviewState::Rejected;
            // Rejection is still a reviewed-memory judgment. Projection rejects
            // MemoryReviewed events that retain evidence-only authority.
            review_governance.authority = AuthorityClass::ReviewedMemory;
            MemoryReviewDecision::Reject
        }
    };
    drafts.push(review_draft(
        item,
        &relationships,
        review_governance,
        &source_channel,
        recorded_at,
        item.proposal_event_id.iter().cloned().collect(),
        TwinEventPayload::MemoryReviewed(MemoryReviewed {
            memory_id: item.item_id.clone(),
            decision: review_decision,
            reviewed_claim,
            rationale,
        }),
    )?);
    Ok(drafts)
}

fn snapshot_covers_all_events(snapshot: &ProjectionSnapshot, events: &[TwinEvent]) -> bool {
    snapshot
        .applied_event_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        == events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>()
}

async fn review_twin_proposal_inner(
    state: &AppState,
    request: ReviewTwinProposalRequest,
) -> Result<ReviewTwinProposalResponse, String> {
    if request.decision == TwinProposalReviewDecision::Reject && request.reviewed_claim.is_some() {
        return Err("reviewedClaim is only valid when accepting a Twin proposal".into());
    }
    let rationale = request
        .rationale
        .as_deref()
        .map(BoundedContent::parse)
        .transpose()?;
    if request.snapshot_reference_time > Utc::now() {
        return Err("Twin proposal snapshot reference time cannot be in the future".into());
    }
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let expected_authority = ticket.authority().clone();
    let expected_snapshot_id = request.expected_snapshot_id.clone();
    let snapshot_reference_time = request.snapshot_reference_time;
    let memory_id = request.memory_id.clone();
    let decision = request.decision;
    let reviewed_claim = request.reviewed_claim.clone();
    let source_channel = SourceChannel::parse("twin_review")?;
    let mut planned_item = None;
    let mut planned_review_time = None;
    let result = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
        .commit_planned(MutationOrigin::Local, &mut || {
            let events = state
                .twin_event_store
                .ordered_events()
                .map_err(MutationError::from)?;
            if snapshot_reference_time > Utc::now() {
                return Err(MutationError::Invalid(
                    "Twin proposal snapshot reference time cannot be in the future".into(),
                ));
            }
            let snapshot = project(&events, snapshot_reference_time)
                .map_err(|error| MutationError::Invalid(error.to_string()))?;
            if snapshot.snapshot_id != expected_snapshot_id
                || !snapshot_covers_all_events(&snapshot, &events)
            {
                return Err(MutationError::Invalid(
                    "Twin proposal snapshot is stale; refresh before reviewing".into(),
                ));
            }
            let viewed_item = snapshot
                .pending_proposals
                .iter()
                .find(|item| item.item_id == memory_id)
                .cloned()
                .ok_or_else(|| {
                    MutationError::Invalid(
                        "Twin proposal is no longer pending; refresh before reviewing".into(),
                    )
                })?;
            let review_time = Utc::now();
            let current_snapshot = project(&events, review_time)
                .map_err(|error| MutationError::Invalid(error.to_string()))?;
            if !snapshot_covers_all_events(&current_snapshot, &events) {
                return Err(MutationError::Invalid(
                    "Twin proposal state changed while preparing review".into(),
                ));
            }
            let item = current_snapshot
                .pending_proposals
                .iter()
                .find(|item| item.item_id == memory_id)
                .cloned()
                .ok_or_else(|| {
                    MutationError::Invalid(
                        "Twin proposal is no longer pending; refresh before reviewing".into(),
                    )
                })?;
            if item != viewed_item {
                return Err(MutationError::Invalid(
                    "Twin proposal changed since it was viewed; refresh before reviewing".into(),
                ));
            }
            let drafts = build_review_drafts(
                &item,
                &events,
                decision,
                reviewed_claim.clone(),
                rationale.clone(),
                review_time,
            )
            .map_err(MutationError::Invalid)?;
            let plan = MutationPlan::new(
                item.causal_stream,
                source_channel.clone(),
                Vec::new(),
                drafts,
            )
            .expecting_authority(expected_authority.clone());
            planned_item = Some(item);
            planned_review_time = Some(review_time);
            Ok(Some(plan))
        });
    let commit = match result {
        Ok(commit) => commit,
        Err(error) => {
            let Some(commit) = error.authority_advanced_commit() else {
                ticket.finish(state).await?;
                return Err(error.to_string());
            };
            let target_aborted = error.authority_advanced_target_aborted();
            drop(ticket);
            let repair = crate::commands::repair_after_authority_mutation(
                state,
                &commit,
                "Twin proposal review",
            )
            .await;
            if target_aborted {
                return Err(match repair {
                    crate::commands::PostAuthorityRepair::Ready(_) => {
                        "Twin proposal review was not applied after authority advanced; refresh before deciding whether to retry".into()
                    }
                    crate::commands::PostAuthorityRepair::NotRequired
                    | crate::commands::PostAuthorityRepair::Unavailable(_) => {
                        "Twin proposal review was not applied and recovery is pending; do not retry".into()
                    }
                });
            }
            return Err(match repair {
                crate::commands::PostAuthorityRepair::Ready(_) => {
                    "Twin proposal review committed and was recovered; do not retry".into()
                }
                crate::commands::PostAuthorityRepair::NotRequired
                | crate::commands::PostAuthorityRepair::Unavailable(_) => {
                    "Twin proposal review committed and recovery is pending; do not retry".into()
                }
            });
        }
    };
    let item = planned_item.ok_or_else(|| {
        "Twin proposal review committed without its planned item; do not retry".to_string()
    })?;
    let review_time = planned_review_time.ok_or_else(|| {
        "Twin proposal review committed without its review time; do not retry".to_string()
    })?;
    let proposal_event_id = item.proposal_event_id.clone().or_else(|| {
        commit.events.iter().find_map(|event| match &event.payload {
            TwinEventPayload::MemoryProposed(proposal)
                if proposal.memory_id == request.memory_id =>
            {
                Some(event.event_id.clone())
            }
            _ => None,
        })
    });
    let review_event_id = commit.events.iter().find_map(|event| match &event.payload {
        TwinEventPayload::MemoryReviewed(review) if review.memory_id == request.memory_id => {
            Some(event.event_id.clone())
        }
        _ => None,
    });
    if commit.authority_token.is_some() {
        drop(ticket);
        match crate::commands::repair_after_authority_mutation(
            state,
            &commit,
            "Twin proposal review",
        )
        .await
        {
            crate::commands::PostAuthorityRepair::Ready(_) => {}
            crate::commands::PostAuthorityRepair::NotRequired
            | crate::commands::PostAuthorityRepair::Unavailable(_) => {
                return Err(
                    "Twin proposal review committed and recovery is pending; do not retry".into(),
                )
            }
        }
    } else {
        ticket.finish(state).await?;
    }
    let response_ticket = crate::commands::acquire_root_epoch(state)
        .await
        .map_err(|error| {
            format!("Twin proposal review committed but response recovery failed: {error}; do not retry")
        })?;
    let current_snapshot = finish_root_read(
        state,
        response_ticket,
        build_snapshot(state, review_time).map(|(_, snapshot)| snapshot),
    )
    .await
    .map_err(|error| {
        format!(
            "Twin proposal review committed but its response snapshot failed: {error}; do not retry"
        )
    })?;
    Ok(ReviewTwinProposalResponse {
        schema_version: TWIN_STATE_COMMAND_SCHEMA_VERSION,
        memory_id: request.memory_id,
        decision: request.decision,
        proposal_event_id: proposal_event_id.ok_or_else(|| {
            "Twin proposal review committed without a proposal event identity; do not retry"
                .to_string()
        })?,
        review_event_id: review_event_id.ok_or_else(|| {
            "Twin proposal review committed without a review event identity; do not retry"
                .to_string()
        })?,
        reference_time: review_time,
        snapshot: current_snapshot,
    })
}

pub(crate) async fn create_companion_capture_inner(
    state: &AppState,
    request: CreateCompanionCaptureRequest,
    captured_at: DateTime<Utc>,
) -> Result<CreateCompanionCaptureResponse, String> {
    create_companion_capture_inner_with_post_commit_checkpoint(
        state,
        request,
        captured_at,
        std::future::ready(()),
    )
    .await
}

async fn create_companion_capture_inner_with_post_commit_checkpoint(
    state: &AppState,
    request: CreateCompanionCaptureRequest,
    captured_at: DateTime<Utc>,
    post_commit_checkpoint: impl std::future::Future<Output = ()>,
) -> Result<CreateCompanionCaptureResponse, String> {
    let (note_create, observation) = plan_companion_capture(request, captured_at)?;
    let root_ticket = crate::commands::acquire_root_epoch(state).await?;
    let expected = root_ticket.authority().clone();
    let mutation = {
        let mut store = state.knowledge_store.write().await;
        store.create_companion_capture_expecting_authority(
            note_create,
            observation,
            expected.clone(),
        )
    };
    let completed = crate::commands::complete_knowledge_note_mutation(
        state,
        &expected,
        None,
        mutation,
        "companion capture",
    )
    .await?;
    let note = completed.note;
    let observation_id = format!("companion-capture-{}", note.id);
    let observation_event_id = completed.commit.as_ref().and_then(|commit| {
        commit.events.iter().find_map(|event| match &event.payload {
            TwinEventPayload::ObservationRecorded(observation)
                if observation.observation_id.as_str() == observation_id =>
            {
                Some(event.event_id.clone())
            }
            _ => None,
        })
    });
    let mut continuation_authority = completed.continuation_authority.clone();
    let repaired = completed.repaired;
    if let Some(commit) = completed.commit.as_ref() {
        drop(root_ticket);
        if !repaired {
            match crate::commands::repair_after_authority_mutation(
                state,
                commit,
                "companion capture",
            )
            .await
            {
                crate::commands::PostAuthorityRepair::Ready(authority) => {
                    continuation_authority = authority;
                }
                repair => crate::commands::acknowledge_reported_repair(repair),
            }
        }
    } else {
        root_ticket.finish(state).await?;
    }
    post_commit_checkpoint.await;
    let observation_event_id = observation_event_id.ok_or_else(|| {
        "Companion capture committed without its observation identity; do not retry".to_string()
    })?;
    if let Err(error) = crate::commands::enqueue_vault_optimizer_note_at_authority(
        state,
        &continuation_authority,
        &note.id,
        "companion_capture",
    )
    .await
    {
        log::warn!("Companion capture committed but optimizer enqueue failed: {error}");
    }
    Ok(CreateCompanionCaptureResponse {
        note,
        observation_event_id,
    })
}

#[tauri::command]
pub async fn list_twin_observations(
    state: State<'_, AppState>,
    request: TwinStatePageRequest,
) -> Result<TwinObservationPage, String> {
    list_twin_observations_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn list_twin_proposals(
    state: State<'_, AppState>,
    request: TwinStatePageRequest,
) -> Result<TwinProposalPage, String> {
    list_twin_proposals_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn create_companion_capture(
    state: State<'_, AppState>,
    request: CreateCompanionCaptureRequest,
) -> Result<CreateCompanionCaptureResponse, String> {
    create_companion_capture_inner(state.inner(), request, Utc::now()).await
}

#[tauri::command]
pub async fn review_twin_proposal(
    state: State<'_, AppState>,
    request: ReviewTwinProposalRequest,
) -> Result<ReviewTwinProposalResponse, String> {
    review_twin_proposal_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn get_twin_state_projection(
    state: State<'_, AppState>,
    request: TwinProjectionRequest,
) -> Result<ProjectionSnapshot, String> {
    get_twin_state_projection_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn rank_twin_attention(
    state: State<'_, AppState>,
    request: TwinAttentionRankRequest,
) -> Result<TwinAttentionRankResponse, String> {
    rank_twin_attention_inner(state.inner(), request).await
}

#[tauri::command]
pub async fn get_twin_event_timeline(
    state: State<'_, AppState>,
    request: TwinStatePageRequest,
) -> Result<TwinTimelinePage, String> {
    get_twin_event_timeline_inner(state.inner(), request).await
}

#[cfg(test)]
#[path = "twin_state_tests.rs"]
mod tests;
