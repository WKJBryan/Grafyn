use super::{build_proposal_drafts, topological_order, ATTENTION_PROFILE_VERSION};
use crate::models::twin_event::{
    AllowedUses, AuthorityClass, ClaimAssertion, EventId, EvidenceType, Governance, Identifier,
    MemoryReviewDecision, ReviewState, Sensitivity, TwinEvent, TwinEventPayload, Visibility,
};
use crate::models::twin_state::{
    ContradictionCluster, ProjectedItemKind, ProjectedStateItem, ProjectionSnapshot, ProposalDraft,
    RelationshipKey, RelationshipVariant, RelationshipVariantState, SnapshotId, StateModelError,
    TemporalStateEntry, TimelineState, MAX_PROJECTED_ITEMS,
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECTION_SCHEMA_VERSION: u16 = 1;
pub const PROJECTION_VERSION: u16 = 1;
const SNAPSHOT_ID_DOMAIN: &[u8] = b"grafyn.twin_projection.canonical_json.v1";

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

fn relationship_variant(event: &TwinEvent, reference_time: DateTime<Utc>) -> RelationshipVariant {
    RelationshipVariant::new(
        event
            .context
            .relationships
            .iter()
            .filter(|relationship| {
                relationship.governance.sensitivity != Sensitivity::Restricted
                    && !matches!(
                        relationship.governance.review,
                        ReviewState::Rejected | ReviewState::Superseded
                    )
                    && relationship
                        .valid_from
                        .is_none_or(|from| reference_time >= from)
                    && relationship.valid_to.is_none_or(|to| reference_time <= to)
            })
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
        for target in event.supersedes.iter().chain(&event.reinforces) {
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
                    && is_ancestor(&candidate.event_id, other, by_id)
            })
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    result
}

fn review_state(event: &TwinEvent) -> Result<TimelineState, ProjectionError> {
    let TwinEventPayload::MemoryReviewed(review) = &event.payload else {
        unreachable!("review_state called with non-review")
    };
    match review.decision {
        MemoryReviewDecision::Accept => {
            if event.governance.review != ReviewState::Accepted
                || !matches!(
                    event.governance.authority,
                    AuthorityClass::ReviewedMemory
                        | AuthorityClass::CanonicalUserRule
                        | AuthorityClass::DeterministicallyVerified { .. }
                )
            {
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

fn sensitivity_rank(value: &Sensitivity) -> u8 {
    match value {
        Sensitivity::Standard => 0,
        Sensitivity::Sensitive => 1,
        Sensitivity::Restricted => 2,
    }
}

fn conservative_review_governance(proposal: &Governance, review: &Governance) -> Governance {
    Governance {
        review: review.review.clone(),
        authority: review.authority.clone(),
        sensitivity: if sensitivity_rank(&proposal.sensitivity)
            >= sensitivity_rank(&review.sensitivity)
        {
            proposal.sensitivity.clone()
        } else {
            review.sensitivity.clone()
        },
        visibility: if proposal.visibility == Visibility::LocalOnly
            || review.visibility == Visibility::LocalOnly
        {
            Visibility::LocalOnly
        } else {
            Visibility::SyncedVault
        },
        allowed_uses: AllowedUses {
            recall: proposal.allowed_uses.recall && review.allowed_uses.recall,
            twin_advisor: proposal.allowed_uses.twin_advisor && review.allowed_uses.twin_advisor,
            twin_simulation: proposal.allowed_uses.twin_simulation
                && review.allowed_uses.twin_simulation,
            export: proposal.allowed_uses.export && review.allowed_uses.export,
            training: proposal.allowed_uses.training && review.allowed_uses.training,
            sync: proposal.allowed_uses.sync && review.allowed_uses.sync,
        },
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

fn observation_item(
    event: &TwinEvent,
    claim: &ClaimAssertion,
    claim_index: usize,
    events: &[TwinEvent],
    active_superseded: &BTreeSet<EventId>,
    reference_time: DateTime<Utc>,
) -> Result<ProjectedStateItem, ProjectionError> {
    let variant = relationship_variant(event, reference_time);
    let (support_count, opposition_count, evidence_event_ids) =
        matching_observation_counts(events, active_superseded, claim, &variant, reference_time);
    let summary = match &event.payload {
        TwinEventPayload::ObservationRecorded(value) => value.summary.clone(),
        _ => None,
    };
    Ok(ProjectedStateItem {
        item_id: Identifier::parse(format!("observation:{}:{claim_index}", event.event_id))
            .map_err(ProjectionError::InvalidIdentifier)?,
        kind: ProjectedItemKind::Observation,
        claim: claim.clone(),
        summary,
        proposal_event_id: None,
        review_event_ids: Vec::new(),
        governance: event.governance.clone(),
        relationship_variant: variant,
        evidence_event_ids,
        support_count,
        opposition_count,
        prior_exact_support_count: support_count.saturating_sub(1),
        last_confirmed_at: fallback_confirmation(event, reference_time),
        valid_from: event.valid_from,
        valid_to: event.valid_to,
        superseded_by: active_superseders(events, &event.event_id, reference_time),
        goals: event.context.goals.clone(),
        tags: event.context.tags.clone(),
    })
}

fn pending_draft_item(
    draft: &ProposalDraft,
    events: &[TwinEvent],
    by_id: &BTreeMap<EventId, &TwinEvent>,
    active_superseded: &BTreeSet<EventId>,
    reference_time: DateTime<Utc>,
) -> Result<ProjectedStateItem, ProjectionError> {
    let sources = draft
        .evidence_event_ids
        .iter()
        .map(|id| {
            by_id
                .get(id)
                .copied()
                .ok_or_else(|| ProjectionError::DanglingReference(id.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sensitivity = sources
        .iter()
        .map(|event| event.governance.sensitivity.clone())
        .max_by_key(sensitivity_rank)
        .unwrap_or(Sensitivity::Standard);
    let visibility = if sources
        .iter()
        .any(|event| event.governance.visibility == Visibility::LocalOnly)
    {
        Visibility::LocalOnly
    } else {
        Visibility::SyncedVault
    };
    let sync_allowed = visibility == Visibility::SyncedVault
        && sources
            .iter()
            .all(|event| event.governance.allowed_uses.sync);
    let goals = sources
        .iter()
        .flat_map(|event| event.context.goals.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let tags = sources
        .iter()
        .flat_map(|event| event.context.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let (support_count, opposition_count, _) = matching_observation_counts(
        events,
        active_superseded,
        &draft.claim,
        &draft.relationship_variant,
        reference_time,
    );
    let last_confirmed_at = sources
        .iter()
        .map(|event| fallback_confirmation(event, reference_time))
        .max()
        .expect("proposal rules always cite evidence");

    Ok(ProjectedStateItem {
        item_id: draft.memory_id.clone(),
        kind: ProjectedItemKind::PendingProposal,
        claim: draft.claim.clone(),
        summary: None,
        proposal_event_id: None,
        review_event_ids: Vec::new(),
        governance: Governance {
            review: ReviewState::Pending,
            authority: AuthorityClass::EvidenceObservation,
            sensitivity,
            visibility,
            allowed_uses: AllowedUses {
                recall: false,
                twin_advisor: false,
                twin_simulation: false,
                export: false,
                training: false,
                sync: sync_allowed,
            },
        },
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
    })
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
    let mut hasher = Sha256::new();
    hasher.update((SNAPSHOT_ID_DOMAIN.len() as u64).to_be_bytes());
    hasher.update(SNAPSHOT_ID_DOMAIN);
    hasher.update((json.len() as u64).to_be_bytes());
    hasher.update(json);
    SnapshotId::parse(format!("{:x}", hasher.finalize())).map_err(ProjectionError::InvalidSnapshot)
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
    let event_order = applied
        .iter()
        .enumerate()
        .map(|(index, event)| (event.event_id.clone(), index))
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
            for (index, claim) in observation.claims.iter().enumerate() {
                let item = observation_item(
                    event,
                    claim,
                    index,
                    &applied,
                    &active_superseded,
                    reference_time,
                )?;
                timeline.push(TemporalStateEntry {
                    item_id: item.item_id.clone(),
                    source_event_id: event.event_id.clone(),
                    state: TimelineState::Observed,
                    effective_at: fallback_confirmation(event, reference_time),
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
        timeline.push(TemporalStateEntry {
            item_id: memory_id.clone(),
            source_event_id: proposal_event.event_id.clone(),
            state: TimelineState::Pending,
            effective_at: fallback_confirmation(proposal_event, reference_time),
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
                valid_from: review.valid_from,
                valid_to: review.valid_to,
            });
        }
        let active_reviews = memory_reviews
            .iter()
            .copied()
            .filter(|event| time_active(event, reference_time))
            .collect::<Vec<_>>();
        let maximal = maximal_reviews(&active_reviews, &by_id);
        let maximal_decisions = maximal
            .iter()
            .filter_map(|event| match &event.payload {
                TwinEventPayload::MemoryReviewed(value) => Some(match value.decision {
                    MemoryReviewDecision::Accept => 0_u8,
                    MemoryReviewDecision::Reject => 1,
                    MemoryReviewDecision::Supersede => 2,
                }),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let conflict = maximal_decisions.len() > 1;
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
                valid_from: proposal_event.valid_from,
                valid_to: proposal_event.valid_to,
            });
        }

        let (claim, governance, accepted) = match selected_review {
            Some(review_event) => {
                let TwinEventPayload::MemoryReviewed(review) = &review_event.payload else {
                    unreachable!()
                };
                (
                    review
                        .reviewed_claim
                        .clone()
                        .unwrap_or_else(|| proposal.claim.clone()),
                    conservative_review_governance(
                        &proposal_event.governance,
                        &review_event.governance,
                    ),
                    review.decision == MemoryReviewDecision::Accept,
                )
            }
            None => (
                proposal.claim.clone(),
                proposal_event.governance.clone(),
                false,
            ),
        };
        let variant = relationship_variant(proposal_event, reference_time);
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
        let mut confirmation = fallback_confirmation(proposal_event, reference_time);
        if accepted {
            confirmation = selected_review.expect("accepted review exists").observed_at;
        }
        let review_ids = memory_reviews
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for event in &applied {
            if event.reinforces.contains(&proposal_event.event_id)
                || review_ids.iter().any(|id| event.reinforces.contains(id))
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
        for review_id in &review_ids {
            superseded_by.extend(active_superseders(&applied, review_id, reference_time));
        }
        superseded_by.sort();
        superseded_by.dedup();
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
            governance,
            relationship_variant: variant,
            evidence_event_ids,
            support_count,
            opposition_count,
            prior_exact_support_count: support_count,
            last_confirmed_at: confirmation,
            valid_from: proposal_event.valid_from,
            valid_to: proposal_event.valid_to,
            superseded_by: superseded_by.clone(),
            goals: proposal_event.context.goals.clone(),
            tags: proposal_event.context.tags.clone(),
        };

        let expired = !time_active(proposal_event, reference_time);
        if !superseded_by.is_empty() {
            timeline.push(TemporalStateEntry {
                item_id: memory_id,
                source_event_id: superseded_by[0].clone(),
                state: TimelineState::Superseded,
                effective_at: by_id[&superseded_by[0]].observed_at,
                valid_from: proposal_event.valid_from,
                valid_to: proposal_event.valid_to,
            });
        } else if expired {
            timeline.push(TemporalStateEntry {
                item_id: memory_id,
                source_event_id: proposal_event.event_id.clone(),
                state: TimelineState::Expired,
                effective_at: proposal_event.valid_to.unwrap_or(reference_time),
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
        let timeline_evidence = derived
            .contradiction_clusters
            .iter()
            .find(|cluster| cluster.memory_ids.contains(&draft.memory_id))
            .map(|cluster| cluster.evidence_event_ids.as_slice())
            .unwrap_or(&draft.evidence_event_ids);
        let source_event_id = timeline_evidence
            .iter()
            .max_by_key(|id| event_order.get(*id))
            .expect("proposal rules always cite evidence")
            .clone();
        let source_event = by_id[&source_event_id];
        let item = pending_draft_item(draft, &applied, &by_id, &active_superseded, reference_time)?;
        timeline.push(TemporalStateEntry {
            item_id: item.item_id.clone(),
            source_event_id,
            state: TimelineState::Pending,
            effective_at: fallback_confirmation(source_event, reference_time),
            valid_from: None,
            valid_to: None,
        });
        pending_proposals.push(item);
    }

    reviewed_memories.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    pending_proposals.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    recent_observations.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    timeline.sort_by(|left, right| {
        event_order[&left.source_event_id]
            .cmp(&event_order[&right.source_event_id])
            .then_with(|| left.state.cmp(&right.state))
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    let relationship_variants =
        group_variants(&reviewed_memories, &pending_proposals, &recent_observations);
    let mut snapshot = ProjectionSnapshot {
        schema_version: PROJECTION_SCHEMA_VERSION,
        projection_version: PROJECTION_VERSION,
        attention_profile_version: ATTENTION_PROFILE_VERSION,
        reference_time,
        applied_event_ids: applied.iter().map(|event| event.event_id.clone()).collect(),
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
mod tests {
    use super::*;
    use crate::models::twin_event::*;
    use crate::services::twin_events::{derive_event_id, test_support::valid_event_for_device};
    use chrono::{Duration, TimeZone};

    fn reference() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap()
    }

    fn proposal(device: &str, memory: &str) -> TwinEvent {
        let mut event = valid_event_for_device(device, 1, Vec::new());
        let claim = match &event.payload {
            TwinEventPayload::ObservationRecorded(value) => value.claims[0].clone(),
            _ => unreachable!(),
        };
        event.payload = TwinEventPayload::MemoryProposed(MemoryProposed {
            memory_id: Identifier::parse(memory).unwrap(),
            claim,
            summary: Some(BoundedSummary::parse("quiet-work preference").unwrap()),
            proposal_source: ProvenanceLabel::parse("exact-rule-v1").unwrap(),
        });
        event.event_type = TwinEventType::MemoryProposed;
        event.governance.review = ReviewState::Pending;
        event.event_id = derive_event_id(&event);
        event
    }

    fn review(
        device: &str,
        memory: &str,
        decision: MemoryReviewDecision,
        parents: Vec<EventId>,
    ) -> TwinEvent {
        let mut event = valid_event_for_device(device, 1, parents);
        event.payload = TwinEventPayload::MemoryReviewed(MemoryReviewed {
            memory_id: Identifier::parse(memory).unwrap(),
            decision: decision.clone(),
            reviewed_claim: None,
            rationale: None,
        });
        event.event_type = TwinEventType::MemoryReviewed;
        event.governance.review = match decision {
            MemoryReviewDecision::Accept => ReviewState::Accepted,
            MemoryReviewDecision::Reject => ReviewState::Rejected,
            MemoryReviewDecision::Supersede => ReviewState::Superseded,
        };
        event.governance.authority = AuthorityClass::ReviewedMemory;
        event.event_id = derive_event_id(&event);
        event
    }

    #[test]
    fn projection_entry_point_is_pure_and_explicitly_timed() {
        let snapshot = project(&[], reference()).unwrap();
        assert_eq!(snapshot.reference_time, reference());
        assert!(snapshot.reviewed_memories.is_empty());
        assert_eq!(
            canonical_snapshot_json(&snapshot).unwrap(),
            serde_json::to_vec(&snapshot).unwrap()
        );
    }

    #[test]
    fn explicit_accept_promotes_while_reject_remains_audit_only() {
        let proposed = proposal("proposal-device", "memory-one");
        let accepted = review(
            "review-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        let snapshot = project(&[accepted.clone(), proposed.clone()], reference()).unwrap();
        assert_eq!(snapshot.reviewed_memories.len(), 1);
        assert!(snapshot.pending_proposals.is_empty());
        assert_eq!(
            snapshot.reviewed_memories[0].last_confirmed_at,
            accepted.observed_at
        );

        let rejected = review(
            "reject-device",
            "memory-one",
            MemoryReviewDecision::Reject,
            vec![proposed.event_id.clone()],
        );
        let snapshot = project(&[proposed, rejected], reference()).unwrap();
        assert!(snapshot.reviewed_memories.is_empty());
        assert!(snapshot.pending_proposals.is_empty());
        assert!(snapshot
            .timeline
            .iter()
            .any(|entry| entry.state == TimelineState::Rejected));
    }

    #[test]
    fn review_cannot_upgrade_proposal_privacy_or_allowed_uses() {
        let mut proposed = proposal("proposal-device", "memory-one");
        proposed.governance.sensitivity = Sensitivity::Restricted;
        proposed.governance.visibility = Visibility::LocalOnly;
        proposed.governance.allowed_uses.twin_simulation = false;
        proposed.governance.allowed_uses.export = false;
        proposed.event_id = derive_event_id(&proposed);
        let accepted = review(
            "review-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        let snapshot = project(&[proposed, accepted], reference()).unwrap();
        let governance = &snapshot.reviewed_memories[0].governance;
        assert_eq!(governance.sensitivity, Sensitivity::Restricted);
        assert_eq!(governance.visibility, Visibility::LocalOnly);
        assert!(!governance.allowed_uses.twin_simulation);
        assert!(!governance.allowed_uses.export);
    }

    #[test]
    fn review_decision_and_governance_state_must_agree() {
        let proposed = proposal("proposal-device", "memory-one");
        let mut accepted = review(
            "review-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        accepted.governance.review = ReviewState::Pending;
        accepted.event_id = derive_event_id(&accepted);
        assert!(matches!(
            project(&[proposed, accepted], reference()),
            Err(ProjectionError::InvalidReview(_))
        ));
    }

    #[test]
    fn closed_validity_negative_age_reinforcement_and_supersession_are_deterministic() {
        let mut proposed = proposal("proposal-device", "memory-one");
        proposed.valid_from = Some(reference());
        proposed.valid_to = Some(reference());
        proposed.event_id = derive_event_id(&proposed);
        let accepted = review(
            "review-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        let snapshot = project(&[proposed.clone(), accepted.clone()], reference()).unwrap();
        assert_eq!(snapshot.reviewed_memories.len(), 1);

        let mut reinforcement =
            valid_event_for_device("reinforce-device", 1, vec![accepted.event_id.clone()]);
        reinforcement.reinforces = vec![accepted.event_id.clone()];
        reinforcement.observed_at = reference() + Duration::days(1);
        reinforcement.event_id = derive_event_id(&reinforcement);
        let snapshot = project(
            &[proposed.clone(), accepted.clone(), reinforcement.clone()],
            reference(),
        )
        .unwrap();
        assert_eq!(
            snapshot.reviewed_memories[0].last_confirmed_at,
            reinforcement.observed_at
        );

        let mut superseder =
            valid_event_for_device("supersede-device", 1, vec![proposed.event_id.clone()]);
        superseder.supersedes = vec![proposed.event_id.clone()];
        superseder.event_id = derive_event_id(&superseder);
        let snapshot = project(
            &[proposed, accepted, superseder, reinforcement],
            reference(),
        )
        .unwrap();
        assert!(snapshot.reviewed_memories.is_empty());
        assert!(snapshot
            .timeline
            .iter()
            .any(|entry| entry.state == TimelineState::Superseded));
    }

    #[test]
    fn concurrent_contradictory_reviews_abstain_until_causally_resolved() {
        let proposed = proposal("proposal-device", "memory-one");
        let accept = review(
            "accept-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        let reject = review(
            "reject-device",
            "memory-one",
            MemoryReviewDecision::Reject,
            vec![proposed.event_id.clone()],
        );
        let snapshot = project(
            &[accept.clone(), proposed.clone(), reject.clone()],
            reference(),
        )
        .unwrap();
        assert!(snapshot.reviewed_memories.is_empty());
        assert_eq!(snapshot.pending_proposals.len(), 1);
        assert!(snapshot
            .timeline
            .iter()
            .any(|entry| entry.state == TimelineState::PendingConflict));

        let resolved = review(
            "resolution-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![accept.event_id.clone(), reject.event_id.clone()],
        );
        let snapshot = project(&[reject, resolved, proposed, accept], reference()).unwrap();
        assert_eq!(snapshot.reviewed_memories.len(), 1);
    }

    #[test]
    fn dangling_and_wrong_type_references_fail_closed() {
        let proposed = proposal("proposal-device", "memory-one");
        let dangling = review(
            "review-device",
            "missing-memory",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        assert!(matches!(
            project(&[proposed.clone(), dangling], reference()),
            Err(ProjectionError::DanglingMemory(_))
        ));

        let mut decision = valid_event_for_device("decision-device", 1, Vec::new());
        decision.payload = TwinEventPayload::DecisionRecorded(DecisionRecorded {
            decision_id: Identifier::parse("decision-one").unwrap(),
            decision: BoundedContent::parse("choose").unwrap(),
            options: Vec::new(),
            stakes: None,
            initial_leaning: None,
        });
        decision.event_type = TwinEventType::DecisionRecorded;
        decision.event_id = derive_event_id(&decision);
        let mut bad = proposed;
        bad.supersedes = vec![decision.event_id.clone()];
        bad.event_id = derive_event_id(&bad);
        assert!(matches!(
            project(&[decision, bad], reference()),
            Err(ProjectionError::WrongReferenceType(_))
        ));
    }

    #[test]
    fn relationship_variants_remain_distinct_and_reordered_input_is_byte_identical() {
        let mut global = proposal("global-device", "global-memory");
        let mut contextual = proposal("context-device", "context-memory");
        contextual.context.relationships = vec![RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("manager").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: Vec::new(),
            governance: Governance::direct_observation(),
        }];
        contextual.event_id = derive_event_id(&contextual);
        global.event_id = derive_event_id(&global);
        let first = project(&[contextual.clone(), global.clone()], reference()).unwrap();
        let second = project(&[global, contextual], reference()).unwrap();
        assert_eq!(
            canonical_snapshot_json(&first).unwrap(),
            canonical_snapshot_json(&second).unwrap()
        );
        assert_eq!(first.relationship_variants.len(), 2);
    }

    #[test]
    fn future_recorded_events_are_not_applied_and_direct_observations_never_become_memory() {
        let mut observation = valid_event_for_device("observation-device", 1, Vec::new());
        observation.recorded_at = reference() + Duration::seconds(1);
        observation.event_id = derive_event_id(&observation);
        let snapshot = project(&[observation], reference()).unwrap();
        assert!(snapshot.applied_event_ids.is_empty());
        assert!(snapshot.reviewed_memories.is_empty());
    }

    #[test]
    fn repeated_observations_are_projected_as_pending_rule_drafts() {
        let observations = ["observation-a", "observation-b", "observation-c"]
            .into_iter()
            .map(|device| valid_event_for_device(device, 1, Vec::new()))
            .collect::<Vec<_>>();

        let snapshot = project(&observations, reference()).unwrap();

        assert_eq!(snapshot.pending_proposals.len(), 1);
        let draft = &snapshot.pending_proposals[0];
        assert_eq!(draft.kind, ProjectedItemKind::PendingProposal);
        assert_eq!(draft.proposal_event_id, None);
        assert_eq!(draft.governance.review, ReviewState::Pending);
        assert_eq!(
            draft.governance.authority,
            AuthorityClass::EvidenceObservation
        );
        assert_eq!(draft.evidence_event_ids.len(), 3);
    }

    #[test]
    fn relationship_variant_state_lists_every_recent_observation_event() {
        let observations = ["observation-a", "observation-b", "observation-c"]
            .into_iter()
            .map(|device| valid_event_for_device(device, 1, Vec::new()))
            .collect::<Vec<_>>();
        let expected = observations
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let snapshot = project(&observations, reference()).unwrap();

        assert_eq!(snapshot.relationship_variants.len(), 1);
        assert_eq!(
            snapshot.relationship_variants[0].observation_event_ids,
            expected
        );
    }

    #[test]
    fn a_review_only_applies_inside_its_closed_validity_interval() {
        let proposed = proposal("proposal-device", "memory-one");
        let mut accepted = review(
            "review-device",
            "memory-one",
            MemoryReviewDecision::Accept,
            vec![proposed.event_id.clone()],
        );
        accepted.valid_from = Some(reference() + Duration::seconds(1));
        accepted.event_id = derive_event_id(&accepted);

        let before = project(&[proposed.clone(), accepted.clone()], reference()).unwrap();
        assert!(before.reviewed_memories.is_empty());
        assert_eq!(before.pending_proposals.len(), 1);

        let at_boundary =
            project(&[proposed, accepted], reference() + Duration::seconds(1)).unwrap();
        assert_eq!(at_boundary.reviewed_memories.len(), 1);
        assert!(at_boundary.pending_proposals.is_empty());
    }

    #[test]
    fn superseded_observations_do_not_contribute_current_support() {
        let observations = ["observation-a", "observation-b", "observation-c"]
            .into_iter()
            .map(|device| valid_event_for_device(device, 1, Vec::new()))
            .collect::<Vec<_>>();
        let mut proposed = proposal("proposal-device", "memory-one");
        let claim = match &observations[0].payload {
            TwinEventPayload::ObservationRecorded(value) => value.claims[0].clone(),
            _ => unreachable!(),
        };
        let TwinEventPayload::MemoryProposed(payload) = &mut proposed.payload else {
            unreachable!()
        };
        payload.claim = claim;
        proposed.event_id = derive_event_id(&proposed);

        let mut superseder = valid_event_for_device(
            "superseder-device",
            1,
            vec![observations[0].event_id.clone()],
        );
        superseder.payload = TwinEventPayload::NoteChanged(NoteChanged {
            note_id: Identifier::parse("note-one").unwrap(),
            change: NoteChangeKind::Updated,
            content_digest: None,
        });
        superseder.event_type = TwinEventType::NoteChanged;
        superseder.supersedes = vec![observations[0].event_id.clone()];
        superseder.event_id = derive_event_id(&superseder);

        let mut events = observations;
        events.extend([proposed, superseder]);
        let snapshot = project(&events, reference()).unwrap();
        let projected = snapshot
            .pending_proposals
            .iter()
            .find(|item| item.item_id.as_str() == "memory-one")
            .unwrap();
        assert_eq!(projected.support_count, 2);
        assert!(!projected.evidence_event_ids.contains(&events[0].event_id));
    }
}
