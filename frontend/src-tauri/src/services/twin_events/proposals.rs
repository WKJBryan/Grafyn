use super::topological_order;
use crate::models::twin_event::{
    ClaimAssertion, ClaimObject, ClaimPolarity, ClaimPredicate, EntityId, EventId, ReviewState,
    Sensitivity, TwinEvent, TwinEventPayload,
};
use crate::models::twin_state::{
    ContradictionCluster, ProposalDraft, ProposalDraftSet, ProposalRule, RelationshipKey,
    RelationshipVariant,
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROPOSAL_RULE_VERSION: u16 = 1;
const MEMORY_ID_DOMAIN: &[u8] = b"grafyn.memory_proposal.exact_claim.v1";
const CLUSTER_ID_DOMAIN: &[u8] = b"grafyn.memory_proposal.contradiction_cluster.v1";

#[derive(Debug)]
pub enum ProposalError {
    EventOrder(super::StoreError),
    InvalidIdentifier(String),
    Serialization(String),
    TooManyDrafts,
    InvalidEvent(String),
}

impl std::fmt::Display for ProposalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventOrder(error) => write!(formatter, "cannot order proposal evidence: {error}"),
            Self::InvalidIdentifier(error) => {
                write!(formatter, "invalid derived proposal ID: {error}")
            }
            Self::Serialization(error) => {
                write!(formatter, "cannot canonicalize proposal: {error}")
            }
            Self::TooManyDrafts => formatter.write_str("proposal output exceeds its item limit"),
            Self::InvalidEvent(error) => write!(formatter, "invalid proposal input event: {error}"),
        }
    }
}

impl std::error::Error for ProposalError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClaimBase {
    subject_id: EntityId,
    predicate: ClaimPredicate,
    object: ClaimObject,
    relationship_variant: RelationshipVariant,
}

fn time_active(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    reference_time: DateTime<Utc>,
) -> bool {
    from.is_none_or(|value| reference_time >= value)
        && to.is_none_or(|value| reference_time <= value)
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
                    && time_active(
                        relationship.valid_from,
                        relationship.valid_to,
                        reference_time,
                    )
            })
            .map(RelationshipKey::from)
            .collect(),
    )
}

fn write_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn canonical_claim_variant_bytes(
    claim: &ClaimAssertion,
    variant: &RelationshipVariant,
) -> Result<Vec<u8>, ProposalError> {
    #[derive(serde::Serialize)]
    struct Identity<'a> {
        rule_version: u16,
        claim: &'a ClaimAssertion,
        relationship_variant: &'a RelationshipVariant,
    }
    serde_json::to_vec(&Identity {
        rule_version: PROPOSAL_RULE_VERSION,
        claim,
        relationship_variant: variant,
    })
    .map_err(|error| ProposalError::Serialization(error.to_string()))
}

fn derived_id(
    prefix: &str,
    domain: &[u8],
    claim: &ClaimAssertion,
    variant: &RelationshipVariant,
) -> Result<crate::models::twin_event::Identifier, ProposalError> {
    let mut hasher = Sha256::new();
    write_framed(&mut hasher, domain);
    write_framed(&mut hasher, &canonical_claim_variant_bytes(claim, variant)?);
    crate::models::twin_event::Identifier::parse(format!("{prefix}:{:x}", hasher.finalize()))
        .map_err(ProposalError::InvalidIdentifier)
}

fn base_claim(base: &ClaimBase, polarity: ClaimPolarity) -> ClaimAssertion {
    ClaimAssertion {
        subject_id: base.subject_id.clone(),
        predicate: base.predicate.clone(),
        object: base.object.clone(),
        polarity,
    }
}

/// Derives stable, pending-only drafts from observation events. It never appends
/// events or promotes memory. Malformed causality is returned as `EventOrder`.
pub fn build_proposal_drafts(
    events: &[TwinEvent],
    reference_time: DateTime<Utc>,
) -> Result<ProposalDraftSet, ProposalError> {
    let eligible = events
        .iter()
        .filter(|event| event.recorded_at <= reference_time)
        .cloned()
        .collect::<Vec<_>>();
    for event in &eligible {
        event.validate().map_err(ProposalError::InvalidEvent)?;
    }
    let applied = topological_order(&eligible).map_err(ProposalError::EventOrder)?;
    let superseded = applied
        .iter()
        .flat_map(|event| event.supersedes.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut evidence: BTreeMap<(ClaimBase, ClaimPolarity), BTreeSet<EventId>> = BTreeMap::new();
    for event in &applied {
        if superseded.contains(&event.event_id)
            || event.governance.sensitivity == Sensitivity::Restricted
            || matches!(
                event.governance.review,
                ReviewState::Rejected | ReviewState::Superseded
            )
            || !time_active(event.valid_from, event.valid_to, reference_time)
        {
            continue;
        }
        let TwinEventPayload::ObservationRecorded(observation) = &event.payload else {
            continue;
        };
        let variant = relationship_variant(event, reference_time);
        for claim in &observation.claims {
            evidence
                .entry((
                    ClaimBase {
                        subject_id: claim.subject_id.clone(),
                        predicate: claim.predicate.clone(),
                        object: claim.object.clone(),
                        relationship_variant: variant.clone(),
                    },
                    claim.polarity.clone(),
                ))
                .or_default()
                .insert(event.event_id.clone());
        }
    }

    let bases = evidence
        .keys()
        .map(|(base, _)| base.clone())
        .collect::<BTreeSet<_>>();
    let mut drafts = Vec::new();
    let mut clusters = Vec::new();
    for base in bases {
        let affirmed = evidence
            .get(&(base.clone(), ClaimPolarity::Affirmed))
            .cloned()
            .unwrap_or_default();
        let denied = evidence
            .get(&(base.clone(), ClaimPolarity::Denied))
            .cloned()
            .unwrap_or_default();
        let contradictory = !affirmed.is_empty() && !denied.is_empty();
        let mut cluster_memory_ids = Vec::new();
        let mut cluster_claims = Vec::new();
        let mut cluster_evidence = BTreeSet::new();
        for (polarity, item_evidence) in [
            (ClaimPolarity::Affirmed, affirmed),
            (ClaimPolarity::Denied, denied),
        ] {
            if item_evidence.len() < 3 && !contradictory {
                continue;
            }
            let claim = base_claim(&base, polarity);
            let memory_id = derived_id(
                "memory",
                MEMORY_ID_DOMAIN,
                &claim,
                &base.relationship_variant,
            )?;
            let mut rules = Vec::new();
            if contradictory {
                rules.push(ProposalRule::Contradiction);
            }
            if item_evidence.len() >= 3 {
                rules.push(ProposalRule::RepeatedExactClaim);
            }
            rules.sort();
            cluster_memory_ids.push(memory_id.clone());
            cluster_claims.push(claim.clone());
            cluster_evidence.extend(item_evidence.iter().cloned());
            drafts.push(ProposalDraft {
                memory_id,
                claim,
                relationship_variant: base.relationship_variant.clone(),
                evidence_event_ids: item_evidence.into_iter().collect(),
                rules,
            });
            if drafts.len() > crate::models::twin_state::MAX_PROJECTED_ITEMS {
                return Err(ProposalError::TooManyDrafts);
            }
        }
        if contradictory {
            cluster_memory_ids.sort();
            cluster_claims.sort();
            let identity_claim = base_claim(&base, ClaimPolarity::Affirmed);
            clusters.push(ContradictionCluster {
                cluster_id: derived_id(
                    "conflict",
                    CLUSTER_ID_DOMAIN,
                    &identity_claim,
                    &base.relationship_variant,
                )?,
                relationship_variant: base.relationship_variant,
                memory_ids: cluster_memory_ids,
                claims: cluster_claims,
                evidence_event_ids: cluster_evidence.into_iter().collect(),
            });
            if clusters.len() > crate::models::twin_state::MAX_PROJECTED_ITEMS {
                return Err(ProposalError::TooManyDrafts);
            }
        }
    }
    drafts.sort_by(|left, right| left.memory_id.cmp(&right.memory_id));
    clusters.sort_by(|left, right| left.cluster_id.cmp(&right.cluster_id));
    Ok(ProposalDraftSet {
        rule_version: PROPOSAL_RULE_VERSION,
        drafts,
        contradiction_clusters: clusters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::*;
    use crate::services::twin_events::{derive_event_id, test_support::valid_event_for_device};
    use chrono::{TimeZone, Utc};

    fn observation(device: &str, polarity: ClaimPolarity, relationship: Option<&str>) -> TwinEvent {
        let mut event = valid_event_for_device(device, 1, Vec::new());
        let TwinEventPayload::ObservationRecorded(payload) = &mut event.payload else {
            unreachable!()
        };
        payload.claims[0].polarity = polarity;
        if let Some(person) = relationship {
            event.context.relationships = vec![RelationshipAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: RelationshipPredicate::parse("works_with").unwrap(),
                object_id: EntityId::parse(person).unwrap(),
                direction: RelationshipDirection::Directed,
                valid_from: None,
                valid_to: None,
                evidence: Vec::new(),
                governance: Governance::direct_observation(),
            }];
        }
        event.event_id = derive_event_id(&event);
        event
    }

    fn reference() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn proposal_entry_point_requires_three_exact_observations() {
        let two = vec![
            observation("device-a", ClaimPolarity::Affirmed, None),
            observation("device-b", ClaimPolarity::Affirmed, None),
        ];
        assert!(build_proposal_drafts(&two, reference())
            .unwrap()
            .drafts
            .is_empty());
        let mut three = two;
        three.push(observation("device-c", ClaimPolarity::Affirmed, None));
        let result = build_proposal_drafts(&three, reference()).unwrap();
        assert_eq!(result.drafts.len(), 1);
        assert_eq!(result.drafts[0].evidence_event_ids.len(), 3);
        assert_eq!(
            result.drafts[0].rules,
            vec![ProposalRule::RepeatedExactClaim]
        );
    }

    #[test]
    fn contradiction_creates_two_pending_drafts_and_one_unresolved_cluster() {
        let events = vec![
            observation("device-a", ClaimPolarity::Affirmed, None),
            observation("device-b", ClaimPolarity::Denied, None),
        ];
        let result = build_proposal_drafts(&events, reference()).unwrap();
        assert_eq!(result.drafts.len(), 2);
        assert_eq!(result.contradiction_clusters.len(), 1);
        assert!(result
            .drafts
            .iter()
            .all(|draft| draft.rules == vec![ProposalRule::Contradiction]));
    }

    #[test]
    fn relationship_variants_and_global_claims_never_collapse() {
        let events = vec![
            observation("device-a", ClaimPolarity::Affirmed, None),
            observation("device-b", ClaimPolarity::Affirmed, None),
            observation("device-c", ClaimPolarity::Affirmed, None),
            observation("device-d", ClaimPolarity::Affirmed, Some("manager")),
            observation("device-e", ClaimPolarity::Affirmed, Some("manager")),
            observation("device-f", ClaimPolarity::Affirmed, Some("manager")),
            observation("device-g", ClaimPolarity::Affirmed, Some("friend")),
            observation("device-h", ClaimPolarity::Affirmed, Some("friend")),
            observation("device-i", ClaimPolarity::Affirmed, Some("friend")),
        ];
        let result = build_proposal_drafts(&events, reference()).unwrap();
        assert_eq!(result.drafts.len(), 3);
        assert_eq!(
            result
                .drafts
                .iter()
                .map(|draft| draft.memory_id.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn ids_and_bytes_are_stable_under_input_reordering_and_evidence_time_changes() {
        let mut events = vec![
            observation("device-a", ClaimPolarity::Affirmed, None),
            observation("device-b", ClaimPolarity::Affirmed, None),
            observation("device-c", ClaimPolarity::Affirmed, None),
        ];
        let first = build_proposal_drafts(&events, reference()).unwrap();
        events.reverse();
        let second = build_proposal_drafts(&events, reference()).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        let original_id = first.drafts[0].memory_id.clone();
        events[0].observed_at = reference();
        events[0].event_id = derive_event_id(&events[0]);
        let changed = build_proposal_drafts(&events, reference()).unwrap();
        assert_eq!(changed.drafts[0].memory_id, original_id);
    }

    #[test]
    fn closed_relationship_validity_and_governance_are_applied_before_variant_identity() {
        let mut event = observation("device-a", ClaimPolarity::Affirmed, Some("manager"));
        event.context.relationships[0].valid_from = Some(reference());
        event.context.relationships[0].valid_to = Some(reference());
        event.event_id = derive_event_id(&event);
        let result = build_proposal_drafts(
            &[
                event.clone(),
                observation("device-b", ClaimPolarity::Affirmed, Some("manager")),
                observation("device-c", ClaimPolarity::Affirmed, Some("manager")),
            ],
            reference(),
        )
        .unwrap();
        assert_eq!(result.drafts[0].relationship_variant.relationships.len(), 1);
        event.context.relationships[0].governance.sensitivity = Sensitivity::Restricted;
        event.event_id = derive_event_id(&event);
        let result = build_proposal_drafts(
            &[
                event,
                observation("device-d", ClaimPolarity::Affirmed, None),
                observation("device-e", ClaimPolarity::Affirmed, None),
            ],
            reference(),
        )
        .unwrap();
        assert!(result.drafts[0]
            .relationship_variant
            .relationships
            .is_empty());
    }
}
