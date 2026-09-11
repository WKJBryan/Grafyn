use super::*;

fn fixture() -> (tempfile::TempDir, EvidenceStore, Relationship) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = EvidenceStore::new(dir.path().into(), "test".into(), "Test".into()).unwrap();
    let state = store
        .reconcile_sources(vec![
            SourceInput {
                id: "a".into(),
                subject_id: "test".into(),
                role: EvidenceRole::TargetStatement,
                text: "My goal is to reach 10000 prospective customers within 30 days.".into(),
                ..Default::default()
            },
            SourceInput {
                id: "b".into(),
                subject_id: "test".into(),
                role: EvidenceRole::TargetStatement,
                text: "I want 10000 potential buyers to see the launch within 30 days.".into(),
                ..Default::default()
            },
        ])
        .unwrap();
    let candidate = Relationship {
        id: "candidate".into(),
        subject_id: "test".into(),
        from_id: "a".into(),
        to_id: "b".into(),
        from_receipt: whole_receipt(&state.sources[0]),
        to_receipt: whole_receipt(&state.sources[1]),
        provenance: "local_semantic_similarity".into(),
        similarity: Some(0.91),
        explanation: "Embedding candidate".into(),
        recorded_at: now(),
        ..Default::default()
    };
    store
        .apply_discovery(DiscoveryOutput {
            embedding_version: None,
            relationships: vec![candidate.clone()],
            embedding_status: "test".into(),
        })
        .unwrap();
    (dir, store, candidate)
}

fn judgment(input: &Value, verdict: PairVerdict) -> PairJudgment {
    PairJudgment {
        verdict,
        direction: PairDirection::Symmetric,
        explanation:
            "Same stated launch reach target and duration, assuming the same counting rule.".into(),
        conditions: vec!["Same launch and counting rule".into()],
        from_quote: input["left"]["passage"].as_str().unwrap().into(),
        to_quote: input["right"]["passage"].as_str().unwrap().into(),
    }
}

#[test]
fn assessment_keeps_receipts_and_tentative_status_with_separate_candidate_score() {
    let (_dir, mut store, candidate) = fixture();
    let input = pair_input(&store.state, &candidate).unwrap();
    let record = new_record(&input, "test-digest".into(), 1);
    let reserved = store
        .reserve_assessment(&candidate, record.clone())
        .unwrap();
    let mut complete = record;
    complete.result = Some(judgment(&input, PairVerdict::Equivalent));
    let saved = store.finish_assessment(&reserved, complete).unwrap();
    let rel = &saved.relationships[0];
    assert_eq!(rel.relation, RelationshipKind::Equivalent);
    assert_eq!(rel.review_status, ReviewStatus::Tentative);
    assert_eq!(rel.similarity, Some(0.91));
    assert_eq!(rel.from_receipt, candidate.from_receipt);
    assert!(usable(&saved, rel));
    assert!(!due(&saved, rel));
}

#[test]
fn model_withholding_is_not_a_human_rejection_and_can_be_overridden() {
    for verdict in [PairVerdict::Unrelated, PairVerdict::Insufficient] {
        let (_dir, mut store, candidate) = fixture();
        let input = pair_input(&store.state, &candidate).unwrap();
        let mut record = new_record(&input, "test".into(), 1);
        let reserved = store
            .reserve_assessment(&candidate, record.clone())
            .unwrap();
        record.result = Some(judgment(&input, verdict));
        let saved = store.finish_assessment(&reserved, record).unwrap();
        assert!(!usable(&saved, &saved.relationships[0]));
        assert_eq!(
            saved.relationships[0].review_status,
            ReviewStatus::Tentative
        );
        store
            .review_relationship(&candidate.id, ReviewStatus::Confirmed, None)
            .unwrap();
        let saved = store.snapshot().unwrap();
        assert!(usable(&saved, &saved.relationships[0]));
        assert!(!due(&saved, &saved.relationships[0]));
    }
}

#[test]
fn fabricated_receipts_and_symmetric_effects_are_rejected() {
    let (_dir, store, candidate) = fixture();
    let input = pair_input(&store.state, &candidate).unwrap();
    let mut result = judgment(&input, PairVerdict::Related);
    result.from_quote = "An invented mechanism".into();
    assert!(validate_judgment(&input, &result).is_err());
    let result = judgment(&input, PairVerdict::Enables);
    assert!(validate_judgment(&input, &result).is_err());
    let mut result = judgment(&input, PairVerdict::Conflicts);
    result.conditions.clear();
    assert!(validate_judgment(&input, &result).is_err());
}

#[test]
fn reverse_effect_is_a_hypothesis_and_preserves_canonical_input_identity() {
    let (_dir, mut store, candidate) = fixture();
    let input = pair_input(&store.state, &candidate).unwrap();
    let mut record = new_record(&input, "test".into(), 1);
    let reserved = store
        .reserve_assessment(&candidate, record.clone())
        .unwrap();
    let mut result = judgment(&input, PairVerdict::Requires);
    result.direction = PairDirection::RightToLeft;
    record.result = Some(result);
    let saved = store.finish_assessment(&reserved, record).unwrap();
    let r = &saved.relationships[0];
    assert_eq!(r.from_id, "b");
    assert_eq!(r.to_id, "a");
    assert_eq!(r.causal_basis.as_deref(), Some("extracted_hypothesis"));
    assert_eq!(
        fingerprint(&pair_input(&saved, r).unwrap()),
        fingerprint(&input)
    );
    assert!(!due(&saved, r));
    let corrected = store
        .review_relationship(
            &r.id,
            ReviewStatus::Confirmed,
            Some(RelationshipKind::Related),
        )
        .unwrap();
    assert!(!corrected.directed);
    assert!(corrected.causal_basis.is_none());
}

#[test]
fn source_edits_or_human_review_beat_an_inflight_assessment() {
    for edit_source in [true, false] {
        let (_dir, mut store, candidate) = fixture();
        let input = pair_input(&store.state, &candidate).unwrap();
        let mut record = new_record(&input, "test".into(), 1);
        let reserved = store
            .reserve_assessment(&candidate, record.clone())
            .unwrap();
        record.result = Some(judgment(&input, PairVerdict::Equivalent));
        if edit_source {
            let mut inputs: Vec<_> = store
                .state
                .sources
                .iter()
                .map(|s| s.input.clone())
                .collect();
            inputs[0].text = "Changed target and counting rule".into();
            store.reconcile_sources(inputs).unwrap();
        } else {
            store
                .review_relationship(&candidate.id, ReviewStatus::Rejected, None)
                .unwrap();
        }
        assert!(store.finish_assessment(&reserved, record).is_err());
    }
}

#[test]
fn failed_attempts_have_cooldown_and_a_three_attempt_limit() {
    let (_dir, store, mut candidate) = fixture();
    let input = pair_input(&store.state, &candidate).unwrap();
    candidate.assessment = Some(new_record(&input, "test".into(), 1));
    assert!(!due(&store.state, &candidate));
    candidate.assessment.as_mut().unwrap().recorded_at = "2000-01-01T00:00:00Z".into();
    assert!(due(&store.state, &candidate));
    candidate.assessment.as_mut().unwrap().attempts = 3;
    assert!(!due(&store.state, &candidate));
}

#[test]
fn newer_goal_revision_changes_assessment_context_without_inventing_missing_quantities() {
    let (_dir, mut store, candidate) = fixture();
    let before = pair_input(&store.state, &candidate).unwrap();
    let goal = GoalInput {
        id: "reach".into(),
        subject_id: "test".into(),
        label: "Reach".into(),
        definition: "Reach prospective buyers".into(),
        receipts: vec![candidate.from_receipt.clone()],
        criteria: vec![GoalCriterion::default()],
        ..Default::default()
    };
    store.save_goal(goal.clone()).unwrap();
    let after = pair_input(&store.state, &candidate).unwrap();
    assert_ne!(fingerprint(&before), fingerprint(&after));
    assert!(after["goals"][0]["criteria"][0]["target"].is_null());
    let mut change = goal;
    change.criteria[0].deadline = Some("2027-01-01".into());
    store.save_goal(change).unwrap();
    assert_ne!(
        fingerprint(&after),
        fingerprint(&pair_input(&store.state, &candidate).unwrap())
    );
}

#[test]
fn stale_goal_assessment_is_visible_as_pending_and_can_be_reserved_again() {
    let (_dir, mut store, candidate) = fixture();
    let input = pair_input(&store.state, &candidate).unwrap();
    let mut record = new_record(&input, "test".into(), 1);
    let reserved = store
        .reserve_assessment(&candidate, record.clone())
        .unwrap();
    record.result = Some(judgment(&input, PairVerdict::Equivalent));
    store.finish_assessment(&reserved, record).unwrap();
    store
        .save_goal(GoalInput {
            id: "goal".into(),
            subject_id: "test".into(),
            label: "New deadline".into(),
            definition: "A changed launch deadline".into(),
            receipts: vec![candidate.from_receipt.clone()],
            ..Default::default()
        })
        .unwrap();
    let saved = store.snapshot().unwrap();
    let r = &saved.relationships[0];
    assert!(r.assessment.as_ref().unwrap().stale);
    assert!(!usable(&saved, r));
    let next_input = pair_input(&saved, r).unwrap();
    assert!(store
        .reserve_assessment(r, new_record(&next_input, "test".into(), 1))
        .is_ok());
}

#[test]
fn matching_passages_keep_their_different_surrounding_context() {
    let (_dir, mut store, _) = fixture();
    let phrase = "I choose the faster option.";
    let texts = [
        format!("For emergency surgery.\n\n{phrase}"),
        format!("For a routine product launch.\n\n{phrase}"),
    ];
    let inputs = store
        .state
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| SourceInput {
            text: texts[i].clone(),
            ..s.input.clone()
        })
        .collect();
    let state = store.reconcile_sources(inputs).unwrap();
    let sources: Vec<_> = state.sources.iter().filter(|s| !s.deleted).collect();
    let receipt = |s: &SourceRecord| Receipt {
        source_id: s.input.id.clone(),
        source_revision: s.revision,
        start: s.input.text.find(phrase).unwrap(),
        end: s.input.text.len(),
        quote: phrase.into(),
        locator: s.input.title.clone(),
    };
    let r = Relationship {
        id: "pair".into(),
        subject_id: "test".into(),
        from_id: "a".into(),
        to_id: "b".into(),
        from_receipt: receipt(sources[0]),
        to_receipt: receipt(sources[1]),
        explanation: "Candidate".into(),
        ..Default::default()
    };
    let input = pair_input(&state, &r).unwrap();
    assert!(input["left"]["adjacent_context"][0]["quote"]
        .as_str()
        .unwrap()
        .contains("emergency surgery"));
    assert!(input["right"]["adjacent_context"][0]["quote"]
        .as_str()
        .unwrap()
        .contains("routine product launch"));
    for side in ["left", "right"] {
        for value in input[side]["adjacent_context"].as_array().unwrap() {
            validate_receipt(
                &state,
                &serde_json::from_value::<Receipt>(value.clone()).unwrap(),
                true,
            )
            .unwrap();
        }
    }
}

#[test]
fn confirmed_link_needs_reaffirmation_after_goal_change_and_omits_old_model_envelope() {
    let (_dir, mut store, candidate) = fixture();
    let mut goal = GoalInput {
        id: "goal".into(),
        subject_id: "test".into(),
        label: "Goal".into(),
        definition: "An original goal definition".into(),
        receipts: vec![candidate.from_receipt.clone()],
        ..Default::default()
    };
    store.save_goal(goal.clone()).unwrap();
    let input = pair_input(&store.state, &candidate).unwrap();
    let mut record = new_record(&input, "test".into(), 1);
    let reserved = store
        .reserve_assessment(&candidate, record.clone())
        .unwrap();
    record.result = Some(judgment(&input, PairVerdict::Equivalent));
    store.finish_assessment(&reserved, record).unwrap();
    store
        .review_relationship(&candidate.id, ReviewStatus::Confirmed, None)
        .unwrap();
    assert_eq!(
        store
            .context_packet(ContextRequest::default())
            .unwrap()
            .relationships
            .len(),
        1
    );
    goal.review_status = ReviewStatus::Rejected;
    store.save_goal(goal).unwrap();
    let state = store.snapshot().unwrap();
    assert!(state.relationships[0].review_stale);
    assert_eq!(
        state.relationships[0].review_status,
        ReviewStatus::Confirmed
    );
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .relationships
        .is_empty());
    store
        .review_relationship(
            &candidate.id,
            ReviewStatus::Confirmed,
            Some(RelationshipKind::Related),
        )
        .unwrap();
    let packet = store.context_packet(ContextRequest::default()).unwrap();
    assert_eq!(packet.relationships.len(), 1);
    assert!(packet.relationships[0].assessment.is_none());
    assert!(!serde_json::to_string(&packet)
        .unwrap()
        .contains("original goal definition"));
    assert!(store.snapshot().unwrap().relationships[0]
        .assessment
        .is_some());
}

#[test]
fn frozen_context_uses_its_cutoff_for_assessment_goal_revisions() {
    let (_dir, mut store, candidate) = fixture();
    let goal = GoalInput {
        id: "goal".into(),
        subject_id: "test".into(),
        label: "Goal".into(),
        definition: "Goal definition".into(),
        effective_at: Some("2020-01-01T00:00:00Z".into()),
        receipts: vec![candidate.from_receipt.clone()],
        ..Default::default()
    };
    store.save_goal(goal.clone()).unwrap();
    let mut change = goal;
    change.definition = "Different future goal".into();
    change.effective_at = Some("2030-01-01T00:00:00Z".into());
    store.save_goal(change).unwrap();
    let mut state = store.state.clone();
    for source in &mut state.sources {
        source.recorded_at = "2019-01-01T00:00:00Z".into();
    }
    for g in &mut state.goals {
        g.recorded_at = "2019-01-01T00:00:00Z".into();
    }
    let mut r = candidate;
    r.recorded_at = "2021-01-01T00:00:00Z".into();
    let input = pair_input_at(&state, &r, Some("2025-01-01T00:00:00Z")).unwrap();
    let mut record = new_record(&input, "test".into(), 1);
    record.result = Some(judgment(&input, PairVerdict::Equivalent));
    r.assessment = Some(record);
    assert!(usable_at(&state, &r, Some("2025-01-01T00:00:00Z")));
    assert!(!usable_at(&state, &r, Some("2031-01-01T00:00:00Z")));
    state.relationships = vec![r];
    assert_eq!(
        context_from_snapshot(
            &state,
            ContextRequest {
                as_of: Some("2025-01-01T00:00:00Z".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .relationships
        .len(),
        1
    );
}
