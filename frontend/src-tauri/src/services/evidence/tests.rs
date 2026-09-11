use super::*;

fn store() -> (tempfile::TempDir, EvidenceStore) {
    let dir = tempfile::tempdir().unwrap();
    let store =
        EvidenceStore::new(dir.path().to_path_buf(), "bryan".into(), "Bryan".into()).unwrap();
    (dir, store)
}

fn draft() -> InterviewDraft {
    InterviewDraft {
        id: "project-choice".into(),
        subject_id: "bryan".into(),
        subject_name: "Bryan".into(),
        situation: "Release a feature under time pressure".into(),
        wanted: "Quality".into(),
        expected: "Ship Friday".into(),
        chosen: "Delay the release".into(),
        actual: "Shipped Monday".into(),
        rejected: vec!["Ship a broken feature".into()],
        rationale: "Protect customer trust".into(),
        ..Default::default()
    }
}

#[test]
fn draft_survives_restart_and_submit_keeps_desire_expectation_choice_outcome_distinct() {
    let (dir, mut store) = store();
    store.save_interview(draft(), false).unwrap();
    let mut resumed =
        EvidenceStore::new(dir.path().to_path_buf(), "bryan".into(), "Bryan".into()).unwrap();
    assert!(resumed.snapshot().unwrap().cases.is_empty());
    let saved = resumed.snapshot().unwrap().interview_draft.unwrap();
    let submitted = resumed.save_interview(saved.clone(), true).unwrap();
    assert_eq!(submitted.cases.len(), 1);
    let case = &submitted.cases[0];
    assert_eq!(case.wanted, "Quality");
    assert_eq!(case.expected, "Ship Friday");
    assert_eq!(case.chosen, "Delay the release");
    assert_eq!(case.actual, "Shipped Monday");
    assert_eq!(case.review_status, ReviewStatus::Tentative);
    assert!(submitted.interview_draft.is_none());
    assert_eq!(resumed.save_interview(saved, true).unwrap().cases.len(), 1);
    assert_eq!(
        resumed
            .reconcile_sources(vec![])
            .unwrap()
            .cases
            .iter()
            .filter(|c| !c.invalidated)
            .count(),
        1
    );
}

#[test]
fn incomplete_submit_and_wrong_subject_cannot_create_cases() {
    let (_, mut store) = store();
    let mut input = draft();
    input.chosen.clear();
    assert!(store.save_interview(input, true).is_err());
    let mut input = draft();
    input.subject_id = "other".into();
    assert!(store.save_interview(input, true).is_err());
    assert!(store.snapshot().unwrap().cases.is_empty());
}

fn source(id: &str, text: &str) -> SourceInput {
    SourceInput {
        id: id.into(),
        note_id: id.into(),
        title: id.into(),
        text: text.into(),
        subject_id: "bryan".into(),
        role: EvidenceRole::TargetStatement,
        ..Default::default()
    }
}

fn proposal(source: &SourceRecord, chosen: &str) -> ExtractionOutput {
    ExtractionOutput {
        cases: vec![DecisionCase {
            subject_id: "bryan".into(),
            situation: "Choice".into(),
            chosen: chosen.into(),
            receipts: vec![whole_receipt(source)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn exact_attribution_required_and_revision_races_fail_closed() {
    let (_dir, mut store) = store();
    let initial = store
        .reconcile_sources(vec![source("a", "Choice: wait")])
        .unwrap();
    let mut invented = proposal(&initial.sources[0], "invented choice");
    assert!(store
        .process_job(&initial.jobs[0].id, invented.clone())
        .is_err());
    invented.cases[0].chosen = "wait".into();
    invented.cases[0].receipts[0].quote = "Choice: ship".into();
    assert!(store.process_job(&initial.jobs[0].id, invented).is_err());
    let same = store
        .reconcile_sources(vec![source("a", "Choice: wait")])
        .unwrap();
    assert_eq!(same.jobs.len(), 1);
    store
        .reconcile_sources(vec![source("a", "Choice: ship")])
        .unwrap();
    assert!(store
        .process_job(&initial.jobs[0].id, proposal(&initial.sources[0], "wait"))
        .is_err());
    assert!(store.snapshot().unwrap().cases.is_empty());

    let mut other = source("b", "Choice: wait");
    other.role = EvidenceRole::ModelOutput;
    let snapshot = store.reconcile_sources(vec![other]).unwrap();
    let job = snapshot.jobs.last().unwrap();
    let src = snapshot.sources.last().unwrap();
    assert!(store.process_job(&job.id, proposal(src, "wait")).is_err());
}

#[test]
fn job_restart_is_explicit_once_and_in_flight_is_bounded() {
    let (dir, mut store) = store();
    let initial = store
        .reconcile_sources(vec![
            source("a", "Choice: wait"),
            source("b", "Choice: ship"),
        ])
        .unwrap();
    store.start_job(&initial.jobs[0].id).unwrap();
    assert!(store.start_job(&initial.jobs[1].id).is_err());
    let mut resumed =
        EvidenceStore::new(dir.path().to_path_buf(), "bryan".into(), "Bryan".into()).unwrap();
    assert_eq!(
        resumed.snapshot().unwrap().jobs[0].status,
        JobStatus::Processing
    );
    resumed.recover_jobs().unwrap();
    assert_eq!(
        resumed.snapshot().unwrap().jobs[0].status,
        JobStatus::Queued
    );
    resumed.start_job(&initial.jobs[0].id).unwrap();
    resumed
        .process_job(&initial.jobs[0].id, proposal(&initial.sources[0], "wait"))
        .unwrap();
    let again = resumed
        .process_job(&initial.jobs[0].id, proposal(&initial.sources[0], "wait"))
        .unwrap();
    assert_eq!(again.cases.len(), 1);
}

#[test]
fn usable_tentative_cases_are_excluded_after_restriction_and_holdout_group() {
    let (_dir, mut store) = store();
    let initial = store
        .reconcile_sources(vec![source("a", "Choice: wait")])
        .unwrap();
    store
        .process_job(&initial.jobs[0].id, proposal(&initial.sources[0], "wait"))
        .unwrap();
    let packet = store.context_packet(ContextRequest::default()).unwrap();
    assert_eq!(packet.cases.len(), 1);
    assert_eq!(packet.cases[0].review_status, ReviewStatus::Tentative);
    assert!(!packet.source_revisions.is_empty());
    let excluded = ContextRequest {
        excluded_source_groups: vec![initial.sources[0].input.source_group.clone()],
        ..Default::default()
    };
    assert!(store.context_packet(excluded).unwrap().cases.is_empty());
    let mut restricted = source("a", "Choice: wait");
    restricted.restricted = true;
    store.reconcile_sources(vec![restricted]).unwrap();
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .cases
        .is_empty());
}

#[test]
fn concurrent_goals_retain_revisions_and_unknown_targets() {
    let (_dir, mut store) = store();
    let input = GoalInput {
        id: "reach".into(),
        subject_id: "bryan".into(),
        label: "Reach".into(),
        definition: "Many people quickly".into(),
        criteria: vec![GoalCriterion::default()],
        effective_at: Some("2025-01-01".into()),
        ..Default::default()
    };
    let first = store.save_goal(input.clone()).unwrap();
    store
        .save_goal(GoalInput {
            id: "quality".into(),
            subject_id: "bryan".into(),
            label: "Quality".into(),
            definition: "Keep trust".into(),
            ..Default::default()
        })
        .unwrap();
    let second = store
        .save_goal(GoalInput {
            status: GoalStatus::Paused,
            effective_at: Some("2025-03-01".into()),
            ..input
        })
        .unwrap();
    assert_eq!(second.revision, first.revision + 1);
    let all = store.snapshot().unwrap();
    assert_eq!(all.goals.len(), 3);
    assert!(all.goals[0].input.criteria[0].target.is_none());
    assert!(all.goals[0].input.criteria[0].deadline.is_none());
    let history = latest_goals(&all, Some(&first.recorded_at));
    assert_eq!(
        history
            .iter()
            .find(|g| g.input.id == "reach")
            .unwrap()
            .revision,
        first.revision
    );
    assert!(store
        .save_goal(GoalInput {
            subject_id: "someone_else".into(),
            label: "Reach".into(),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn conflicting_answers_stay_separate_and_unusable_until_resolved() {
    let (_dir, mut store) = store();
    let snapshot = store
        .reconcile_sources(vec![
            source("a", "Choice: wait"),
            source("b", "Choice: ship"),
        ])
        .unwrap();
    store
        .process_job(&snapshot.jobs[0].id, proposal(&snapshot.sources[0], "wait"))
        .unwrap();
    store
        .process_job(&snapshot.jobs[1].id, proposal(&snapshot.sources[1], "ship"))
        .unwrap();
    assert_eq!(store.snapshot().unwrap().cases.len(), 2);
    assert!(store
        .snapshot()
        .unwrap()
        .cases
        .iter()
        .all(|case| case.conflict));
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .cases
        .is_empty());
}

#[test]
fn causal_review_cannot_upgrade_belief_and_both_endpoint_receipts_are_required() {
    let (_dir, mut store) = store();
    let snapshot = store
        .reconcile_sources(vec![
            source("a", "Review protects customer trust"),
            source("b", "Customer trust enables growth"),
        ])
        .unwrap();
    let relation = Relationship {
        subject_id: "bryan".into(),
        from_id: "a".into(),
        to_id: "b".into(),
        relation: RelationshipKind::Enables,
        directed: true,
        explanation: "A stated belief about trust".into(),
        causal_basis: Some("target_stated_belief".into()),
        from_receipt: whole_receipt(&snapshot.sources[0]),
        to_receipt: whole_receipt(&snapshot.sources[1]),
        ..Default::default()
    };
    let mut forged = relation.clone();
    forged.to_receipt.quote = "invented".into();
    assert!(store
        .process_job(
            &snapshot.jobs[0].id,
            ExtractionOutput {
                relationships: vec![forged],
                ..Default::default()
            }
        )
        .is_err());
    let saved = store
        .process_job(
            &snapshot.jobs[0].id,
            ExtractionOutput {
                relationships: vec![relation],
                ..Default::default()
            },
        )
        .unwrap();
    let reviewed = store
        .review_relationship(&saved.relationships[0].id, ReviewStatus::Confirmed, None)
        .unwrap();
    assert_eq!(
        reviewed.causal_basis.as_deref(),
        Some("target_stated_belief")
    );
    store
        .reconcile_sources(vec![
            source("a", "Review protects trust differently"),
            source("b", "Customer trust enables growth"),
        ])
        .unwrap();
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .relationships
        .is_empty());
}

#[test]
fn automatic_goal_cannot_invent_quantity_from_qualitative_receipt() {
    let (_dir, mut store) = store();
    let snapshot = store
        .reconcile_sources(vec![source("a", "I want to reach many people quickly")])
        .unwrap();
    let goal = GoalInput {
        subject_id: "bryan".into(),
        label: "Reach".into(),
        definition: "Reach many people".into(),
        receipts: vec![whole_receipt(&snapshot.sources[0])],
        criteria: vec![GoalCriterion {
            target: Some(1000.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(store
        .process_job(
            &snapshot.jobs[0].id,
            ExtractionOutput {
                goals: vec![goal],
                ..Default::default()
            }
        )
        .is_err());
}

#[test]
fn heldout_copies_with_different_group_labels_cannot_leak() {
    let (_dir, mut store) = store();
    let mut visible = source("a", "Choice: wait");
    visible.source_group = "visible".into();
    let first = store.reconcile_sources(vec![visible.clone()]).unwrap();
    store
        .process_job(&first.jobs[0].id, proposal(&first.sources[0], "wait"))
        .unwrap();
    let mut heldout = source("b", "Choice: wait");
    heldout.source_group = "heldout".into();
    heldout.held_out = true;
    store.reconcile_sources(vec![visible, heldout]).unwrap();
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .cases
        .is_empty());
}

#[test]
fn interview_expected_effect_needs_explicit_bridge_before_second_order_goal_path() {
    let (_dir, mut store) = store();
    let mut input = draft();
    let first = store.save_interview(input.clone(), true).unwrap();
    assert_eq!(first.nodes.iter().filter(|n| !n.invalidated).count(), 2);
    assert_eq!(
        first
            .relationships
            .iter()
            .filter(|r| !r.invalidated)
            .count(),
        1
    );
    assert_eq!(first.goals.len(), 1);
    assert_eq!(first.goals[0].input.definition, "Quality");
    assert_eq!(
        first.cases[0].goal_revisions[0].goal_id,
        first.goals[0].input.id
    );
    input.expected_goal_relation = Some(RelationshipKind::Inhibits);
    let updated = store.save_interview(input, true).unwrap();
    let relations: Vec<_> = updated
        .relationships
        .iter()
        .filter(|r| !r.invalidated)
        .collect();
    assert_eq!(relations.len(), 2);
    let action = updated
        .nodes
        .iter()
        .find(|n| !n.invalidated && n.kind == EvidenceNodeKind::Action)
        .unwrap();
    let first_order = relations.iter().find(|r| r.from_id == action.id).unwrap();
    let second_order = relations
        .iter()
        .find(|r| r.from_id == first_order.to_id)
        .unwrap();
    assert_eq!(second_order.relation, RelationshipKind::Inhibits);
    assert_eq!(
        second_order.causal_basis.as_deref(),
        Some("target_stated_belief")
    );
    assert!(relations.iter().all(|r| r.similarity.is_none()));
    assert_eq!(
        store
            .context_packet(ContextRequest::default())
            .unwrap()
            .nodes
            .len(),
        2
    );
}

#[test]
fn personal_statement_without_choice_is_tentative_context_and_revocable() {
    let (_dir, mut store) = store();
    let snapshot = store
        .reconcile_sources(vec![source(
            "a",
            "I prefer careful review when customer trust is at risk",
        )])
        .unwrap();
    let statement = PersonalEvidence {
        subject_id: "bryan".into(),
        statement: "Careful review matters when customer trust is at risk".into(),
        kind: "preference".into(),
        receipts: vec![whole_receipt(&snapshot.sources[0])],
        ..Default::default()
    };
    store
        .process_job(
            &snapshot.jobs[0].id,
            ExtractionOutput {
                statements: vec![statement],
                ..Default::default()
            },
        )
        .unwrap();
    let packet = store.context_packet(ContextRequest::default()).unwrap();
    assert_eq!(packet.statements.len(), 1);
    assert!(packet.cases.is_empty());
    assert!(!packet.source_revisions.is_empty());
    assert_eq!(packet.statements[0].review_status, ReviewStatus::Tentative);
    store.reconcile_sources(vec![]).unwrap();
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .statements
        .is_empty());
}

#[test]
fn metadata_rewrite_is_noop_but_structured_answer_change_invalidates_job() {
    let (_dir, mut store) = store();
    let mut input = source("a", "Choice: wait");
    let first = store.reconcile_sources(vec![input.clone()]).unwrap();
    input.observed_at = Some("2026-09-05T12:00:00Z".into());
    assert_eq!(
        store
            .reconcile_sources(vec![input.clone()])
            .unwrap()
            .jobs
            .len(),
        1
    );
    input.structured_case = Some(serde_json::json!({"target_answer":"wait"}));
    let changed = store.reconcile_sources(vec![input]).unwrap();
    assert_eq!(changed.jobs.len(), 2);
    assert_ne!(
        changed.sources.last().unwrap().content_hash,
        first.sources[0].content_hash
    );
}

#[test]
fn personal_ablation_removes_goal_only_quotes_and_unresolved_fields() {
    let packet = ContextPacket {
        goals: vec![GoalRevision {
            input: GoalInput {
                label: "HIDDEN_GOAL".into(),
                receipts: vec![Receipt {
                    quote: "HIDDEN_GOAL_QUOTE".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            revision: 1,
            recorded_at: now(),
            invalidated: false,
        }],
        source_revisions: vec![Receipt {
            quote: "HIDDEN_GOAL_QUOTE".into(),
            ..Default::default()
        }],
        unresolved: vec!["HIDDEN_GOAL_UNRESOLVED".into()],
        ..Default::default()
    };
    assert!(!serde_json::to_string(&without_goal_paths(&packet))
        .unwrap()
        .contains("HIDDEN_GOAL"));
}

#[test]
fn future_effective_goal_does_not_replace_current_goal_early() {
    let (_dir, mut store) = store();
    let input = GoalInput {
        id: "quality".into(),
        subject_id: "bryan".into(),
        label: "Quality".into(),
        definition: "Current quality".into(),
        ..Default::default()
    };
    store.save_goal(input.clone()).unwrap();
    store
        .save_goal(GoalInput {
            definition: "Future quality".into(),
            effective_at: Some("2099-01-01".into()),
            ..input
        })
        .unwrap();
    assert_eq!(
        store
            .context_packet(ContextRequest::default())
            .unwrap()
            .goals[0]
            .input
            .definition,
        "Current quality"
    );
}

#[test]
fn rejected_statement_cannot_return_as_a_same_receipt_paraphrase() {
    let (_dir, mut store) = store();
    let snapshot = store
        .reconcile_sources(vec![source(
            "a",
            "I prefer careful review when customer trust is at risk",
        )])
        .unwrap();
    let statement = PersonalEvidence {
        subject_id: "bryan".into(),
        statement: "Careful review protects customer trust".into(),
        kind: "preference".into(),
        receipts: vec![whole_receipt(&snapshot.sources[0])],
        ..Default::default()
    };
    let output = ExtractionOutput {
        statements: vec![statement.clone()],
        needs_review: vec!["One other ambiguous passage remains".into()],
        ..Default::default()
    };
    let saved = store.process_job(&snapshot.jobs[0].id, output).unwrap();
    let id = saved.statements[0].id.clone();
    let reviewed = store.review_statement(&id, ReviewStatus::Rejected).unwrap();
    assert_eq!(reviewed.receipts, saved.statements[0].receipts);
    let paraphrase = PersonalEvidence {
        statement: "Trust deserves a careful review".into(),
        ..statement
    };
    let retried = store
        .process_job(
            &snapshot.jobs[0].id,
            ExtractionOutput {
                statements: vec![paraphrase],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(retried.statements.len(), 1);
    assert!(store
        .context_packet(ContextRequest::default())
        .unwrap()
        .statements
        .is_empty());
    store
        .review_statement(&id, ReviewStatus::Confirmed)
        .unwrap();
    assert_eq!(
        store
            .context_packet(ContextRequest::default())
            .unwrap()
            .statements
            .len(),
        1
    );
}

#[test]
fn unassigned_speaker_similarity_is_browseable_but_not_personal_context() {
    let (_dir, mut store) = store();
    let mut unknown = source("b", "An unknown speaker discusses customer trust");
    unknown.role = EvidenceRole::Unknown;
    let snapshot = store
        .reconcile_sources(vec![source("a", "I protect customer trust"), unknown])
        .unwrap();
    let relation = Relationship {
        id: "browseable-related".into(),
        subject_id: "bryan".into(),
        from_id: "a".into(),
        to_id: "b".into(),
        explanation: "Similar topic only".into(),
        from_receipt: whole_receipt(&snapshot.sources[0]),
        to_receipt: whole_receipt(&snapshot.sources[1]),
        recorded_at: now(),
        ..Default::default()
    };
    let saved = store
        .apply_discovery(DiscoveryOutput {
            embedding_version: None,
            relationships: vec![relation],
            embedding_status: "ready".into(),
        })
        .unwrap();
    assert_eq!(saved.relationships.len(), 1);
    let packet = store.context_packet(ContextRequest::default()).unwrap();
    assert!(packet.relationships.is_empty());
    assert!(packet.source_revisions.is_empty());
}
