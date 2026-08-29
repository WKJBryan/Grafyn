use crate::models::twin_event::{AuthorityClass, ReviewState, Sensitivity, Visibility};
use crate::models::twin_state::{
    AttentionCandidate, AttentionComponent, AttentionContribution, AttentionExplanation,
    AttentionProfile, AttentionRequest, AttentionScore, AttentionVector, AttentionWeight,
    ExcludedSelection, ExclusionReasonCode, ProjectedItemKind, RankedSelection,
    RelationshipVariant, SelectionDestination, SelectionTrace, SnapshotId, BASIS_POINTS_MAX,
};
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

pub const ATTENTION_PROFILE_VERSION: u16 = 1;
pub const RECENCY_HORIZON_SECONDS: u64 = 31_536_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionError {
    ArithmeticOverflow,
    InvalidWeightSum(u32),
    ComponentOutOfRange(u16),
    RequestTooLarge(&'static str),
}

impl std::fmt::Display for AttentionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("attention arithmetic overflow"),
            Self::InvalidWeightSum(sum) => {
                write!(formatter, "attention weights sum to {sum}, not 10000")
            }
            Self::ComponentOutOfRange(score) => {
                write!(formatter, "attention component {score} exceeds 10000")
            }
            Self::RequestTooLarge(field) => {
                write!(formatter, "attention request {field} is too large")
            }
        }
    }
}

impl std::error::Error for AttentionError {}

pub fn profile_weights(profile: AttentionProfile) -> Vec<AttentionWeight> {
    use AttentionComponent::*;
    let values: &[(AttentionComponent, u16)] = match profile {
        AttentionProfile::Recall => &[(Relevance, 4_000), (Confidence, 3_000), (Recency, 3_000)],
        AttentionProfile::Decision => &[
            (Confidence, 2_500),
            (GoalMatch, 2_500),
            (RelationshipMatch, 2_500),
            (Contradiction, 2_500),
        ],
        AttentionProfile::Simulation => &[
            (RelationshipMatch, 4_000),
            (Recurrence, 3_500),
            (Recency, 2_500),
        ],
        AttentionProfile::Reflection => &[
            (Contradiction, 4_000),
            (Novelty, 3_000),
            (Recurrence, 3_000),
        ],
        AttentionProfile::CaptureReview => &[
            (Novelty, 4_000),
            (Recurrence, 3_500),
            (ConfidenceGap, 2_500),
        ],
    };
    values
        .iter()
        .map(|(component, weight)| AttentionWeight {
            component: *component,
            weight: *weight,
        })
        .collect()
}

/// Computes one checked weighted mean. No component is rounded separately; after
/// checked accumulation this performs the sole profile rounding operation,
/// `(sum + 5000) / 10000` (integer half-up).
pub fn weighted_score(values: &[(u16, u16)]) -> Result<u16, AttentionError> {
    let mut weight_sum = 0_u32;
    let mut sum = 0_u64;
    for (score, weight) in values {
        if *score > BASIS_POINTS_MAX {
            return Err(AttentionError::ComponentOutOfRange(*score));
        }
        weight_sum = weight_sum
            .checked_add(u32::from(*weight))
            .ok_or(AttentionError::ArithmeticOverflow)?;
        let contribution = u64::from(*score)
            .checked_mul(u64::from(*weight))
            .ok_or(AttentionError::ArithmeticOverflow)?;
        sum = sum
            .checked_add(contribution)
            .ok_or(AttentionError::ArithmeticOverflow)?;
    }
    if weight_sum != u32::from(BASIS_POINTS_MAX) {
        return Err(AttentionError::InvalidWeightSum(weight_sum));
    }
    let rounded = sum
        .checked_add(5_000)
        .ok_or(AttentionError::ArithmeticOverflow)?
        / 10_000;
    u16::try_from(rounded).map_err(|_| AttentionError::ArithmeticOverflow)
}

fn ratio_basis_points(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        return 0;
    }
    let scaled = numerator.saturating_mul(10_000);
    let rounded = scaled.saturating_add(denominator / 2) / denominator;
    rounded.min(10_000) as u16
}

fn normalize_exact(value: &str) -> String {
    value
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_set(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn set_jaccard<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>, empty_empty: u16) -> u16 {
    if left.is_empty() && right.is_empty() {
        return empty_empty;
    }
    ratio_basis_points(
        left.intersection(right).count() as u64,
        left.union(right).count() as u64,
    )
}

pub fn relevance_score(candidate: &AttentionCandidate, query: &str) -> AttentionScore {
    let query = token_set(query);
    let item = &candidate.item;
    let mut text = format!(
        "{} {} {}",
        item.claim.subject_id.as_str(),
        item.claim.predicate.as_str(),
        item.claim.object.as_str()
    );
    if let Some(summary) = &item.summary {
        text.push(' ');
        text.push_str(summary.as_str());
    }
    for value in item.goals.iter().chain(&item.tags) {
        text.push(' ');
        text.push_str(value);
    }
    AttentionScore::new(set_jaccard(&query, &token_set(&text), 0)).expect("bounded jaccard")
}

pub fn recurrence_score(support_count: u16) -> AttentionScore {
    AttentionScore::new(support_count.min(5) * 2_000).expect("bounded recurrence")
}

pub fn contradiction_score(opposition_count: u16) -> AttentionScore {
    AttentionScore::new(opposition_count.min(5) * 2_000).expect("bounded contradiction")
}

pub fn confidence_score(support_count: u16, opposition_count: u16) -> AttentionScore {
    let recurrence = recurrence_score(support_count).get();
    let ratio = ratio_basis_points(
        u64::from(support_count),
        u64::from(support_count) + u64::from(opposition_count),
    );
    AttentionScore::new(recurrence.min(ratio)).expect("bounded confidence")
}

pub fn novelty_score(prior_exact_support_count: u16) -> AttentionScore {
    AttentionScore::new(10_000 - prior_exact_support_count.min(5) * 2_000).expect("bounded novelty")
}

fn relationship_set(variant: &RelationshipVariant) -> BTreeSet<String> {
    variant
        .relationships
        .iter()
        .map(|relationship| {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{:?}",
                relationship.subject_id.as_str(),
                relationship.predicate.as_str(),
                relationship.object_id.as_str(),
                relationship.direction
            )
        })
        .collect()
}

pub fn relationship_match_score(
    candidate: &RelationshipVariant,
    requested: &RelationshipVariant,
) -> AttentionScore {
    AttentionScore::new(set_jaccard(
        &relationship_set(candidate),
        &relationship_set(requested),
        10_000,
    ))
    .expect("bounded relationship jaccard")
}

pub fn goal_match_score(candidate: &[String], requested: &[String]) -> AttentionScore {
    if requested.is_empty() {
        return AttentionScore::ZERO;
    }
    let normalize = |values: &[String]| {
        values
            .iter()
            .map(|value| normalize_exact(value))
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>()
    };
    AttentionScore::new(set_jaccard(&normalize(candidate), &normalize(requested), 0))
        .expect("bounded goal jaccard")
}

pub fn recency_score(
    last_confirmed_at: DateTime<Utc>,
    reference_time: DateTime<Utc>,
) -> AttentionScore {
    let seconds = reference_time
        .signed_duration_since(last_confirmed_at)
        .num_seconds()
        .max(0) as u64;
    if seconds >= RECENCY_HORIZON_SECONDS {
        return AttentionScore::ZERO;
    }
    AttentionScore::new(ratio_basis_points(
        RECENCY_HORIZON_SECONDS - seconds,
        RECENCY_HORIZON_SECONDS,
    ))
    .expect("bounded recency")
}

pub fn attention_vector(
    candidate: &AttentionCandidate,
    request: &AttentionRequest,
) -> AttentionVector {
    let item = &candidate.item;
    AttentionVector {
        relevance: relevance_score(candidate, &request.query),
        confidence: confidence_score(item.support_count, item.opposition_count),
        recency: recency_score(item.last_confirmed_at, request.reference_time),
        recurrence: recurrence_score(item.support_count),
        novelty: novelty_score(item.prior_exact_support_count),
        relationship_match: relationship_match_score(
            &item.relationship_variant,
            &request.relationship_variant,
        ),
        goal_match: goal_match_score(&item.goals, &request.goals),
        contradiction: contradiction_score(item.opposition_count),
    }
}

pub fn score_candidate(
    candidate: &AttentionCandidate,
    profile: AttentionProfile,
    request: &AttentionRequest,
) -> Result<AttentionExplanation, AttentionError> {
    let vector = attention_vector(candidate, request);
    let weights = profile_weights(profile);
    let values = weights
        .iter()
        .map(|weight| (vector.component(weight.component).get(), weight.weight))
        .collect::<Vec<_>>();
    let final_score = AttentionScore::new(weighted_score(&values)?)
        .map_err(|_| AttentionError::ArithmeticOverflow)?;
    let contributions = weights
        .iter()
        .map(|weight| {
            let component_score = vector.component(weight.component);
            AttentionContribution {
                component: weight.component,
                component_score,
                weight: weight.weight,
                weighted_basis_points: u64::from(component_score.get()) * u64::from(weight.weight),
            }
        })
        .collect::<Vec<_>>();
    let explanation = format!(
        "v{} {:?}: {} => {}bp",
        ATTENTION_PROFILE_VERSION,
        profile,
        contributions
            .iter()
            .map(|part| format!(
                "{:?}={}x{}",
                part.component,
                part.component_score.get(),
                part.weight
            ))
            .collect::<Vec<_>>()
            .join(", "),
        final_score.get()
    );
    Ok(AttentionExplanation {
        profile,
        profile_version: ATTENTION_PROFILE_VERSION,
        vector,
        contributions,
        final_score,
        explanation,
    })
}

fn allowed_use(
    candidate: &AttentionCandidate,
    profile: AttentionProfile,
    destination: SelectionDestination,
) -> bool {
    let allowed = &candidate.item.governance.allowed_uses;
    if matches!(
        profile,
        AttentionProfile::Reflection | AttentionProfile::CaptureReview
    ) && destination != SelectionDestination::Local
    {
        return false;
    }
    let profile_allowed = match profile {
        AttentionProfile::Recall | AttentionProfile::Reflection => allowed.recall,
        AttentionProfile::Decision => allowed.twin_advisor,
        AttentionProfile::Simulation => allowed.twin_simulation,
        AttentionProfile::CaptureReview => true,
    };
    profile_allowed
        && match destination {
            SelectionDestination::Export => allowed.export,
            SelectionDestination::Sync => allowed.sync,
            SelectionDestination::Local | SelectionDestination::Network => true,
        }
}

fn first_exclusion(
    candidate: &AttentionCandidate,
    profile: AttentionProfile,
    request: &AttentionRequest,
) -> Option<ExclusionReasonCode> {
    let item = &candidate.item;
    if !allowed_use(candidate, profile, request.destination) {
        return Some(ExclusionReasonCode::AllowedUse);
    }
    if item.governance.sensitivity == Sensitivity::Restricted {
        return Some(ExclusionReasonCode::Sensitivity);
    }
    if request.destination != SelectionDestination::Local
        && item.governance.visibility == Visibility::LocalOnly
    {
        return Some(ExclusionReasonCode::Visibility);
    }
    let review_allowed = match profile {
        AttentionProfile::Simulation => item.governance.review == ReviewState::Accepted,
        AttentionProfile::CaptureReview => {
            (item.kind == ProjectedItemKind::PendingProposal
                && item.governance.review == ReviewState::Pending)
                || (item.kind == ProjectedItemKind::Observation
                    && item.governance.review == ReviewState::NotApplicable)
        }
        AttentionProfile::Recall | AttentionProfile::Decision | AttentionProfile::Reflection => {
            matches!(
                item.governance.review,
                ReviewState::Accepted | ReviewState::NotApplicable
            )
        }
    };
    if !review_allowed
        || matches!(
            item.governance.review,
            ReviewState::Rejected | ReviewState::Superseded
        )
    {
        return Some(ExclusionReasonCode::Review);
    }
    let authority_allowed = match profile {
        AttentionProfile::Simulation => matches!(
            item.governance.authority,
            AuthorityClass::ReviewedMemory
                | AuthorityClass::CanonicalUserRule
                | AuthorityClass::DeterministicallyVerified { .. }
        ),
        AttentionProfile::CaptureReview => {
            matches!(
                item.governance.authority,
                AuthorityClass::EvidenceObservation
            )
        }
        AttentionProfile::Recall | AttentionProfile::Decision | AttentionProfile::Reflection => {
            matches!(
                item.governance.authority,
                AuthorityClass::EvidenceObservation
                    | AuthorityClass::ReviewedMemory
                    | AuthorityClass::CanonicalUserRule
                    | AuthorityClass::DeterministicallyVerified { .. }
            )
        }
    };
    if !authority_allowed {
        return Some(ExclusionReasonCode::Authority);
    }
    if !item.is_temporally_active(request.reference_time) {
        return Some(ExclusionReasonCode::Temporal);
    }
    None
}

/// Ranks only candidates that survive the fixed governance gate order. Errors
/// describe malformed/bounded requests or checked arithmetic failures; excluded
/// content is represented in the trace by item ID and the first reason code only.
pub fn rank(
    snapshot_id: SnapshotId,
    candidates: &[AttentionCandidate],
    profile: AttentionProfile,
    request: &AttentionRequest,
) -> Result<SelectionTrace, AttentionError> {
    if request.query.len() > 32_768 {
        return Err(AttentionError::RequestTooLarge("query"));
    }
    if request.goals.len() > 64 || request.relationship_variant.relationships.len() > 64 {
        return Err(AttentionError::RequestTooLarge("context"));
    }
    if candidates.len() > crate::models::twin_state::MAX_PROJECTED_ITEMS {
        return Err(AttentionError::RequestTooLarge("candidates"));
    }
    let mut ordered = candidates.to_vec();
    ordered.sort_by(|left, right| left.item.item_id.cmp(&right.item.item_id));
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    for candidate in &ordered {
        if let Some(reason) = first_exclusion(candidate, profile, request) {
            excluded.push(ExcludedSelection {
                item_id: candidate.item.item_id.clone(),
                reason,
            });
            continue;
        }
        selected.push(RankedSelection {
            item_id: candidate.item.item_id.clone(),
            attention: score_candidate(candidate, profile, request)?,
        });
    }
    selected.sort_by(|left, right| {
        right
            .attention
            .final_score
            .cmp(&left.attention.final_score)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    selected.truncate(usize::from(request.limit));
    Ok(SelectionTrace {
        snapshot_id,
        profile,
        profile_version: ATTENTION_PROFILE_VERSION,
        selected,
        excluded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::*;
    use crate::models::twin_state::*;
    use chrono::{Duration, TimeZone};

    fn governance(review: ReviewState, authority: AuthorityClass) -> Governance {
        Governance {
            review,
            authority,
            sensitivity: Sensitivity::Standard,
            visibility: Visibility::SyncedVault,
            allowed_uses: AllowedUses {
                recall: true,
                twin_advisor: true,
                twin_simulation: true,
                export: true,
                training: false,
                sync: true,
            },
        }
    }

    fn candidate(id: &str) -> AttentionCandidate {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        AttentionCandidate {
            item: ProjectedStateItem {
                item_id: Identifier::parse(id).unwrap(),
                kind: ProjectedItemKind::ReviewedMemory,
                claim: ClaimAssertion {
                    subject_id: EntityId::parse("owner").unwrap(),
                    predicate: ClaimPredicate::parse("prefers").unwrap(),
                    object: ClaimObject::parse("quiet focused work").unwrap(),
                    polarity: ClaimPolarity::Affirmed,
                },
                summary: Some(BoundedSummary::parse("deep work in quiet rooms").unwrap()),
                proposal_event_id: None,
                review_event_ids: Vec::new(),
                governance: governance(ReviewState::Accepted, AuthorityClass::ReviewedMemory),
                relationship_variant: RelationshipVariant::global(),
                evidence_event_ids: Vec::new(),
                support_count: 3,
                opposition_count: 1,
                prior_exact_support_count: 2,
                last_confirmed_at: at,
                valid_from: None,
                valid_to: None,
                superseded_by: Vec::new(),
                goals: vec!["ship safely".into()],
                tags: vec!["work".into()],
            },
        }
    }

    fn request(at: DateTime<Utc>) -> AttentionRequest {
        AttentionRequest {
            query: "quiet work".into(),
            relationship_variant: RelationshipVariant::global(),
            goals: vec!["ship safely".into()],
            reference_time: at,
            destination: SelectionDestination::Local,
            limit: 20,
        }
    }

    #[test]
    fn profiles_use_fixed_v1_weights_and_half_up_rounding() {
        for profile in [
            AttentionProfile::Recall,
            AttentionProfile::Decision,
            AttentionProfile::Simulation,
            AttentionProfile::Reflection,
            AttentionProfile::CaptureReview,
        ] {
            assert_eq!(
                profile_weights(profile)
                    .iter()
                    .map(|item| u32::from(item.weight))
                    .sum::<u32>(),
                10_000
            );
        }
        assert_eq!(
            weighted_score(&[(5_001, 5_000), (5_000, 5_000)]).unwrap(),
            5_001
        );
        assert!(matches!(
            weighted_score(&[(1, 1)]),
            Err(AttentionError::InvalidWeightSum(1))
        ));
        use AttentionComponent::*;
        let pairs = |profile| {
            profile_weights(profile)
                .into_iter()
                .map(|value| (value.component, value.weight))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pairs(AttentionProfile::Recall),
            vec![(Relevance, 4_000), (Confidence, 3_000), (Recency, 3_000)]
        );
        assert_eq!(
            pairs(AttentionProfile::Decision),
            vec![
                (Confidence, 2_500),
                (GoalMatch, 2_500),
                (RelationshipMatch, 2_500),
                (Contradiction, 2_500)
            ]
        );
        assert_eq!(
            pairs(AttentionProfile::Simulation),
            vec![
                (RelationshipMatch, 4_000),
                (Recurrence, 3_500),
                (Recency, 2_500)
            ]
        );
        assert_eq!(
            pairs(AttentionProfile::Reflection),
            vec![
                (Contradiction, 4_000),
                (Novelty, 3_000),
                (Recurrence, 3_000)
            ]
        );
        assert_eq!(
            pairs(AttentionProfile::CaptureReview),
            vec![
                (Novelty, 4_000),
                (Recurrence, 3_500),
                (ConfidenceGap, 2_500)
            ]
        );
    }

    #[test]
    fn every_component_is_bounded_and_matches_v1_formulas() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let vector = attention_vector(&candidate("memory-a"), &request(at));
        assert_eq!(vector.recurrence.get(), 6_000);
        assert_eq!(vector.contradiction.get(), 2_000);
        assert_eq!(vector.confidence.get(), 6_000);
        assert_eq!(vector.novelty.get(), 6_000);
        assert_eq!(vector.relationship_match.get(), 10_000);
        assert_eq!(vector.goal_match.get(), 10_000);
        assert_eq!(vector.recency.get(), 10_000);
        for score in [
            vector.relevance,
            vector.confidence,
            vector.recency,
            vector.recurrence,
            vector.novelty,
            vector.relationship_match,
            vector.goal_match,
            vector.contradiction,
        ] {
            assert!(score.get() <= 10_000);
        }
        assert_eq!(confidence_score(1, 2).get(), 2_000);
        assert_eq!(confidence_score(5, 1).get(), 8_333);
        assert_eq!(recurrence_score(100).get(), 10_000);
        assert_eq!(contradiction_score(100).get(), 10_000);
        assert_eq!(novelty_score(100).get(), 0);
    }

    #[test]
    fn time_and_context_components_have_declared_edge_behavior() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        assert_eq!(recency_score(at + Duration::seconds(1), at).get(), 10_000);
        assert_eq!(
            recency_score(at - Duration::seconds(RECENCY_HORIZON_SECONDS as i64), at).get(),
            0
        );
        assert_eq!(goal_match_score(&[], &[]).get(), 0);
        assert_eq!(
            relationship_match_score(
                &RelationshipVariant::global(),
                &RelationshipVariant::global()
            )
            .get(),
            10_000
        );
        let mut relation = candidate("relation").item.relationship_variant;
        relation.relationships.push(RelationshipKey {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("manager").unwrap(),
            direction: RelationshipDirection::Directed,
        });
        assert_eq!(
            relationship_match_score(&relation, &RelationshipVariant::global()).get(),
            0
        );
    }

    #[test]
    fn ranking_is_score_descending_then_id_ascending_with_full_explanation() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let trace = rank(
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap(),
            &[candidate("memory-b"), candidate("memory-a")],
            AttentionProfile::Recall,
            &request(at),
        )
        .unwrap();
        assert_eq!(trace.selected[0].item_id.as_str(), "memory-a");
        assert_eq!(trace.selected[0].attention.contributions.len(), 3);
        assert!(trace.selected[0]
            .attention
            .explanation
            .contains("Relevance"));
    }

    #[test]
    fn contradiction_component_boosts_reflection_without_changing_claim_identity() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let low = candidate("low-contradiction");
        let mut high = candidate("high-contradiction");
        high.item.opposition_count = 5;
        assert!(
            score_candidate(&high, AttentionProfile::Reflection, &request(at))
                .unwrap()
                .final_score
                > score_candidate(&low, AttentionProfile::Reflection, &request(at))
                    .unwrap()
                    .final_score
        );
    }

    #[test]
    fn gates_are_first_failure_only_and_run_before_scores() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let mut blocked = candidate("blocked");
        blocked.item.support_count = u16::MAX;
        blocked.item.governance.allowed_uses.recall = false;
        blocked.item.governance.sensitivity = Sensitivity::Restricted;
        blocked.item.governance.visibility = Visibility::LocalOnly;
        blocked.item.governance.review = ReviewState::Pending;
        blocked.item.governance.authority = AuthorityClass::EvidenceObservation;
        blocked.item.valid_to = Some(at - Duration::seconds(1));
        let trace = rank(
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap(),
            &[blocked],
            AttentionProfile::Recall,
            &AttentionRequest {
                destination: SelectionDestination::Network,
                ..request(at)
            },
        )
        .unwrap();
        assert!(trace.selected.is_empty());
        assert_eq!(trace.excluded[0].reason, ExclusionReasonCode::AllowedUse);
    }

    #[test]
    fn each_gate_reports_its_stable_reason_and_local_only_is_local_eligible() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let snapshot = || {
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap()
        };
        let run = |candidate: AttentionCandidate, profile, destination| {
            rank(
                snapshot(),
                &[candidate],
                profile,
                &AttentionRequest {
                    destination,
                    ..request(at)
                },
            )
            .unwrap()
        };
        let mut value = candidate("sensitive");
        value.item.governance.sensitivity = Sensitivity::Restricted;
        assert_eq!(
            run(value, AttentionProfile::Recall, SelectionDestination::Local).excluded[0].reason,
            ExclusionReasonCode::Sensitivity
        );
        let mut value = candidate("local-only");
        value.item.governance.visibility = Visibility::LocalOnly;
        assert_eq!(
            run(
                value.clone(),
                AttentionProfile::Recall,
                SelectionDestination::Network
            )
            .excluded[0]
                .reason,
            ExclusionReasonCode::Visibility
        );
        assert_eq!(
            run(value, AttentionProfile::Recall, SelectionDestination::Local)
                .selected
                .len(),
            1
        );
        let mut value = candidate("pending");
        value.item.governance.review = ReviewState::Pending;
        assert_eq!(
            run(value, AttentionProfile::Recall, SelectionDestination::Local).excluded[0].reason,
            ExclusionReasonCode::Review
        );
        let mut value = candidate("weak-authority");
        value.item.governance.authority = AuthorityClass::EvidenceObservation;
        assert_eq!(
            run(
                value,
                AttentionProfile::Simulation,
                SelectionDestination::Local
            )
            .excluded[0]
                .reason,
            ExclusionReasonCode::Authority
        );
        let mut value = candidate("expired");
        value.item.valid_to = Some(at - Duration::seconds(1));
        assert_eq!(
            run(value, AttentionProfile::Recall, SelectionDestination::Local).excluded[0].reason,
            ExclusionReasonCode::Temporal
        );
    }

    #[test]
    fn one_hundred_high_recurrence_pending_proposals_never_enter_simulation() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let values = (0..100)
            .map(|index| {
                let mut value = candidate(&format!("pending-{index}"));
                value.item.kind = ProjectedItemKind::PendingProposal;
                value.item.support_count = u16::MAX;
                value.item.governance.review = ReviewState::Pending;
                value.item.governance.authority = AuthorityClass::EvidenceObservation;
                value
            })
            .collect::<Vec<_>>();
        let trace = rank(
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap(),
            &values,
            AttentionProfile::Simulation,
            &request(at),
        )
        .unwrap();
        assert!(trace.selected.is_empty());
        assert_eq!(trace.excluded.len(), 100);
    }

    #[test]
    fn reflection_and_capture_review_are_local_only_surfaces() {
        let at = Utc.with_ymd_and_hms(2026, 8, 29, 0, 0, 0).unwrap();
        let snapshot = || {
            SnapshotId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap()
        };
        for profile in [
            AttentionProfile::Reflection,
            AttentionProfile::CaptureReview,
        ] {
            let mut value = candidate("local-surface");
            if profile == AttentionProfile::CaptureReview {
                value.item.kind = ProjectedItemKind::PendingProposal;
                value.item.governance.review = ReviewState::Pending;
                value.item.governance.authority = AuthorityClass::EvidenceObservation;
            }
            let trace = rank(
                snapshot(),
                &[value],
                profile,
                &AttentionRequest {
                    destination: SelectionDestination::Network,
                    ..request(at)
                },
            )
            .unwrap();
            assert_eq!(trace.excluded[0].reason, ExclusionReasonCode::AllowedUse);
        }
    }
}
