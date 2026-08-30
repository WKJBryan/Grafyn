    use super::*;
    use crate::models::twin::CanvasResponseRef;
    use tempfile::tempdir;

    fn coordinated_decision_store(
        root: &std::path::Path,
    ) -> (
        TwinStore,
        std::sync::Arc<crate::services::twin_events::TwinEventStore>,
        std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    ) {
        let data = root.join("data");
        let vault = root.join("vault");
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
        (
            TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator.clone()),
            events,
            coordinator,
        )
    }
    #[test]
    fn coordinated_decision_create_and_outcome_capture_only_explicit_persisted_fields() {
        let root = tempdir().unwrap();
        let (mut store, events, _) = coordinated_decision_store(root.path());
        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-one".to_string(),
                session_id: "session-one".to_string(),
                tile_id: "tile-one".to_string(),
                decision: "Ship now?".to_string(),
                options: vec!["Wait".to_string(), "Ship".to_string()],
                stakes: Some("Release quality".to_string()),
                initial_leaning: Some("Wait".to_string()),
                review_date: Some("2026-09-30".to_string()),
                primitive_assessment: Default::default(),
                context_version: Some("context-v1".to_string()),
            })
            .unwrap();
        let create_events = events.ordered_events().unwrap();
        assert_eq!(create_events.len(), 1);
        let crate::models::twin_event::TwinEventPayload::DecisionRecorded(created) =
            &create_events[0].payload
        else {
            panic!("direct decision creation needs one typed decision event");
        };
        assert_eq!(created.decision_id.as_str(), "decision-one");
        assert_eq!(created.review_date.as_ref().unwrap().as_str(), "2026-09-30");

        store
            .update_decision_outcome_with_response_id(
                &episode.id,
                DecisionOutcomeUpdate {
                    selected_response: Some(CanvasResponseRef {
                        tile_id: "tile-one".to_string(),
                        model_id: "model-a".to_string(),
                    }),
                    chosen_option: Some("Ship".to_string()),
                    confidence: Some(0.87),
                    review_date: Some("2026-10-30".to_string()),
                    outcome: Some("Released".to_string()),
                    regret_score: Some(2),
                    lesson: Some("Stage the rollout".to_string()),
                    missed_something: Some("Support load".to_string()),
                    primitive_assessment: None,
                    correction_note: Some("Use a canary next time".to_string()),
                },
                Some("response-persisted".to_string()),
            )
            .unwrap();
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 2);
        let crate::models::twin_event::TwinEventPayload::DecisionOutcomeRecorded(outcome) =
            &captured[1].payload
        else {
            panic!("outcome mutation needs one typed follow-up event");
        };
        assert_eq!(outcome.outcome.as_ref().unwrap().as_str(), "Released");
        assert_eq!(outcome.chosen_option.as_ref().unwrap().as_str(), "Ship");
        assert_eq!(
            outcome.selected_response_id.as_ref().unwrap().as_str(),
            "response-persisted"
        );
        assert_eq!(outcome.confidence_basis_points, Some(8700));
        assert_eq!(outcome.review_date.as_ref().unwrap().as_str(), "2026-10-30");
        assert_eq!(
            outcome.correction_note.as_ref().unwrap().as_str(),
            "Use a canary next time"
        );
        assert_eq!(outcome.regret_score, Some(2));
        assert_eq!(
            outcome.lesson.as_ref().unwrap().as_str(),
            "Stage the rollout"
        );
        assert_eq!(
            outcome.missed_something.as_ref().unwrap().as_str(),
            "Support load"
        );
    }

    #[test]
    fn decision_and_trace_recover_as_one_compound_mutation() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_decision_store(root.path());
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));

        assert!(store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-crash".to_string(),
                session_id: "session-crash".to_string(),
                tile_id: "tile-crash".to_string(),
                decision: "Recover both?".to_string(),
                options: vec!["No".to_string(), "Yes".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: Default::default(),
                context_version: None,
            })
            .is_err());
        assert!(!store.trace_cache.contains_key("session-crash"));
        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        assert_eq!(events.ordered_events().unwrap().len(), 1);
        assert_eq!(
            store
                .get_decision_episode("decision-crash")
                .unwrap()
                .decision,
            "Recover both?"
        );
        let trace = store.get_session_trace("session-crash").unwrap();
        assert_eq!(trace.events.len(), 1);
        assert_eq!(
            trace.events[0].event_type,
            TraceEventType::DecisionEpisodeCreated
        );
    }

    #[test]
    fn legacy_auto_promoted_is_not_decision_evidence() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let now = Utc::now();
        let legacy = crate::models::twin::UserRecord {
            id: "legacy-auto".to_string(),
            kind: crate::models::twin::UserRecordKind::Preference,
            content: "legacy decision authority".to_string(),
            evidence_refs: Vec::new(),
            confidence: 1.0,
            origin: crate::models::twin::RecordOrigin::Inferred,
            promotion_state: PromotionState::AutoPromoted,
            created_at: now,
            updated_at: now,
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        };
        store.record_cache.insert(legacy.id.clone(), legacy);
        store.records_cache_ready = true;
        let packet = store
            .build_decision_evidence_packet(
                &[],
                &["legacy-auto".to_string()],
                &[],
                &[],
                &DecisionMirrorConfig::default(),
            )
            .unwrap();
        assert!(packet.selected_sources.is_empty());
    }

    #[test]
    fn materialized_legacy_artifacts_are_not_decision_evidence() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let now = Utc::now();
        let legacy = crate::models::twin::UserRecord {
            id: "legacy-artifact-record".to_string(),
            kind: crate::models::twin::UserRecordKind::Preference,
            content: "legacy artifact authority".to_string(),
            evidence_refs: Vec::new(),
            confidence: 1.0,
            origin: crate::models::twin::RecordOrigin::Inferred,
            promotion_state: PromotionState::AutoPromoted,
            created_at: now,
            updated_at: now,
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        };
        store
            .write_pretty_json(
                &store.records_path.join("legacy-artifact-record.json"),
                &legacy,
            )
            .unwrap();
        let item = store
            .create_constitution_item(crate::models::twin::ConstitutionItemCreate {
                claim: "legacy-only constitution evidence".to_string(),
                dimension: "values".to_string(),
                scope: Vec::new(),
                priority: 1.0,
                confidence: 1.0,
                status: ConstitutionStatus::Active,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: vec![legacy.id.clone()],
                source: None,
            })
            .unwrap();
        let gap = store
            .create_action_gap(crate::models::twin::ActionGapCreate {
                stated_value: "Move quickly".to_string(),
                revealed_behavior: "Waited".to_string(),
                driver_hypothesis: None,
                somatic_taste_signal: None,
                decision_risk: "legacy-only gap evidence".to_string(),
                evidence_refs: Vec::new(),
                linked_record_ids: vec![legacy.id],
                confidence: 1.0,
                status: ConstitutionStatus::Active,
            })
            .unwrap();

        let packet = store
            .build_decision_evidence_packet(
                &[],
                &[],
                &[item.id],
                &[gap.id],
                &DecisionMirrorConfig::default(),
            )
            .unwrap();
        assert!(packet.selected_sources.is_empty());
    }

    #[test]
    fn decision_episode_and_reflection_card_persist_with_scores() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-1".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Should Grafyn build Decision Mirror first?".to_string(),
                options: vec!["Decision Mirror".to_string(), "Topology".to_string()],
                stakes: Some("Product direction".to_string()),
                initial_leaning: Some("Decision Mirror".to_string()),
                review_date: Some("2026-05-15".to_string()),
                primitive_assessment: PrimitiveDecisionAssessment {
                    stakes: Some("high".to_string()),
                    reversibility: Some("medium".to_string()),
                    time_horizon: Some("weeks".to_string()),
                    uncertainty: Some("medium".to_string()),
                    agency: Some("high".to_string()),
                    value_tension: Some("ambition vs proof".to_string()),
                    constraint_pressure: None,
                    taste_aesthetic_pull: None,
                    somatic_signal: None,
                    action_gap_risk: None,
                    outcome_feedback: None,
                },
                context_version: None,
            })
            .expect("decision episode should persist");

        let card = store
            .record_reflection_card(ReflectionCardCreate {
                decision_episode_id: episode.id.clone(),
                session_id: episode.session_id.clone(),
                tile_id: episode.tile_id.clone(),
                model_id: "openai/gpt-4".to_string(),
                content: [
                    "## Decision Frame",
                    "You seem pulled toward ambitious topology.",
                    "## Likely Reasoning Pattern",
                    "You tend to prefer large architecture.",
                    "## Recommendation",
                    "Build the smaller proof first.",
                ]
                .join("\n"),
                cited_note_ids: Vec::new(),
                cited_user_record_ids: Vec::new(),
                cited_constitution_item_ids: Vec::new(),
                cited_action_gap_ids: Vec::new(),
                evidence_packet: None,
            })
            .expect("reflection card should persist");

        assert_eq!(card.decision_episode_id, "decision-1");
        assert!(card.scores.unsupported_claim_count > 0);
        assert!(card.scores.overall_score <= 1.0);
        assert_eq!(card.evidence_packet.selected_sources.len(), 0);
        assert_eq!(
            card.evidence_packet
                .config_snapshot
                .as_ref()
                .expect("config snapshot should persist")
                .preset,
            DecisionMirrorPreset::Balanced
        );
        store
            .append_trace_event(
                &episode.session_id,
                TraceEventType::FeedbackRecorded,
                json!({
                    "feedback_type": "reject",
                    "response": {
                        "tile_id": episode.tile_id.clone(),
                        "model_id": "openai/gpt-4",
                    },
                    "rationale": "Decision Mirror reflection marked Not Me",
                }),
            )
            .expect("feedback event should persist");

        let decision_rows = store
            .list_decision_episodes_with_reflections()
            .expect("decision rows should list with traces");
        assert_eq!(decision_rows[0].reflection_cards.len(), 1);
        assert_eq!(decision_rows[0].feedback_events.len(), 1);
        assert_eq!(
            decision_rows[0].feedback_events[0]
                .payload
                .get("feedback_type")
                .and_then(|value| value.as_str()),
            Some("reject")
        );
        let episodes = store
            .list_decision_episodes()
            .expect("episodes should list");
        assert_eq!(episodes.len(), 1);

        let legacy_card: ReflectionCard = serde_json::from_str(
            r#"{
                "id": "legacy-card",
                "decision_episode_id": "decision-1",
                "session_id": "session-1",
                "tile_id": "tile-1",
                "model_id": "openai/gpt-4",
                "content": "legacy reflection",
                "scores": { "overall_score": 0.5 },
                "created_at": "2026-05-08T00:00:00Z"
            }"#,
        )
        .expect("legacy reflection cards should deserialize");
        assert!(legacy_card.evidence_packet.config_snapshot.is_none());
    }

    #[test]
    fn decision_mirror_config_presets_persist() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let default_config = store
            .get_decision_mirror_config()
            .expect("default config should load");
        assert_eq!(default_config.preset, DecisionMirrorPreset::Balanced);

        let updated = store
            .update_decision_mirror_config(DecisionMirrorConfigUpdate {
                preset: Some(DecisionMirrorPreset::EvidenceStrict),
                weights: None,
                advanced_enabled: Some(true),
            })
            .expect("config should update");
        assert_eq!(updated.preset, DecisionMirrorPreset::EvidenceStrict);
        assert!(
            updated.weights.evidence_grounding_weight
                > default_config.weights.evidence_grounding_weight
        );

        let persisted = store
            .get_decision_mirror_config()
            .expect("persisted config should load");
        assert_eq!(persisted.preset, DecisionMirrorPreset::EvidenceStrict);

        let reset = store
            .reset_decision_mirror_config()
            .expect("config should reset");
        assert_eq!(reset.preset, DecisionMirrorPreset::Balanced);
    }

    #[test]
    fn decision_episode_old_json_loads_with_default_prediction_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let store = TwinStore::new(temp_dir.path().to_path_buf());

        let old_json = r#"{
            "id": "legacy-episode",
            "session_id": "session-1",
            "tile_id": "tile-1",
            "decision": "Ship now or wait?",
            "options": ["Ship now", "Wait"],
            "chosen_option": "Ship now",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#;
        std::fs::create_dir_all(&store.decisions_path).expect("decisions dir");
        std::fs::write(store.decisions_path.join("legacy-episode.json"), old_json)
            .expect("legacy episode should write");

        let episodes = store
            .list_decision_episodes()
            .expect("legacy episode should deserialize");
        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert!(episode.twin_prediction.is_none());
        assert!(episode.prediction_status.is_none());
        assert!(episode.agreement.is_none());
        assert!(episode.correction_note.is_none());
        assert!(episode.context_version.is_none());
        assert!(episode.outcome_recorded_at.is_none());
    }

    #[test]
    fn sealed_prediction_redacted_from_reflections_until_outcome() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-sealed".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: vec!["Take it".to_string(), "Stay".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("decision episode should persist");
        assert_eq!(episode.prediction_status.as_deref(), Some("requested"));

        let mut sealed = episode.clone();
        sealed.twin_prediction = Some(TwinPrediction {
            predicted_option: "Stay".to_string(),
            matched_option_index: Some(1),
            confidence: Some(0.7),
            rationale: Some("Family proximity outweighs salary here.".to_string()),
            parse_mode: "json".to_string(),
            model_id: "test/model".to_string(),
            context_version: "ctx-test".to_string(),
            sealed_at: Utc::now(),
        });
        sealed.prediction_status = Some("sealed".to_string());
        store
            .write_decision_file(&sealed)
            .expect("sealed episode should write");

        let listed = store
            .list_decision_episodes_with_reflections()
            .expect("episodes should list");
        let item = listed
            .iter()
            .find(|item| item.episode.id == "decision-sealed")
            .expect("episode should be listed");
        assert!(item.prediction_sealed);
        assert!(item.episode.twin_prediction.is_none());

        store
            .update_decision_outcome(
                "decision-sealed",
                DecisionOutcomeUpdate {
                    chosen_option: Some("Stay".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");

        let listed = store
            .list_decision_episodes_with_reflections()
            .expect("episodes should list after outcome");
        let item = listed
            .iter()
            .find(|item| item.episode.id == "decision-sealed")
            .expect("episode should be listed after outcome");
        assert!(!item.prediction_sealed);
        let prediction = item
            .episode
            .twin_prediction
            .as_ref()
            .expect("prediction should be revealed after outcome");
        assert_eq!(prediction.predicted_option, "Stay");
    }

    fn decided_episode(
        store: &mut TwinStore,
        id: &str,
        decision: &str,
        chosen: &str,
        correction_note: Option<&str>,
    ) -> DecisionEpisode {
        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: id.to_string(),
                session_id: "session-1".to_string(),
                tile_id: format!("tile-{id}"),
                decision: decision.to_string(),
                options: vec!["Option A".to_string(), "Option B".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");
        let mut decided = episode.clone();
        decided.chosen_option = Some(chosen.to_string());
        decided.correction_note = correction_note.map(|note| note.to_string());
        store
            .write_decision_file(&decided)
            .expect("decided episode should write");
        decided
    }

    #[test]
    fn decision_cases_rank_relevance_above_correction_notes() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        decided_episode(
            &mut store,
            "case-relevant",
            "Accept the Denver relocation offer with higher salary?",
            "Option B",
            None,
        );
        decided_episode(
            &mut store,
            "case-correction",
            "Buy the relocation boxes early?",
            "Option A",
            Some("Twin guessed wrong here"),
        );
        // Undecided episode must never appear as a case.
        store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "case-undecided".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-undecided".to_string(),
                decision: "Relocation salary salary salary?".to_string(),
                options: vec!["A".to_string(), "B".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("undecided episode should persist");

        let cases = store
            .select_decision_cases("Denver relocation salary decision", None, 5)
            .expect("cases should select");
        assert_eq!(cases.len(), 2);
        // The strongly relevant case outranks the weakly relevant one even
        // though the weak one carries a correction note.
        assert_eq!(cases[0].id, "case-relevant");
        assert_eq!(cases[1].id, "case-correction");
        assert!(cases.iter().all(|case| case.id != "case-undecided"));

        let excluded = store
            .select_decision_cases(
                "Denver relocation salary decision",
                Some("case-relevant"),
                5,
            )
            .expect("exclusion should apply");
        assert!(excluded.iter().all(|case| case.id != "case-relevant"));

        // Zero overlap: most recent decided cases, capped at two.
        let recent = store
            .select_decision_cases("zzz qqq xyzzy", None, 5)
            .expect("fallback should select");
        assert!(!recent.is_empty());
        assert!(recent.len() <= 2);
        assert!(recent.iter().all(|case| case.chosen_option.is_some()));
    }

    fn prediction_options() -> Vec<String> {
        vec![
            "Take the Denver job".to_string(),
            "Stay in Austin".to_string(),
        ]
    }

    #[test]
    fn parse_twin_prediction_strict_json() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 2, "confidence": 0.8, "rationale": "Family proximity wins."}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.parse_mode, "json");
        assert_eq!(draft.matched_option_index, Some(1));
        assert_eq!(draft.predicted_option, "Stay in Austin");
        assert_eq!(draft.confidence, Some(0.8));
        assert_eq!(draft.rationale.as_deref(), Some("Family proximity wins."));
    }

    #[test]
    fn parse_twin_prediction_fenced_json_with_language_tag() {
        let raw = "```json\n{\"predicted_option\": \"Take the Denver job\", \"option_index\": 1, \"confidence\": 0.6, \"rationale\": \"Growth.\"}\n```";
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.parse_mode, "json");
        assert_eq!(draft.matched_option_index, Some(0));
    }

    #[test]
    fn parse_twin_prediction_index_as_string_and_one_based() {
        let raw = r#"{"option_index": "1", "confidence": 0.5}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        // 1-based reading preferred: "1" means the first listed option.
        assert_eq!(draft.matched_option_index, Some(0));
        assert_eq!(draft.predicted_option, "Take the Denver job");
    }

    #[test]
    fn parse_twin_prediction_text_beats_conflicting_index() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 1, "confidence": 0.9}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        // The option text is what the model said; the conflicting index loses.
        assert_eq!(draft.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_out_of_range_index_with_valid_text() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 9}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_bare_text_and_labels() {
        let options = prediction_options();
        let bare = parse_twin_prediction("  Stay in Austin\n", &options);
        assert_eq!(bare.parse_mode, "string_match");
        assert_eq!(bare.matched_option_index, Some(1));

        let letter = parse_twin_prediction("B", &options);
        assert_eq!(letter.matched_option_index, Some(1));

        let labeled = parse_twin_prediction("Option 2", &options);
        assert_eq!(labeled.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_garbage_falls_to_raw() {
        let long_garbage = "I think there are many considerations here ".repeat(40);
        let draft = parse_twin_prediction(&long_garbage, &prediction_options());
        assert_eq!(draft.parse_mode, "raw");
        assert!(draft.matched_option_index.is_none());
        assert!(draft.predicted_option.chars().count() <= 500);
    }

    #[test]
    fn parse_twin_prediction_reversed_braces_does_not_panic() {
        // The closing brace appears BEFORE the opening one, so `raw.find('{')` finds a
        // start index greater than `raw.rfind('}')`'s end index. Slicing `&raw[start..=end]`
        // without checking `start <= end` panics — which previously crashed the spawned
        // sealed-prediction task, silently skipping both `attach_twin_prediction` and
        // `mark_twin_prediction_failed`.
        let raw = "Option A} — but {incomplete";
        let draft = parse_twin_prediction(raw, &prediction_options());
        // No panic reaching here is the primary assertion. The malformed brace pair is
        // simply not treated as JSON, so it falls through the same fallback chain as any
        // other non-JSON text.
        assert_ne!(draft.parse_mode, "json");
    }

    #[test]
    fn extract_json_slice_rejects_reversed_braces() {
        assert_eq!(extract_json_slice("Option A} — but {incomplete"), None);
        assert_eq!(extract_json_slice("no braces here"), None);
        assert_eq!(extract_json_slice("only open {"), None);
        assert_eq!(extract_json_slice("only close }"), None);
        assert_eq!(
            extract_json_slice(r#"prefix {"a": 1} suffix"#),
            Some(r#"{"a": 1}"#)
        );
    }

    #[test]
    fn parse_twin_prediction_sanitizes_confidence() {
        let options = prediction_options();
        let percent = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": 73}"#,
            &options,
        );
        assert_eq!(percent.confidence, Some(0.73));

        let overshoot = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": 1.2}"#,
            &options,
        );
        assert_eq!(overshoot.confidence, Some(1.0));

        let negative = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": -0.1}"#,
            &options,
        );
        assert_eq!(negative.confidence, Some(0.0));

        let null = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": null}"#,
            &options,
        );
        assert!(null.confidence.is_none());
    }

    #[test]
    fn attach_twin_prediction_refuses_after_outcome_and_duplicates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-attach".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("episode should persist");

        let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
        let sealed = store
            .attach_twin_prediction(&episode.id, draft.clone(), "test/model", "ctx-test")
            .expect("first attach should seal");
        assert_eq!(sealed.prediction_status.as_deref(), Some("sealed"));
        let first_sealed_at = sealed.twin_prediction.as_ref().unwrap().sealed_at;

        // Duplicate attach is a no-op.
        let duplicate = store
            .attach_twin_prediction(&episode.id, draft.clone(), "other/model", "ctx-test")
            .expect("duplicate attach should not error");
        assert_eq!(
            duplicate.twin_prediction.as_ref().unwrap().sealed_at,
            first_sealed_at
        );
        assert_eq!(
            duplicate.twin_prediction.as_ref().unwrap().model_id,
            "test/model"
        );

        // Outcome-first race: attach after the choice is recorded.
        let late_episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-late".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-2".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");
        store
            .update_decision_outcome(
                &late_episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("Stay in Austin".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        let refused = store
            .attach_twin_prediction(&late_episode.id, draft, "test/model", "ctx-test")
            .expect("late attach should not error");
        assert!(refused.twin_prediction.is_none());
        assert_eq!(
            refused.prediction_status.as_deref(),
            Some("outcome_recorded_first")
        );
    }

    #[test]
    fn outcome_computes_agreement_and_canonicalizes_choice() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-agree".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("episode should persist");
        let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
        store
            .attach_twin_prediction(&episode.id, draft, "test/model", "ctx-test")
            .expect("prediction should seal");

        // Label + case variant resolves to the canonical option text and
        // agreement computes via index comparison.
        let updated = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("option 2".to_string()),
                    correction_note: None,
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        assert_eq!(updated.chosen_option.as_deref(), Some("Stay in Austin"));
        assert_eq!(updated.agreement, Some(true));
        assert!(updated.outcome_recorded_at.is_some());

        // Editing the choice recomputes agreement and accepts a correction
        // note; outcome_recorded_at is not reset.
        let first_recorded_at = updated.outcome_recorded_at;
        let edited = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("  TAKE the denver JOB ".to_string()),
                    correction_note: Some("Twin overweighted family proximity.".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("edited outcome should record");
        assert_eq!(edited.chosen_option.as_deref(), Some("Take the Denver job"));
        assert_eq!(edited.agreement, Some(false));
        assert_eq!(
            edited.correction_note.as_deref(),
            Some("Twin overweighted family proximity.")
        );
        assert_eq!(edited.outcome_recorded_at, first_recorded_at);
    }

    #[test]
    fn outcome_without_prediction_records_no_agreement_and_keeps_free_text() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-free".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");

        let updated = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("Negotiated a remote arrangement instead".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        // Free text that matches no option is preserved verbatim.
        assert_eq!(
            updated.chosen_option.as_deref(),
            Some("Negotiated a remote arrangement instead")
        );
        assert!(updated.agreement.is_none());
    }
