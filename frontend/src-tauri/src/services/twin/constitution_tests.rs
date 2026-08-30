    use super::*;
    use crate::models::note::{Note, NoteStatus};
    use crate::models::twin::{PromotionState, UserRecordKind};
    use tempfile::tempdir;

    use crate::models::twin::ConstitutionStatus;

    #[test]
    fn coordinated_constitution_and_action_gap_mutations_use_observations_then_feedback() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator);
        let item = store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Prefer reversible experiments".to_string(),
                dimension: "values".to_string(),
                scope: vec!["work".to_string()],
                priority: 0.8,
                confidence: 0.8,
                status: ConstitutionStatus::Candidate,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: None,
            })
            .unwrap();
        store
            .update_constitution_item(
                &item.id,
                ConstitutionItemUpdate {
                    priority: Some(0.9),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .review_constitution_item(
                &item.id,
                ConstitutionReviewRequest {
                    action: MemoryDigestAction::Keep,
                    rationale: Some("Reviewed".to_string()),
                },
            )
            .unwrap();
        let gap = store
            .create_action_gap(ActionGapCreate {
                stated_value: "Move quickly".to_string(),
                revealed_behavior: "Wait for evidence".to_string(),
                driver_hypothesis: None,
                somatic_taste_signal: None,
                decision_risk: "Delay".to_string(),
                evidence_refs: Vec::new(),
                linked_record_ids: Vec::new(),
                confidence: 0.7,
                status: ConstitutionStatus::Candidate,
            })
            .unwrap();
        store
            .review_action_gap(
                &gap.id,
                ConstitutionReviewRequest {
                    action: MemoryDigestAction::Reject,
                    rationale: None,
                },
            )
            .unwrap();
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 5);
        assert!(matches!(
            captured[0].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
        assert!(matches!(
            captured[1].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
        assert!(matches!(
            captured[2].payload,
            crate::models::twin_event::TwinEventPayload::FeedbackRecorded(_)
        ));
        assert!(matches!(
            captured[3].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
        assert!(matches!(
            captured[4].payload,
            crate::models::twin_event::TwinEventPayload::FeedbackRecorded(_)
        ));
        assert!(captured.iter().all(|event| !matches!(
            event.payload,
            crate::models::twin_event::TwinEventPayload::MemoryProposed(_)
                | crate::models::twin_event::TwinEventPayload::MemoryReviewed(_)
        )));
    }
    #[test]
    fn constitution_review_and_trace_recover_as_one_compound_mutation() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store =
            TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator.clone());
        let item = store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Recover the review".to_string(),
                dimension: "values".to_string(),
                scope: Vec::new(),
                priority: 0.8,
                confidence: 0.8,
                status: ConstitutionStatus::Candidate,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: None,
            })
            .unwrap();
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));

        let (_, commit) = store
            .review_constitution_item_with_commit(
                &item.id,
                ConstitutionReviewRequest {
                    action: MemoryDigestAction::Keep,
                    rationale: Some("Reviewed".to_string()),
                },
            )
            .unwrap();
        assert!(commit.postcommit_warning);
        assert!(!store.trace_cache.contains_key("constitution-review"));
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        assert_eq!(events.ordered_events().unwrap().len(), 2);
        assert_eq!(
            store
                .get_session_trace("constitution-review")
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[test]
    fn legacy_auto_promoted_cannot_seed_or_activate_constitution() {
        let now = Utc::now();
        let record = UserRecord {
            id: "legacy-auto".to_string(),
            kind: UserRecordKind::Preference,
            content: "legacy constitutional claim".to_string(),
            evidence_refs: Vec::new(),
            confidence: 1.0,
            origin: RecordOrigin::Inferred,
            promotion_state: PromotionState::AutoPromoted,
            created_at: now,
            updated_at: now,
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        };
        assert!(!constitution_allows_record(&record));
        assert_eq!(
            constitution_status_from_record(&record),
            ConstitutionStatus::Candidate
        );
    }

    #[test]
    fn materialized_legacy_artifacts_are_overlaid_without_rewriting_disk() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let now = Utc::now();
        let legacy = UserRecord {
            id: "legacy-auto".to_string(),
            kind: UserRecordKind::Preference,
            content: "legacy constitutional authority".to_string(),
            evidence_refs: Vec::new(),
            confidence: 1.0,
            origin: RecordOrigin::Inferred,
            promotion_state: PromotionState::AutoPromoted,
            created_at: now,
            updated_at: now,
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        };
        assert!(store
            .write_pretty_json(&store.records_path.join("legacy-auto.json"), &legacy)
            .unwrap()
            .authority_token
            .is_none());
        let item = store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Legacy-only active constitution".to_string(),
                dimension: "values".to_string(),
                scope: Vec::new(),
                priority: 1.0,
                confidence: 1.0,
                status: ConstitutionStatus::Active,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: vec![legacy.id.clone()],
                source: Some("legacy_materialized".to_string()),
            })
            .unwrap();
        let gap = store
            .create_action_gap(ActionGapCreate {
                stated_value: "Act quickly".to_string(),
                revealed_behavior: "Waited".to_string(),
                driver_hypothesis: None,
                somatic_taste_signal: None,
                decision_risk: "Legacy-only action gap".to_string(),
                evidence_refs: Vec::new(),
                linked_record_ids: vec![legacy.id.clone()],
                confidence: 1.0,
                status: ConstitutionStatus::Active,
            })
            .unwrap();

        let listed_item = store
            .list_constitution_items()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == item.id)
            .unwrap();
        let listed_gap = store
            .list_action_gaps()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == gap.id)
            .unwrap();
        assert_eq!(listed_item.status, ConstitutionStatus::Candidate);
        assert_eq!(listed_gap.status, ConstitutionStatus::Candidate);
        let (selected_items, selected_gaps) = store
            .select_constitution_context("Legacy-only active constitution action gap")
            .unwrap();
        assert!(selected_items
            .iter()
            .all(|candidate| candidate.id != item.id));
        assert!(selected_gaps.iter().all(|candidate| candidate.id != gap.id));

        assert_eq!(
            store
                .read_constitution_file(&store.constitution_file_path(&item.id))
                .unwrap()
                .status,
            ConstitutionStatus::Active
        );
        assert_eq!(
            store
                .read_action_gap_file(&store.action_gap_file_path(&gap.id))
                .unwrap()
                .status,
            ConstitutionStatus::Active
        );

        let endorsed = UserRecord {
            id: "endorsed-record".to_string(),
            promotion_state: PromotionState::Endorsed,
            content: "independent approved support".to_string(),
            ..legacy.clone()
        };
        assert!(store
            .write_pretty_json(&store.records_path.join("endorsed-record.json"), &endorsed)
            .unwrap()
            .authority_token
            .is_none());
        let preserved = store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Approved independent constitution".to_string(),
                dimension: "values".to_string(),
                scope: Vec::new(),
                priority: 1.0,
                confidence: 1.0,
                status: ConstitutionStatus::Active,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: vec![legacy.id, endorsed.id],
                source: None,
            })
            .unwrap();
        assert_eq!(
            store
                .list_constitution_items()
                .unwrap()
                .into_iter()
                .find(|candidate| candidate.id == preserved.id)
                .unwrap()
                .status,
            ConstitutionStatus::Active
        );
        assert!(store
            .select_constitution_context("Approved independent constitution")
            .unwrap()
            .0
            .iter()
            .any(|candidate| candidate.id == preserved.id));
    }

    fn test_note(id: &str, title: &str, content: &str, source_type: Option<&str>) -> Note {
        let now = Utc::now();
        let mut properties = HashMap::new();
        if let Some(source_type) = source_type {
            properties.insert(
                "source_type".to_string(),
                Value::String(source_type.to_string()),
            );
        }
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties,
            ..Default::default()
        }
    }

    #[test]
    fn constitution_inference_keeps_repeated_behavior_pending_review() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for index in 0..3 {
            store
                .append_trace_event(
                    "session-1",
                    TraceEventType::PromptSubmitted,
                    serde_json::json!({
                        "tile_id": format!("tile-{}", index),
                        "prompt": "Please implement this with exact files, commands, and tests.",
                        "models": ["openai/gpt-4o"],
                    }),
                )
                .expect("trace event should append");
        }

        store
            .run_twin_inference()
            .expect("record inference should run");
        let summary = store
            .run_constitution_inference_with_notes(&[])
            .expect("constitution inference should run");
        let items = store
            .list_constitution_items()
            .expect("constitution should list");
        let item = items
            .iter()
            .find(|item| item.claim.contains("concrete implementation details"))
            .expect("behavior-derived constitution item should exist");

        assert_eq!(summary.scanned_behavior_events, 3);
        assert_eq!(summary.auto_active_items, 0);
        assert_eq!(summary.review_candidate_items, 1);
        assert_eq!(item.status, ConstitutionStatus::Candidate);
        assert_eq!(item.source.as_deref(), Some("behavior_inference"));
        assert!(item
            .evidence_refs
            .iter()
            .all(|evidence| evidence.source_type.as_deref() == Some("behavior")));
    }

    #[test]
    fn interviewee_answers_become_research_findings_not_personal_constitution() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-1",
            "Interview: onboarding",
            "### Message 1: User\n\nHow do you decide whether an AI workflow is useful?\n\n### Message 2: Interviewee\n\nI need to see a working demo before I trust the system.\n\n### Message 3: User\n\nCan you give a concrete example and compare it with your current workflow?",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let items = store
            .list_constitution_items()
            .expect("constitution should list");
        let records = store.list_user_records().expect("records should list");

        assert_eq!(summary.scanned_interviews, 1);
        assert_eq!(summary.extracted_research_findings, 1);
        assert!(items
            .iter()
            .any(|item| item.claim.contains("concrete examples")));
        assert!(!items
            .iter()
            .any(|item| item.claim.contains("working demo before I trust")));
        assert!(records.iter().any(|record| {
            record.kind == UserRecordKind::Fact
                && record.content.contains("working demo before I trust")
                && record.metadata.get("source_type").and_then(Value::as_str)
                    == Some("interview_answer")
        }));
    }

    #[test]
    fn unlabeled_interview_notes_import_but_do_not_extract() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-2",
            "Unlabeled interview",
            "How do you decide whether an AI workflow is useful?\nI need a working demo.",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");

        assert_eq!(summary.scanned_interviews, 1);
        assert_eq!(summary.extracted_research_findings, 0);
        assert_eq!(summary.skipped_domain_claims, 1);
        assert!(store
            .list_constitution_items()
            .expect("constitution should list")
            .is_empty());
    }

    #[test]
    fn constitution_inference_prunes_stale_vault_derived_items_and_records() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Old note-backed claim".to_string(),
                dimension: "reasoning".to_string(),
                scope: vec!["note".to_string()],
                priority: 0.58,
                confidence: 0.58,
                status: ConstitutionStatus::Candidate,
                evidence_refs: vec![EvidenceRef {
                    trace_id: "note-old-note".to_string(),
                    event_id: "old-note".to_string(),
                    session_id: "vault-note".to_string(),
                    tile_id: None,
                    model_id: None,
                    note: Some("Old note".to_string()),
                    source_type: Some("note".to_string()),
                    source_id: Some("old-note".to_string()),
                    source_label: Some("Vault note".to_string()),
                    excerpt: Some("old evidence".to_string()),
                    speaker_role: Some("user".to_string()),
                }],
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: Some("note_inference".to_string()),
            })
            .expect("old constitution item should be created");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Fact,
                content: "Interview finding: old vault finding".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.66,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::from([
                    ("source_type".to_string(), json!("interview_answer")),
                    ("source_note_id".to_string(), json!("old-note")),
                ]),
            })
            .expect("old record should be created");
        store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Old guided setup claim".to_string(),
                dimension: "values".to_string(),
                scope: vec!["setup".to_string()],
                priority: 0.9,
                confidence: 0.9,
                status: ConstitutionStatus::Active,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: Some("guided_setup".to_string()),
            })
            .expect("old setup constitution item should be created");

        let current_notes = vec![test_note(
            "current-interview",
            "Current interview",
            "### Message 1: User\n\nCan you give a concrete example of how you make tradeoffs?\n\n### Message 2: Interviewee\n\nI compare impact and risk before deciding.",
            Some("interview"),
        )];
        let summary = store
            .run_constitution_inference_with_notes(&current_notes)
            .expect("constitution inference should run");

        assert_eq!(summary.pruned_stale_constitution_items, 2);
        assert_eq!(summary.pruned_stale_records, 1);
        assert!(store
            .list_constitution_items()
            .expect("constitution should list")
            .iter()
            .all(|item| !item.claim.contains("Old note-backed claim")
                && !item.claim.contains("Old guided setup claim")));
        assert!(store
            .list_user_records()
            .expect("records should list")
            .iter()
            .all(|record| !record.content.contains("old vault finding")));
    }

    #[test]
    fn coordinated_prune_uses_tombstone_and_legacy_pruned_observation() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator);
        let stale = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Fact,
                content: "Stale note-derived record".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.66,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::from([
                    ("source_type".to_string(), json!("interview_answer")),
                    ("source_note_id".to_string(), json!("missing-note")),
                ]),
            })
            .unwrap();

        let summary = store.run_constitution_inference_with_notes(&[]).unwrap();
        assert_eq!(summary.pruned_stale_records, 1);
        assert!(!store.record_file_path(&stale.id).exists());
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(captured[1]
            .context
            .tags
            .iter()
            .any(|tag| tag == "legacy_pruned"));
        assert!(matches!(
            captured[1].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
    }

    #[test]
    fn constitution_inference_rewrites_setup_from_current_interview_questions() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-setup",
            "Interview: strategy",
            "### Message 1: User\n\nWhat are some personal values or cultural values that drive your decisions?\n\n### Message 2: Interviewee\n\nI want work to make something better and different.\n\n### Message 3: User\n\nCan you walk us through a concrete example and how you balance innovation and stability?",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let setup = store
            .get_constitution_setup()
            .expect("setup should load after inference");

        assert!(summary.updated_setup_entries >= 4);
        assert!(setup
            .values
            .iter()
            .any(|entry| entry.contains("values and cultural assumptions")));
        assert!(setup
            .tastes
            .iter()
            .any(|entry| entry.contains("concrete walkthroughs")));
        assert!(setup
            .constraints
            .iter()
            .any(|entry| entry.contains("interviewee answers as research evidence")));
        assert!(setup
            .action_tendencies
            .iter()
            .any(|entry| entry.contains("follow-up questions")));
    }

    #[test]
    fn constitution_setup_accepts_legacy_json_without_identity() {
        let setup: ConstitutionSetup = serde_json::from_str(
            r#"{
                "values": ["evidence-backed work"],
                "tastes": ["clean UX"],
                "constraints": [],
                "somatic_cues": [],
                "action_tendencies": []
            }"#,
        )
        .expect("legacy setup should parse");

        assert_eq!(setup.twin_name, None);
        assert_eq!(setup.twin_role, None);
        assert!(setup.source_boundaries.is_empty());
        assert_eq!(setup.values, vec!["evidence-backed work"]);
    }

    #[test]
    fn save_constitution_setup_trims_identity_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let saved = store
            .save_constitution_setup(ConstitutionSetup {
                twin_name: Some("  Alex Chen  ".to_string()),
                twin_role: Some("  founder deciding from product evidence  ".to_string()),
                source_boundaries: vec![
                    "  Use reviewed notes only.  ".to_string(),
                    "".to_string(),
                    " Uploaded interviews define domain context. ".to_string(),
                ],
                values: vec![" evidence-backed work ".to_string()],
                ..ConstitutionSetup::default()
            })
            .expect("setup should save");

        assert_eq!(saved.twin_name.as_deref(), Some("Alex Chen"));
        assert_eq!(
            saved.twin_role.as_deref(),
            Some("founder deciding from product evidence")
        );
        assert_eq!(
            saved.source_boundaries,
            vec![
                "Use reviewed notes only.".to_string(),
                "Uploaded interviews define domain context.".to_string()
            ]
        );
        assert_eq!(saved.values, vec!["evidence-backed work"]);
    }

    #[test]
    fn constitution_inference_preserves_configured_identity() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        store
            .save_constitution_setup(ConstitutionSetup {
                twin_name: Some("Alex Chen".to_string()),
                twin_role: Some("founder deciding from product evidence".to_string()),
                source_boundaries: vec!["Use reviewed notes only.".to_string()],
                values: vec!["evidence-backed work".to_string()],
                ..ConstitutionSetup::default()
            })
            .expect("identity setup should save");

        let notes = vec![test_note(
            "interview-setup",
            "Interview: strategy",
            "### Message 1: User\n\nCan you walk us through a concrete example and how you balance innovation and stability?",
            Some("interview"),
        )];
        store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let setup = store
            .get_constitution_setup()
            .expect("setup should load after inference");

        assert_eq!(setup.twin_name.as_deref(), Some("Alex Chen"));
        assert_eq!(
            setup.twin_role.as_deref(),
            Some("founder deciding from product evidence")
        );
        assert_eq!(
            setup.source_boundaries,
            vec!["Use reviewed notes only.".to_string()]
        );
    }
