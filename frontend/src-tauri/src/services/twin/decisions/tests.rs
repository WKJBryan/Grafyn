use super::prediction::extract_json_slice;
use super::*;
use tempfile::tempdir;

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
            repeat_of: None,
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
    let store = TwinStore::new(temp_dir.path().to_path_buf());

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
            repeat_of: None,
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
        option_probabilities: None,
        elicitation_prompt: None,
        temperature: None,
        sealed_payload_hash: None,
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
            repeat_of: None,
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
            repeat_of: None,
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
            repeat_of: None,
        })
        .expect("episode should persist");

    let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
    let sealed = store
        .attach_twin_prediction(
            &episode.id,
            draft.clone(),
            "test/model",
            "ctx-test",
            None,
            None,
        )
        .expect("first attach should seal");
    assert_eq!(sealed.prediction_status.as_deref(), Some("sealed"));
    let first_sealed_at = sealed.twin_prediction.as_ref().unwrap().sealed_at;

    // Duplicate attach is a no-op.
    let duplicate = store
        .attach_twin_prediction(
            &episode.id,
            draft.clone(),
            "other/model",
            "ctx-test",
            None,
            None,
        )
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
            repeat_of: None,
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
        .attach_twin_prediction(
            &late_episode.id,
            draft,
            "test/model",
            "ctx-test",
            None,
            None,
        )
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
            repeat_of: None,
        })
        .expect("episode should persist");
    let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
    store
        .attach_twin_prediction(&episode.id, draft, "test/model", "ctx-test", None, None)
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
            repeat_of: None,
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

fn create_for(id: &str, decision: &str, options: Vec<String>) -> DecisionEpisodeCreate {
    DecisionEpisodeCreate {
        id: id.to_string(),
        session_id: "session-1".to_string(),
        tile_id: format!("tile-{id}"),
        decision: decision.to_string(),
        options,
        stakes: None,
        initial_leaning: None,
        review_date: None,
        primitive_assessment: PrimitiveDecisionAssessment::default(),
        context_version: Some("ctx-test".to_string()),
        repeat_of: None,
    }
}

#[test]
fn parse_twin_prediction_option_probabilities_validated_not_renormalized() {
    let options = prediction_options();
    let parse = |probs: &str| {
        parse_twin_prediction(
            &format!(
                r#"{{"predicted_option": "Stay in Austin", "option_probabilities": {probs}}}"#
            ),
            &options,
        )
    };
    let valid = parse("[0.3, 0.6]");
    assert_eq!(valid.parse_mode, "json");
    assert_eq!(valid.option_probabilities, Some(vec![0.3, 0.6]));
    for bad in [
        "[0.5]",
        "[0.2, 0.3, 0.5]",
        "[1.2, 0.1]",
        "[-0.1, 0.9]",
        "[\"0.5\", 0.5]",
        "0.5",
    ] {
        let draft = parse(bad);
        assert_eq!(
            draft.parse_mode, "json",
            "bad vector must not break JSON parse: {bad}"
        );
        assert!(draft.option_probabilities.is_none(), "{bad}");
        assert_eq!(draft.matched_option_index, Some(1));
    }
    assert!(parse_twin_prediction("Stay in Austin", &options)
        .option_probabilities
        .is_none());
}

#[test]
fn sealing_stamps_payload_hash_prompt_temperature_and_probabilities() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = TwinStore::new(temp_dir.path().to_path_buf());
    store
        .record_decision_episode(create_for(
            "seal-1",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    let draft = parse_twin_prediction(
        r#"{"predicted_option": "Stay in Austin", "option_probabilities": [0.25, 0.75]}"#,
        &prediction_options(),
    );
    let sealed = store
        .attach_twin_prediction(
            "seal-1",
            draft,
            "test/model",
            "ctx-test",
            Some("the prompt"),
            Some(0.2),
        )
        .expect("prediction should seal");
    let prediction = sealed.twin_prediction.expect("prediction stored");
    assert_eq!(prediction.option_probabilities, Some(vec![0.25, 0.75]));
    assert_eq!(prediction.elicitation_prompt.as_deref(), Some("the prompt"));
    assert_eq!(prediction.temperature, Some(0.2));
    assert_eq!(
        prediction.sealed_payload_hash,
        Some(sealed_payload_hash(
            "Take the Denver job?",
            &prediction_options()
        ))
    );
    // Canonical format pinned: decision + "\n" + options joined by "\n".
    assert_eq!(
        sealed_payload_hash("D", &["A".to_string(), "B".to_string()]),
        format!("{:016x}", super::super::shared::fnv1a64("D\nA\nB"))
    );
}

#[test]
fn post_seal_edit_voids_prediction_and_skips_agreement() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = TwinStore::new(temp_dir.path().to_path_buf());
    let draft = || parse_twin_prediction("Stay in Austin", &prediction_options());

    // Path 1: re-recording the same id with changed options.
    store
        .record_decision_episode(create_for(
            "void-1",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    store
        .attach_twin_prediction("void-1", draft(), "test/model", "ctx-test", None, None)
        .expect("prediction should seal");
    let mut changed = prediction_options();
    changed.push("Move to Seattle".to_string());
    let rerecorded = store
        .record_decision_episode(create_for("void-1", "Take the Denver job?", changed))
        .expect("re-record should persist");
    assert_eq!(rerecorded.prediction_status.as_deref(), Some("voided"));
    assert!(rerecorded.twin_prediction.is_some(), "kept for audit");
    let decided = store
        .update_decision_outcome(
            "void-1",
            DecisionOutcomeUpdate {
                chosen_option: Some("Stay in Austin".to_string()),
                ..DecisionOutcomeUpdate::default()
            },
        )
        .expect("outcome should record");
    assert!(decided.agreement.is_none());

    // Path 2: decision text changed on disk after sealing (hand edit / future edit path).
    store
        .record_decision_episode(create_for(
            "void-2",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    let mut edited = store
        .attach_twin_prediction("void-2", draft(), "test/model", "ctx-test", None, None)
        .expect("prediction should seal");
    edited.decision = "Take the Boston job?".to_string();
    store
        .write_decision_file(&edited)
        .expect("edit should write");
    let decided = store
        .update_decision_outcome(
            "void-2",
            DecisionOutcomeUpdate {
                chosen_option: Some("Stay in Austin".to_string()),
                ..DecisionOutcomeUpdate::default()
            },
        )
        .expect("outcome should record");
    assert_eq!(decided.prediction_status.as_deref(), Some("voided"));
    assert!(decided.agreement.is_none());
    let trace = store
        .list_session_traces()
        .expect("traces should list")
        .into_iter()
        .find(|trace| trace.session_id == "session-1")
        .expect("trace exists");
    assert_eq!(
        trace
            .events
            .iter()
            .filter(|event| event.event_type == TraceEventType::TwinPredictionVoided)
            .count(),
        2
    );

    // Unchanged payload keeps the seal and scores normally.
    store
        .record_decision_episode(create_for(
            "keep-1",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    store
        .attach_twin_prediction("keep-1", draft(), "test/model", "ctx-test", None, None)
        .expect("prediction should seal");
    let decided = store
        .update_decision_outcome(
            "keep-1",
            DecisionOutcomeUpdate {
                chosen_option: Some("Stay in Austin".to_string()),
                ..DecisionOutcomeUpdate::default()
            },
        )
        .expect("outcome should record");
    assert_eq!(decided.prediction_status.as_deref(), Some("sealed"));
    assert_eq!(decided.agreement, Some(true));
}

#[test]
fn answer_visibility_is_stamped_at_first_choice_and_unknown_without_render() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = TwinStore::new(temp_dir.path().to_path_buf());
    let choose = |store: &mut TwinStore, id: &str| {
        store
            .update_decision_outcome(
                id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("Stay in Austin".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record")
    };

    store
        .record_decision_episode(create_for(
            "seen",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    let rendered_at = Utc::now() - chrono::Duration::seconds(5);
    store
        .mark_twin_answer_rendered("seen", rendered_at)
        .expect("render should stamp");
    store
        .mark_twin_answer_rendered("seen", Utc::now())
        .expect("second stamp is a no-op");
    let seen = choose(&mut store, "seen");
    assert_eq!(seen.twin_answer_rendered_at, Some(rendered_at));
    assert_eq!(seen.twin_answer_visible_before_choice, Some(true));

    store
        .record_decision_episode(create_for(
            "unseen",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("episode should persist");
    let unseen = choose(&mut store, "unseen");
    assert_eq!(unseen.twin_answer_visible_before_choice, None);
}

#[test]
fn repeat_of_must_reference_another_existing_episode() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = TwinStore::new(temp_dir.path().to_path_buf());
    // An invalid repeat_of is dropped, never a create failure — a bad
    // value must not kill the user's prompt in send_prompt.
    let mut missing = create_for("repeat-1", "Take the Denver job?", prediction_options());
    missing.repeat_of = Some("does-not-exist".to_string());
    let stored = store
        .record_decision_episode(missing)
        .expect("invalid repeat_of must not fail the episode");
    assert_eq!(stored.repeat_of, None);

    store
        .record_decision_episode(create_for(
            "original",
            "Take the Denver job?",
            prediction_options(),
        ))
        .expect("original should persist");
    let mut repeat = create_for("repeat-1", "Take the Denver job?", prediction_options());
    repeat.repeat_of = Some("original".to_string());
    let stored = store
        .record_decision_episode(repeat)
        .expect("repeat should persist");
    assert_eq!(stored.repeat_of.as_deref(), Some("original"));
}
