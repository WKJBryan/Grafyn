use super::proposals::effective_exposure;
use super::{build_proposal_drafts, topological_order, ATTENTION_PROFILE_VERSION};
use crate::models::twin_event::{
    AuthorityClass, CausalStream, ClaimAssertion, ClaimObject, ClaimPolarity, ClaimPredicate,
    EntityId, EventId, EvidenceType, Governance, Identifier, MemoryReviewDecision, ReviewState,
    Sensitivity, TwinEvent, TwinEventPayload,
};
use crate::models::twin_state::{
    ContradictionCluster, ProjectedItemKind, ProjectedStateItem, ProjectionSnapshot, ProposalDraft,
    RelationshipKey, RelationshipVariant, RelationshipVariantState, SnapshotId, StateModelError,
    TemporalStateEntry, TimelineState, EXPECTED_PROJECTION_SCHEMA_VERSION,
    EXPECTED_PROJECTION_VERSION, MAX_PROJECTED_ITEMS,
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECTION_SCHEMA_VERSION: u16 = EXPECTED_PROJECTION_SCHEMA_VERSION;
pub const PROJECTION_VERSION: u16 = EXPECTED_PROJECTION_VERSION;
const SNAPSHOT_ID_DOMAIN: &[u8] = b"grafyn.twin_projection.canonical_json.v1";
const LEGACY_PROJECTION_VERSION: u16 = 2;

#[derive(Debug)]
pub enum ProjectionError {
    EventOrder(super::StoreError),
    Proposal(super::ProposalError),
    DanglingReference(EventId),
    WrongReferenceType(EventId),
    DanglingMemory(String),
    DuplicateMemory(String),
    InvalidReview(String),
    InvalidIdentifier(String),
    TooManyItems(&'static str),
    Serialization(String),
    InvalidSnapshot(StateModelError),
    InvalidEvent(String),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventOrder(error) => write!(formatter, "cannot order projection events: {error}"),
            Self::Proposal(error) => write!(formatter, "cannot derive proposal signals: {error}"),
            Self::DanglingReference(id) => {
                write!(formatter, "projection reference is missing: {id}")
            }
            Self::WrongReferenceType(id) => write!(
                formatter,
                "projection reference has the wrong event type: {id}"
            ),
            Self::DanglingMemory(id) => write!(formatter, "memory review has no proposal: {id}"),
            Self::DuplicateMemory(id) => {
                write!(formatter, "memory ID has multiple proposal events: {id}")
            }
            Self::InvalidReview(id) => {
                write!(formatter, "memory review governance is invalid: {id}")
            }
            Self::InvalidIdentifier(error) => {
                write!(formatter, "derived state identifier is invalid: {error}")
            }
            Self::TooManyItems(name) => {
                write!(formatter, "projection {name} exceeds its item limit")
            }
            Self::Serialization(error) => {
                write!(formatter, "cannot serialize canonical projection: {error}")
            }
            Self::InvalidSnapshot(error) => {
                write!(formatter, "invalid projection snapshot: {error}")
            }
            Self::InvalidEvent(error) => {
                write!(formatter, "invalid projection input event: {error}")
            }
        }
    }
}

impl std::error::Error for ProjectionError {}

fn time_active(event: &TwinEvent, reference_time: DateTime<Utc>) -> bool {
    event.valid_from.is_none_or(|from| reference_time >= from)
        && event.valid_to.is_none_or(|to| reference_time <= to)
}

fn relationship_variant(event: &TwinEvent, _reference_time: DateTime<Utc>) -> RelationshipVariant {
    RelationshipVariant::new(
        event
            .context
            .relationships
            .iter()
            .map(RelationshipKey::from)
            .collect(),
    )
}

fn fallback_confirmation(event: &TwinEvent, reference_time: DateTime<Utc>) -> DateTime<Utc> {
    event
        .occurred_at
        .filter(|occurred| *occurred <= reference_time)
        .unwrap_or(event.observed_at)
}

fn active_superseders(
    events: &[TwinEvent],
    target: &EventId,
    reference_time: DateTime<Utc>,
) -> Vec<EventId> {
    let mut values = events
        .iter()
        .filter(|event| {
            event.supersedes.contains(target)
                && time_active(event, reference_time)
                && !matches!(
                    event.governance.review,
                    ReviewState::Rejected | ReviewState::Superseded
                )
        })
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn active_superseded_event_ids(
    events: &[TwinEvent],
    reference_time: DateTime<Utc>,
) -> BTreeSet<EventId> {
    events
        .iter()
        .filter(|event| {
            time_active(event, reference_time)
                && !matches!(
                    event.governance.review,
                    ReviewState::Rejected | ReviewState::Superseded
                )
        })
        .flat_map(|event| event.supersedes.iter().cloned())
        .collect()
}

fn event_target_type_allowed(event: &TwinEvent) -> bool {
    matches!(
        event.payload,
        TwinEventPayload::ObservationRecorded(_)
            | TwinEventPayload::MemoryProposed(_)
            | TwinEventPayload::MemoryReviewed(_)
    )
}

fn supersession_target_allowed(event: &TwinEvent, target: &TwinEvent) -> bool {
    if event_target_type_allowed(target) {
        return true;
    }
    matches!(
        (&event.payload, &target.payload),
        (
            TwinEventPayload::CanvasResponseRecorded(current),
            TwinEventPayload::CanvasResponseRecorded(previous),
        ) if current.session_id == previous.session_id
            && current.tile_id == previous.tile_id
            && current.response_id == previous.response_id
    )
}

fn validate_references(events: &[TwinEvent]) -> Result<(), ProjectionError> {
    let by_id = events
        .iter()
        .map(|event| (event.event_id.clone(), event))
        .collect::<BTreeMap<_, _>>();
    for event in events {
        for parent in &event.causal_parents {
            if !by_id.contains_key(parent) {
                return Err(ProjectionError::DanglingReference(parent.clone()));
            }
        }
        for target in &event.supersedes {
            let Some(target_event) = by_id.get(target) else {
                return Err(ProjectionError::DanglingReference(target.clone()));
            };
            if !supersession_target_allowed(event, target_event) {
                return Err(ProjectionError::WrongReferenceType(target.clone()));
            }
        }
        for target in &event.reinforces {
            let Some(target_event) = by_id.get(target) else {
                return Err(ProjectionError::DanglingReference(target.clone()));
            };
            if !event_target_type_allowed(target_event) {
                return Err(ProjectionError::WrongReferenceType(target.clone()));
            }
        }
        for evidence in event
            .evidence
            .iter()
            .chain(
                event
                    .context
                    .relationships
                    .iter()
                    .flat_map(|value| &value.evidence),
            )
            .filter(|evidence| evidence.evidence_type == EvidenceType::Event)
        {
            let id = EventId::parse(evidence.source_id.as_str())
                .map_err(|_| ProjectionError::InvalidIdentifier(evidence.source_id.to_string()))?;
            if !by_id.contains_key(&id) {
                return Err(ProjectionError::DanglingReference(id));
            }
        }
    }
    Ok(())
}

fn is_ancestor(
    ancestor: &EventId,
    descendant: &TwinEvent,
    by_id: &BTreeMap<EventId, &TwinEvent>,
) -> bool {
    let mut pending = descendant.causal_parents.clone();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if &id == ancestor {
            return true;
        }
        if visited.insert(id.clone()) {
            if let Some(event) = by_id.get(&id) {
                pending.extend(event.causal_parents.iter().cloned());
            }
        }
    }
    false
}

fn maximal_reviews<'a>(
    reviews: &[&'a TwinEvent],
    by_id: &BTreeMap<EventId, &'a TwinEvent>,
) -> Vec<&'a TwinEvent> {
    let mut result = reviews
        .iter()
        .copied()
        .filter(|candidate| {
            !reviews.iter().any(|other| {
                candidate.event_id != other.event_id
                    && (is_ancestor(&candidate.event_id, other, by_id)
                        || other.supersedes.contains(&candidate.event_id))
            })
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    result
}

type ReviewOutcome = (
    u8,
    ClaimAssertion,
    CausalStream,
    Governance,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn review_outcome(
    proposal_event: &TwinEvent,
    review_event: &TwinEvent,
    by_id: &BTreeMap<EventId, &TwinEvent>,
    reference_time: DateTime<Utc>,
) -> ReviewOutcome {
    let TwinEventPayload::MemoryProposed(proposal) = &proposal_event.payload else {
        unreachable!("review outcome requires proposal")
    };
    let TwinEventPayload::MemoryReviewed(review) = &review_event.payload else {
        unreachable!("review outcome requires review")
    };
    let decision = match review.decision {
        MemoryReviewDecision::Accept => 0,
        MemoryReviewDecision::Reject => 1,
        MemoryReviewDecision::Supersede => 2,
    };
    let (causal_stream, governance) = effective_exposure(
        [
            proposal_event.event_id.clone(),
            review_event.event_id.clone(),
        ],
        by_id,
        reference_time,
        review_event.governance.review.clone(),
        review_event.governance.authority.clone(),
    );
    let valid_to = match (proposal_event.valid_to, review_event.valid_to) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    (
        decision,
        review
            .reviewed_claim
            .clone()
            .unwrap_or_else(|| proposal.claim.clone()),
        causal_stream,
        governance,
        proposal_event.valid_from.max(review_event.valid_from),
        valid_to,
    )
}

fn review_state(event: &TwinEvent) -> Result<TimelineState, ProjectionError> {
    let TwinEventPayload::MemoryReviewed(review) = &event.payload else {
        unreachable!("review_state called with non-review")
    };
    if !matches!(
        event.governance.authority,
        AuthorityClass::ReviewedMemory
            | AuthorityClass::CanonicalUserRule
            | AuthorityClass::DeterministicallyVerified { .. }
    ) {
        return Err(ProjectionError::InvalidReview(event.event_id.to_string()));
    }
    match review.decision {
        MemoryReviewDecision::Accept => {
            if event.governance.review != ReviewState::Accepted {
                return Err(ProjectionError::InvalidReview(event.event_id.to_string()));
            }
            Ok(TimelineState::Accepted)
        }
        MemoryReviewDecision::Reject => {
            if event.governance.review != ReviewState::Rejected {
                return Err(ProjectionError::InvalidReview(event.event_id.to_string()));
            }
            Ok(TimelineState::Rejected)
        }
        MemoryReviewDecision::Supersede => {
            if event.governance.review != ReviewState::Superseded {
                return Err(ProjectionError::InvalidReview(event.event_id.to_string()));
            }
            Ok(TimelineState::Superseded)
        }
    }
}

fn validate_projection_governance(event: &TwinEvent) -> Result<(), ProjectionError> {
    let valid = match &event.payload {
        TwinEventPayload::ObservationRecorded(_) => {
            event.governance.review == ReviewState::NotApplicable
                && event.governance.authority == AuthorityClass::EvidenceObservation
        }
        TwinEventPayload::MemoryProposed(_) => {
            event.governance.review == ReviewState::Pending
                && event.governance.authority == AuthorityClass::EvidenceObservation
        }
        TwinEventPayload::MemoryReviewed(_) => {
            review_state(event)?;
            true
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(ProjectionError::InvalidReview(event.event_id.to_string()))
    }
}

fn matching_observation_counts(
    events: &[TwinEvent],
    active_superseded: &BTreeSet<EventId>,
    claim: &ClaimAssertion,
    variant: &RelationshipVariant,
    reference_time: DateTime<Utc>,
) -> (u16, u16, Vec<EventId>) {
    let mut support = BTreeSet::new();
    let mut opposition = BTreeSet::new();
    for event in events {
        if active_superseded.contains(&event.event_id)
            || !time_active(event, reference_time)
            || event.governance.sensitivity == Sensitivity::Restricted
            || matches!(
                event.governance.review,
                ReviewState::Rejected | ReviewState::Superseded
            )
            || relationship_variant(event, reference_time) != *variant
        {
            continue;
        }
        let TwinEventPayload::ObservationRecorded(observation) = &event.payload else {
            continue;
        };
        for observed in &observation.claims {
            if observed.subject_id == claim.subject_id
                && observed.predicate == claim.predicate
                && observed.object == claim.object
            {
                if observed.polarity == claim.polarity {
                    support.insert(event.event_id.clone());
                } else {
                    opposition.insert(event.event_id.clone());
                }
            }
        }
    }
    let support_count = u16::try_from(support.len()).unwrap_or(u16::MAX);
    let opposition_count = u16::try_from(opposition.len()).unwrap_or(u16::MAX);
    (
        support_count,
        opposition_count,
        support.into_iter().collect(),
    )
}

fn event_evidence(event: &TwinEvent) -> Result<Vec<EventId>, ProjectionError> {
    let mut ids = event
        .evidence
        .iter()
        .filter(|evidence| evidence.evidence_type == EvidenceType::Event)
        .map(|evidence| {
            EventId::parse(evidence.source_id.as_str())
                .map_err(|_| ProjectionError::InvalidIdentifier(evidence.source_id.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn projected_observation_item(
    event: &TwinEvent,
    claim: &ClaimAssertion,
    item_suffix: &str,
    support_count: u16,
    opposition_count: u16,
    mut evidence_event_ids: Vec<EventId>,
    by_id: &BTreeMap<EventId, &TwinEvent>,
    events: &[TwinEvent],
    reference_time: DateTime<Utc>,
) -> Result<ProjectedStateItem, ProjectionError> {
    evidence_event_ids.extend(event_evidence(event)?);
    evidence_event_ids.sort();
    evidence_event_ids.dedup();
    let (causal_stream, governance) = effective_exposure(
        std::iter::once(event.event_id.clone()).chain(evidence_event_ids.iter().cloned()),
        by_id,
        reference_time,
        ReviewState::NotApplicable,
        AuthorityClass::EvidenceObservation,
    );
    evidence_event_ids.truncate(crate::models::twin_state::MAX_STATE_LINKS);
    let summary = match &event.payload {
        TwinEventPayload::ObservationRecorded(value) => value.summary.clone(),
        _ => None,
    };
    let mut superseded_by = active_superseders(events, &event.event_id, reference_time);
    superseded_by.truncate(crate::models::twin_state::MAX_STATE_LINKS);
    let mut goals = event.context.goals.clone();
    goals.sort();
    let mut tags = event.context.tags.clone();
    tags.sort();
    Ok(ProjectedStateItem {
        item_id: Identifier::parse(format!("observation:{}:{item_suffix}", event.event_id))
            .map_err(ProjectionError::InvalidIdentifier)?,
        kind: ProjectedItemKind::Observation,
        claim: claim.clone(),
        summary,
        proposal_event_id: None,
        review_event_ids: Vec::new(),
        causal_stream,
        governance,
        relationship_variant: relationship_variant(event, reference_time),
        evidence_event_ids,
        support_count,
        opposition_count,
        prior_exact_support_count: support_count.saturating_sub(1),
        last_confirmed_at: fallback_confirmation(event, reference_time),
        valid_from: event.valid_from,
        valid_to: event.valid_to,
        superseded_by,
        goals,
        tags,
    })
}

fn observation_item(
    event: &TwinEvent,
    claim: &ClaimAssertion,
    claim_index: usize,
    events: &[TwinEvent],
    by_id: &BTreeMap<EventId, &TwinEvent>,
    active_superseded: &BTreeSet<EventId>,
    reference_time: DateTime<Utc>,
) -> Result<ProjectedStateItem, ProjectionError> {
    let variant = relationship_variant(event, reference_time);
    let (support_count, opposition_count, evidence_event_ids) =
        matching_observation_counts(events, active_superseded, claim, &variant, reference_time);
    projected_observation_item(
        event,
        claim,
        &claim_index.to_string(),
        support_count,
        opposition_count,
        evidence_event_ids,
        by_id,
        events,
        reference_time,
    )
}

fn companion_capture_note_id(event: &TwinEvent) -> Option<Identifier> {
    let TwinEventPayload::ObservationRecorded(observation) = &event.payload else {
        return None;
    };
    if event.context.source_channel.as_str() != "companion_capture"
        || !observation.claims.is_empty()
    {
        return None;
    }
    let content_digest = observation.content_digest.as_ref()?;
    let mut notes = event
        .evidence
        .iter()
        .filter(|evidence| evidence.evidence_type == EvidenceType::Note);
    let note = notes.next()?;
    if notes.next().is_some() || note.digest.as_ref() != Some(content_digest) {
        return None;
    }
    if observation.observation_id.as_str()
        != format!("companion-capture-{}", note.source_id).as_str()
    {
        return None;
    }
    Some(note.source_id.clone())
}

fn companion_capture_observation_item(
    event: &TwinEvent,
    events: &[TwinEvent],
    by_id: &BTreeMap<EventId, &TwinEvent>,
    reference_time: DateTime<Utc>,
) -> Result<Option<ProjectedStateItem>, ProjectionError> {
    let Some(note_id) = companion_capture_note_id(event) else {
        return Ok(None);
    };
    let claim = ClaimAssertion {
        subject_id: EntityId::parse("owner").map_err(ProjectionError::InvalidIdentifier)?,
        predicate: ClaimPredicate::parse("recorded_note")
            .map_err(ProjectionError::InvalidIdentifier)?,
        object: ClaimObject::parse(note_id.as_str()).map_err(ProjectionError::InvalidIdentifier)?,
        polarity: ClaimPolarity::Affirmed,
    };
    projected_observation_item(
        event,
        &claim,
        "capture",
        1,
        0,
        vec![event.event_id.clone()],
        by_id,
        events,
        reference_time,
    )
    .map(Some)
}

fn pending_draft_item(
    draft: &ProposalDraft,
    events: &[TwinEvent],
    by_id: &BTreeMap<EventId, &TwinEvent>,
    active_superseded: &BTreeSet<EventId>,
    reference_time: DateTime<Utc>,
) -> Result<(ProjectedStateItem, EventId), ProjectionError> {
    let (support_count, opposition_count, support_event_ids) = matching_observation_counts(
        events,
        active_superseded,
        &draft.claim,
        &draft.relationship_variant,
        reference_time,
    );
    let sources = support_event_ids
        .iter()
        .map(|id| {
            by_id
                .get(id)
                .copied()
                .ok_or_else(|| ProjectionError::DanglingReference(id.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Aggregate every matching support before applying each context vector's
    // own deterministic lexicographic bound. The 64 retained evidence IDs are
    // an explanation subset only and never define projected state.
    let goals = sources
        .iter()
        .flat_map(|event| event.context.goals.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(crate::models::twin_state::MAX_STATE_LINKS)
        .collect();
    let tags = sources
        .iter()
        .flat_map(|event| event.context.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(crate::models::twin_state::MAX_STATE_LINKS)
        .collect();
    let (causal_stream, governance) = effective_exposure(
        support_event_ids.iter().cloned(),
        by_id,
        reference_time,
        ReviewState::Pending,
        AuthorityClass::EvidenceObservation,
    );
    let timeline_source = sources
        .iter()
        .max_by_key(|event| {
            (
                fallback_confirmation(event, reference_time),
                event.event_id.clone(),
            )
        })
        .expect("proposal rules always cite evidence");
    let last_confirmed_at = fallback_confirmation(timeline_source, reference_time);
    let timeline_source_event_id = timeline_source.event_id.clone();

    Ok((
        ProjectedStateItem {
            item_id: draft.memory_id.clone(),
            kind: ProjectedItemKind::PendingProposal,
            claim: draft.claim.clone(),
            summary: None,
            proposal_event_id: None,
            review_event_ids: Vec::new(),
            causal_stream,
            governance,
            relationship_variant: draft.relationship_variant.clone(),
            evidence_event_ids: draft.evidence_event_ids.clone(),
            support_count,
            opposition_count,
            prior_exact_support_count: support_count,
            last_confirmed_at,
            valid_from: None,
            valid_to: None,
            superseded_by: Vec::new(),
            goals,
            tags,
        },
        timeline_source_event_id,
    ))
}

fn group_variants(
    reviewed: &[ProjectedStateItem],
    pending: &[ProjectedStateItem],
    observations: &[ProjectedStateItem],
) -> Vec<RelationshipVariantState> {
    #[derive(Default)]
    struct Group {
        reviewed: BTreeSet<Identifier>,
        pending: BTreeSet<Identifier>,
        observations: BTreeSet<EventId>,
    }
    let mut groups: BTreeMap<RelationshipVariant, Group> = BTreeMap::new();
    for item in reviewed {
        groups
            .entry(item.relationship_variant.clone())
            .or_default()
            .reviewed
            .insert(item.item_id.clone());
    }
    for item in pending {
        groups
            .entry(item.relationship_variant.clone())
            .or_default()
            .pending
            .insert(item.item_id.clone());
    }
    for item in observations {
        for id in &item.evidence_event_ids {
            groups
                .entry(item.relationship_variant.clone())
                .or_default()
                .observations
                .insert(id.clone());
        }
    }
    groups
        .into_iter()
        .map(|(relationship_variant, group)| RelationshipVariantState {
            relationship_variant,
            reviewed_memory_ids: group.reviewed.into_iter().collect(),
            pending_memory_ids: group.pending.into_iter().collect(),
            observation_event_ids: group.observations.into_iter().collect(),
        })
        .collect()
}

#[derive(serde::Serialize)]
struct SnapshotBody<'a> {
    schema_version: u16,
    projection_version: u16,
    attention_profile_version: u16,
    reference_time: DateTime<Utc>,
    applied_event_ids: &'a [EventId],
    reviewed_memories: &'a [ProjectedStateItem],
    pending_proposals: &'a [ProjectedStateItem],
    relationship_variants: &'a [RelationshipVariantState],
    timeline: &'a [TemporalStateEntry],
    contradiction_clusters: &'a [ContradictionCluster],
    recent_observations: &'a [ProjectedStateItem],
}

#[derive(serde::Serialize)]
struct LegacyTemporalStateEntry<'a> {
    item_id: &'a Identifier,
    source_event_id: &'a EventId,
    state: &'a TimelineState,
    effective_at: &'a DateTime<Utc>,
    valid_from: &'a Option<DateTime<Utc>>,
    valid_to: &'a Option<DateTime<Utc>>,
}

#[derive(serde::Serialize)]
struct LegacySnapshotBody<'a> {
    schema_version: u16,
    projection_version: u16,
    attention_profile_version: u16,
    reference_time: DateTime<Utc>,
    applied_event_ids: &'a [EventId],
    reviewed_memories: &'a [ProjectedStateItem],
    pending_proposals: &'a [ProjectedStateItem],
    relationship_variants: &'a [RelationshipVariantState],
    timeline: &'a [LegacyTemporalStateEntry<'a>],
    contradiction_clusters: &'a [ContradictionCluster],
    recent_observations: &'a [ProjectedStateItem],
}

fn snapshot_id_from_body_json(json: &[u8]) -> Result<SnapshotId, ProjectionError> {
    let mut hasher = Sha256::new();
    hasher.update((SNAPSHOT_ID_DOMAIN.len() as u64).to_be_bytes());
    hasher.update(SNAPSHOT_ID_DOMAIN);
    hasher.update((json.len() as u64).to_be_bytes());
    hasher.update(json);
    SnapshotId::parse(format!("{:x}", hasher.finalize())).map_err(ProjectionError::InvalidSnapshot)
}

fn derive_snapshot_id(snapshot: &ProjectionSnapshot) -> Result<SnapshotId, ProjectionError> {
    let body = SnapshotBody {
        schema_version: snapshot.schema_version,
        projection_version: snapshot.projection_version,
        attention_profile_version: snapshot.attention_profile_version,
        reference_time: snapshot.reference_time,
        applied_event_ids: &snapshot.applied_event_ids,
        reviewed_memories: &snapshot.reviewed_memories,
        pending_proposals: &snapshot.pending_proposals,
        relationship_variants: &snapshot.relationship_variants,
        timeline: &snapshot.timeline,
        contradiction_clusters: &snapshot.contradiction_clusters,
        recent_observations: &snapshot.recent_observations,
    };
    let json = serde_json::to_vec(&body)
        .map_err(|error| ProjectionError::Serialization(error.to_string()))?;
    snapshot_id_from_body_json(&json)
}

fn legacy_v2_snapshot_body_json(snapshot: &ProjectionSnapshot) -> Result<Vec<u8>, ProjectionError> {
    snapshot
        .validate()
        .map_err(ProjectionError::InvalidSnapshot)?;
    let mut timeline = snapshot
        .timeline
        .iter()
        .map(|entry| LegacyTemporalStateEntry {
            item_id: &entry.item_id,
            source_event_id: &entry.source_event_id,
            state: &entry.state,
            effective_at: &entry.effective_at,
            valid_from: &entry.valid_from,
            valid_to: &entry.valid_to,
        })
        .collect::<Vec<_>>();
    timeline.sort_by(|left, right| {
        left.source_event_id
            .cmp(right.source_event_id)
            .then_with(|| left.state.cmp(right.state))
            .then_with(|| left.item_id.cmp(right.item_id))
            .then_with(|| left.effective_at.cmp(right.effective_at))
            .then_with(|| left.valid_from.cmp(right.valid_from))
            .then_with(|| left.valid_to.cmp(right.valid_to))
    });
    serde_json::to_vec(&LegacySnapshotBody {
        schema_version: snapshot.schema_version,
        projection_version: LEGACY_PROJECTION_VERSION,
        attention_profile_version: snapshot.attention_profile_version,
        reference_time: snapshot.reference_time,
        applied_event_ids: &snapshot.applied_event_ids,
        reviewed_memories: &snapshot.reviewed_memories,
        pending_proposals: &snapshot.pending_proposals,
        relationship_variants: &snapshot.relationship_variants,
        timeline: &timeline,
        contradiction_clusters: &snapshot.contradiction_clusters,
        recent_observations: &snapshot.recent_observations,
    })
    .map_err(|error| ProjectionError::Serialization(error.to_string()))
}

pub(crate) fn legacy_v2_snapshot_id(
    snapshot: &ProjectionSnapshot,
) -> Result<SnapshotId, ProjectionError> {
    snapshot_id_from_body_json(&legacy_v2_snapshot_body_json(snapshot)?)
}

pub fn canonical_snapshot_json(snapshot: &ProjectionSnapshot) -> Result<Vec<u8>, ProjectionError> {
    snapshot
        .validate()
        .map_err(ProjectionError::InvalidSnapshot)?;
    if derive_snapshot_id(snapshot)? != snapshot.snapshot_id {
        return Err(ProjectionError::Serialization(
            "snapshot ID does not match canonical content".into(),
        ));
    }
    serde_json::to_vec(snapshot).map_err(|error| ProjectionError::Serialization(error.to_string()))
}

/// Rebuilds versioned Twin state from immutable events at an explicit time.
/// It returns typed causality/reference/governance errors and performs no I/O.
pub fn project(
    events: &[TwinEvent],
    reference_time: DateTime<Utc>,
) -> Result<ProjectionSnapshot, ProjectionError> {
    let eligible = events
        .iter()
        .filter(|event| event.recorded_at <= reference_time)
        .cloned()
        .collect::<Vec<_>>();
    for event in &eligible {
        event.validate().map_err(ProjectionError::InvalidEvent)?;
        validate_projection_governance(event)?;
    }
    let applied = topological_order(&eligible).map_err(ProjectionError::EventOrder)?;
    if applied.len() > MAX_PROJECTED_ITEMS {
        return Err(ProjectionError::TooManyItems("events"));
    }
    validate_references(&applied)?;
    let by_id = applied
        .iter()
        .map(|event| (event.event_id.clone(), event))
        .collect::<BTreeMap<_, _>>();
    let active_superseded = active_superseded_event_ids(&applied, reference_time);

    let mut proposals = BTreeMap::<Identifier, &TwinEvent>::new();
    let mut reviews = BTreeMap::<Identifier, Vec<&TwinEvent>>::new();
    let mut known_nonmemory_ids = BTreeSet::<Identifier>::new();
    for event in &applied {
        match &event.payload {
            TwinEventPayload::MemoryProposed(value) => {
                if proposals.insert(value.memory_id.clone(), event).is_some() {
                    return Err(ProjectionError::DuplicateMemory(value.memory_id.to_string()));
                }
            }
            TwinEventPayload::MemoryReviewed(value) => {
                reviews.entry(value.memory_id.clone()).or_default().push(event);
            }
            TwinEventPayload::DecisionRecorded(value) => {
                known_nonmemory_ids.insert(value.decision_id.clone());
            }
            TwinEventPayload::DecisionOutcomeRecorded(value) => {
                if !applied.iter().any(|candidate| {
                    matches!(&candidate.payload, TwinEventPayload::DecisionRecorded(decision) if decision.decision_id == value.decision_id)
                }) {
                    return Err(ProjectionError::DanglingMemory(value.decision_id.to_string()));
                }
            }
            _ => {}
        }
    }
    for memory_id in reviews.keys() {
        if !proposals.contains_key(memory_id) {
            if known_nonmemory_ids.contains(memory_id) {
                let event = reviews
                    .get(memory_id)
                    .and_then(|values| values.first())
                    .expect("review exists");
                return Err(ProjectionError::WrongReferenceType(event.event_id.clone()));
            }
            return Err(ProjectionError::DanglingMemory(memory_id.to_string()));
        }
    }
    let materialized_memory_ids = proposals.keys().cloned().collect::<BTreeSet<_>>();

    let mut timeline = Vec::new();
    let mut recent_observations = Vec::new();
    for event in &applied {
        if let TwinEventPayload::ObservationRecorded(observation) = &event.payload {
            if let Some(item) =
                companion_capture_observation_item(event, &applied, &by_id, reference_time)?
            {
                timeline.push(TemporalStateEntry {
                    item_id: item.item_id.clone(),
                    source_event_id: event.event_id.clone(),
                    state: TimelineState::Observed,
                    effective_at: fallback_confirmation(event, reference_time),
                    relationship_variant: item.relationship_variant.clone(),
                    valid_from: event.valid_from,
                    valid_to: event.valid_to,
                });
                if item.is_temporally_active(reference_time)
                    && event.governance.sensitivity != Sensitivity::Restricted
                    && !matches!(
                        event.governance.review,
                        ReviewState::Rejected | ReviewState::Superseded
                    )
                {
                    recent_observations.push(item);
                }
            }
            for (index, claim) in observation.claims.iter().enumerate() {
                let item = observation_item(
                    event,
                    claim,
                    index,
                    &applied,
                    &by_id,
                    &active_superseded,
                    reference_time,
                )?;
                timeline.push(TemporalStateEntry {
                    item_id: item.item_id.clone(),
                    source_event_id: event.event_id.clone(),
                    state: TimelineState::Observed,
                    effective_at: fallback_confirmation(event, reference_time),
                    relationship_variant: item.relationship_variant.clone(),
                    valid_from: event.valid_from,
                    valid_to: event.valid_to,
                });
                if item.is_temporally_active(reference_time)
                    && event.governance.sensitivity != Sensitivity::Restricted
                    && !matches!(
                        event.governance.review,
                        ReviewState::Rejected | ReviewState::Superseded
                    )
                {
                    recent_observations.push(item);
                }
            }
        }
    }

    let mut reviewed_memories = Vec::new();
    let mut pending_proposals = Vec::new();
    for (memory_id, proposal_event) in proposals {
        let TwinEventPayload::MemoryProposed(proposal) = &proposal_event.payload else {
            unreachable!()
        };
        let variant = relationship_variant(proposal_event, reference_time);
        timeline.push(TemporalStateEntry {
            item_id: memory_id.clone(),
            source_event_id: proposal_event.event_id.clone(),
            state: TimelineState::Pending,
            effective_at: fallback_confirmation(proposal_event, reference_time),
            relationship_variant: variant.clone(),
            valid_from: proposal_event.valid_from,
            valid_to: proposal_event.valid_to,
        });
        let memory_reviews = reviews.get(&memory_id).cloned().unwrap_or_default();
        for review in &memory_reviews {
            timeline.push(TemporalStateEntry {
                item_id: memory_id.clone(),
                source_event_id: review.event_id.clone(),
                state: review_state(review)?,
                effective_at: review.observed_at,
                relationship_variant: variant.clone(),
                valid_from: review.valid_from,
                valid_to: review.valid_to,
            });
        }
        let active_reviews = memory_reviews
            .iter()
            .copied()
            .filter(|event| {
                time_active(event, reference_time) && !active_superseded.contains(&event.event_id)
            })
            .collect::<Vec<_>>();
        let maximal = maximal_reviews(&active_reviews, &by_id);
        let maximal_outcomes = maximal
            .iter()
            .map(|event| review_outcome(proposal_event, event, &by_id, reference_time))
            .collect::<BTreeSet<_>>();
        let conflict = maximal_outcomes.len() > 1;
        let selected_review = (!conflict).then(|| maximal.first().copied()).flatten();
        if conflict {
            timeline.push(TemporalStateEntry {
                item_id: memory_id.clone(),
                source_event_id: maximal[0].event_id.clone(),
                state: TimelineState::PendingConflict,
                effective_at: maximal
                    .iter()
                    .map(|event| event.observed_at)
                    .max()
                    .expect("conflict has reviews"),
                relationship_variant: variant.clone(),
                valid_from: proposal_event.valid_from,
                valid_to: proposal_event.valid_to,
            });
        }

        let (claim, accepted) = match selected_review {
            Some(review_event) => {
                let TwinEventPayload::MemoryReviewed(review) = &review_event.payload else {
                    unreachable!()
                };
                (
                    review
                        .reviewed_claim
                        .clone()
                        .unwrap_or_else(|| proposal.claim.clone()),
                    review.decision == MemoryReviewDecision::Accept,
                )
            }
            None => (proposal.claim.clone(), false),
        };
        let (support_count, opposition_count, observed_evidence) = matching_observation_counts(
            &applied,
            &active_superseded,
            &claim,
            &variant,
            reference_time,
        );
        let mut evidence_event_ids = event_evidence(proposal_event)?;
        evidence_event_ids.extend(observed_evidence);
        evidence_event_ids.sort();
        evidence_event_ids.dedup();
        let exposure_ids = std::iter::once(proposal_event.event_id.clone())
            .chain(
                selected_review
                    .into_iter()
                    .map(|event| event.event_id.clone()),
            )
            .chain(evidence_event_ids.iter().cloned())
            .collect::<Vec<_>>();
        let (causal_stream, governance) = effective_exposure(
            exposure_ids,
            &by_id,
            reference_time,
            selected_review
                .map(|event| event.governance.review.clone())
                .unwrap_or(ReviewState::Pending),
            selected_review
                .map(|event| event.governance.authority.clone())
                .unwrap_or(AuthorityClass::EvidenceObservation),
        );
        evidence_event_ids.truncate(crate::models::twin_state::MAX_STATE_LINKS);
        let mut confirmation = fallback_confirmation(proposal_event, reference_time);
        if accepted {
            confirmation = selected_review.expect("accepted review exists").observed_at;
        }
        let all_review_ids = memory_reviews
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let review_ids = all_review_ids
            .iter()
            .take(crate::models::twin_state::MAX_STATE_LINKS)
            .cloned()
            .collect::<Vec<_>>();
        let reinforcement_targets = std::iter::once(proposal_event.event_id.clone())
            .chain(
                accepted
                    .then_some(selected_review)
                    .flatten()
                    .map(|event| event.event_id.clone()),
            )
            .collect::<BTreeSet<_>>();
        for event in &applied {
            if event
                .reinforces
                .iter()
                .any(|id| reinforcement_targets.contains(id))
            {
                if active_superseded.contains(&event.event_id)
                    || !time_active(event, reference_time)
                    || matches!(
                        event.governance.review,
                        ReviewState::Rejected | ReviewState::Superseded
                    )
                {
                    continue;
                }
                confirmation = confirmation.max(event.observed_at);
            }
        }
        let mut superseded_by =
            active_superseders(&applied, &proposal_event.event_id, reference_time);
        for review_id in &all_review_ids {
            superseded_by.extend(
                active_superseders(&applied, review_id, reference_time)
                    .into_iter()
                    .filter(|superseder_id| {
                        !matches!(
                            &by_id[superseder_id].payload,
                            TwinEventPayload::MemoryReviewed(review)
                                if review.memory_id == memory_id
                        )
                    }),
            );
        }
        superseded_by.sort();
        superseded_by.dedup();
        superseded_by.truncate(crate::models::twin_state::MAX_STATE_LINKS);
        let mut goals = proposal_event.context.goals.clone();
        goals.sort();
        let mut tags = proposal_event.context.tags.clone();
        tags.sort();
        let item = ProjectedStateItem {
            item_id: memory_id.clone(),
            kind: if accepted {
                ProjectedItemKind::ReviewedMemory
            } else {
                ProjectedItemKind::PendingProposal
            },
            claim,
            summary: proposal.summary.clone(),
            proposal_event_id: Some(proposal_event.event_id.clone()),
            review_event_ids: review_ids,
            causal_stream,
            governance,
            relationship_variant: variant.clone(),
            evidence_event_ids,
            support_count,
            opposition_count,
            prior_exact_support_count: support_count,
            last_confirmed_at: confirmation,
            valid_from: proposal_event.valid_from,
            valid_to: proposal_event.valid_to,
            superseded_by: superseded_by.clone(),
            goals,
            tags,
        };

        let expired = !time_active(proposal_event, reference_time);
        if !superseded_by.is_empty() {
            timeline.push(TemporalStateEntry {
                item_id: memory_id,
                source_event_id: superseded_by[0].clone(),
                state: TimelineState::Superseded,
                effective_at: by_id[&superseded_by[0]].observed_at,
                relationship_variant: variant.clone(),
                valid_from: proposal_event.valid_from,
                valid_to: proposal_event.valid_to,
            });
        } else if expired {
            timeline.push(TemporalStateEntry {
                item_id: memory_id,
                source_event_id: proposal_event.event_id.clone(),
                state: TimelineState::Expired,
                effective_at: proposal_event.valid_to.unwrap_or(reference_time),
                relationship_variant: variant,
                valid_from: proposal_event.valid_from,
                valid_to: proposal_event.valid_to,
            });
        } else if accepted {
            reviewed_memories.push(item);
        } else if selected_review.is_none() || conflict {
            pending_proposals.push(item);
        }
    }

    let derived =
        build_proposal_drafts(&applied, reference_time).map_err(ProjectionError::Proposal)?;
    for draft in &derived.drafts {
        if materialized_memory_ids.contains(&draft.memory_id) {
            continue;
        }
        let (item, source_event_id) =
            pending_draft_item(draft, &applied, &by_id, &active_superseded, reference_time)?;
        timeline.push(TemporalStateEntry {
            item_id: item.item_id.clone(),
            source_event_id,
            state: TimelineState::Pending,
            effective_at: item.last_confirmed_at,
            relationship_variant: item.relationship_variant.clone(),
            valid_from: None,
            valid_to: None,
        });
        pending_proposals.push(item);
    }

    reviewed_memories.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    pending_proposals.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    recent_observations.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    timeline.sort_by(|left, right| {
        left.source_event_id
            .cmp(&right.source_event_id)
            .then_with(|| left.state.cmp(&right.state))
            .then_with(|| left.item_id.cmp(&right.item_id))
            .then_with(|| left.effective_at.cmp(&right.effective_at))
            .then_with(|| left.relationship_variant.cmp(&right.relationship_variant))
            .then_with(|| left.valid_from.cmp(&right.valid_from))
            .then_with(|| left.valid_to.cmp(&right.valid_to))
    });
    let relationship_variants =
        group_variants(&reviewed_memories, &pending_proposals, &recent_observations);
    let mut snapshot = ProjectionSnapshot {
        schema_version: PROJECTION_SCHEMA_VERSION,
        projection_version: PROJECTION_VERSION,
        attention_profile_version: ATTENTION_PROFILE_VERSION,
        reference_time,
        applied_event_ids: applied
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        reviewed_memories,
        pending_proposals,
        relationship_variants,
        timeline,
        contradiction_clusters: derived.contradiction_clusters,
        recent_observations,
        snapshot_id: SnapshotId::parse(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("static digest"),
    };
    snapshot.snapshot_id = derive_snapshot_id(&snapshot)?;
    snapshot
        .validate()
        .map_err(ProjectionError::InvalidSnapshot)?;
    Ok(snapshot)
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
