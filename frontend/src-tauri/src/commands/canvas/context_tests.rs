use super::*;
use crate::commands::canvas::test_support::build_tile;
use crate::models::canvas::{CanvasViewport, PromptTile};
use crate::models::note::NoteStatus;
use crate::models::twin::{ConstitutionSetup, PrimitiveDecisionAssessment};
use chrono::Utc;

fn build_session(tiles: Vec<PromptTile>) -> CanvasSession {
    CanvasSession {
        id: "session-1".to_string(),
        title: "Canvas".to_string(),
        description: None,
        prompt_tiles: tiles,
        debates: Vec::new(),
        viewport: CanvasViewport::default(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tags: Vec::new(),
        status: "draft".to_string(),
        pinned_note_ids: Vec::new(),
    }
}

fn build_request(
    prompt: &str,
    parent_tile_id: &str,
    parent_model_id: &str,
    context_mode: ContextMode,
) -> PromptRequest {
    PromptRequest {
        prompt: prompt.to_string(),
        prompt_type: PromptType::Standard,
        system_prompt: None,
        models: vec!["openai/gpt-4".to_string()],
        position: None,
        context_mode,
        twin_answer_mode: TwinAnswerMode::default(),
        twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
        twin_context_policy: None,
        twin_llm_provider: None,
        decision_metadata: None,
        parent_tile_id: Some(parent_tile_id.to_string()),
        parent_model_id: Some(parent_model_id.to_string()),
        temperature: 0.7,
        max_tokens: None,
        web_search: false,
        web_search_max_results: 5,
        reasoning_effort: "none".to_string(),
    }
}

fn build_root_request(prompt: &str, context_mode: ContextMode) -> PromptRequest {
    PromptRequest {
        prompt: prompt.to_string(),
        prompt_type: PromptType::Standard,
        system_prompt: None,
        models: vec!["openai/gpt-4".to_string()],
        position: None,
        context_mode,
        twin_answer_mode: TwinAnswerMode::default(),
        twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
        twin_context_policy: None,
        twin_llm_provider: None,
        decision_metadata: None,
        parent_tile_id: None,
        parent_model_id: None,
        temperature: 0.7,
        max_tokens: None,
        web_search: false,
        web_search_max_results: 5,
        reasoning_effort: "none".to_string(),
    }
}

fn relationship_variant(object_id: &str) -> crate::models::twin_state::RelationshipVariant {
    crate::models::twin_state::RelationshipVariant::new(vec![
        crate::models::twin_state::RelationshipKey {
            subject_id: crate::models::twin_event::EntityId::parse("owner").unwrap(),
            predicate: crate::models::twin_event::RelationshipPredicate::parse("works_with")
                .unwrap(),
            object_id: crate::models::twin_event::EntityId::parse(object_id).unwrap(),
            direction: crate::models::twin_event::RelationshipDirection::Directed,
        },
    ])
}

fn twin_evidence(
    variant: crate::models::twin_state::RelationshipVariant,
    context_version: &str,
) -> TwinEvidenceSnapshot {
    TwinEvidenceSnapshot {
        projection_snapshot_id: crate::models::twin_state::SnapshotId::parse("a".repeat(64))
            .unwrap(),
        reference_time: Utc::now(),
        prompt_context_digest: crate::models::twin_event::ContentDigest::parse("b".repeat(64))
            .unwrap(),
        prompt_context_version: Some(context_version.into()),
        twin_relationship_variant: variant,
        evidence_event_ids: Vec::new(),
        note_ids: Vec::new(),
    }
}

fn build_retrieval_result(
    id: &str,
    title: &str,
    snippet: &str,
    score: f32,
    reasons: &[&str],
) -> RetrievalResult {
    RetrievalResult {
        note: crate::models::note::NoteMeta {
            id: id.to_string(),
            title: title.to_string(),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::default(),
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
        },
        score,
        snippet: snippet.to_string(),
        relevance_reasons: reasons.iter().map(|reason| reason.to_string()).collect(),
    }
}

fn build_constitution_item(
    id: &str,
    claim: &str,
    status: crate::models::twin::ConstitutionStatus,
    source_type: &str,
) -> ConstitutionItem {
    ConstitutionItem {
        id: id.to_string(),
        claim: claim.to_string(),
        dimension: "values".to_string(),
        scope: vec!["general".to_string()],
        priority: 0.8,
        confidence: 0.82,
        status,
        evidence_refs: vec![crate::models::twin::EvidenceRef {
            trace_id: format!("trace-{}", id),
            event_id: format!("event-{}", id),
            session_id: "session-1".to_string(),
            tile_id: None,
            model_id: None,
            note: Some("Evidence note".to_string()),
            source_type: Some(source_type.to_string()),
            source_id: Some(format!("source-{}", id)),
            source_label: Some("Interview question".to_string()),
            excerpt: Some("Can you give a concrete example?".to_string()),
            speaker_role: Some("user".to_string()),
        }],
        tensions: Vec::new(),
        linked_record_ids: Vec::new(),
        source: Some("interview_behavior_inference".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn build_action_gap(id: &str) -> ActionGap {
    ActionGap {
        id: id.to_string(),
        stated_value: "Protect mission alignment".to_string(),
        revealed_behavior: "Accepts attractive adjacent projects".to_string(),
        driver_hypothesis: Some("Funding pressure".to_string()),
        somatic_taste_signal: Some("Prestige pull".to_string()),
        decision_risk: "May divert faculty from core mission work".to_string(),
        evidence_refs: Vec::new(),
        linked_record_ids: Vec::new(),
        confidence: 0.72,
        status: crate::models::twin::ConstitutionStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn test_twin_identity_setup() -> ConstitutionSetup {
    ConstitutionSetup {
        twin_name: Some("Alex Chen".into()),
        twin_role: Some("founder deciding from product evidence".into()),
        source_boundaries: vec!["Use reviewed notes and uploaded interviews only.".into()],
        ..ConstitutionSetup::default()
    }
}

#[test]
fn test_build_note_context_prompt_with_notes() {
    let notes = vec![
        ("id1".into(), "Note A".into(), "Content of A".into()),
        ("id2".into(), "Note B".into(), "Content of B".into()),
    ];
    let prompt = build_note_context_prompt(&notes);

    assert!(prompt.contains("Note A"));
    assert!(prompt.contains("Content of A"));
    assert!(prompt.contains("Note B"));
    assert!(prompt.contains("id: id1"));
}

#[test]
fn test_build_note_context_prompt_empty() {
    let notes: Vec<(String, String, String)> = vec![];
    let prompt = build_note_context_prompt(&notes);

    assert!(prompt.contains("No relevant notes were found"));
}

#[test]
fn test_build_chunk_context_prompt_groups_by_parent() {
    let chunks = vec![
        ChunkResult {
            chunk_id: "c1".into(),
            parent_note_id: "note-a".into(),
            parent_title: "Note A".into(),
            text: "First paragraph of A".into(),
            start_char: 0,
            end_char: 20,
            depth_score: 1.0,
            search_score: 5.0,
            token_estimate: 10,
        },
        ChunkResult {
            chunk_id: "c2".into(),
            parent_note_id: "note-b".into(),
            parent_title: "Note B".into(),
            text: "Content of B".into(),
            start_char: 0,
            end_char: 12,
            depth_score: 1.0,
            search_score: 4.0,
            token_estimate: 8,
        },
        ChunkResult {
            chunk_id: "c3".into(),
            parent_note_id: "note-a".into(),
            parent_title: "Note A".into(),
            text: "Second paragraph of A".into(),
            start_char: 21,
            end_char: 42,
            depth_score: 0.5,
            search_score: 3.5,
            token_estimate: 10,
        },
    ];
    let prompt = build_chunk_context_prompt(&chunks);

    // Both chunks from Note A should be under the same heading
    assert!(prompt.contains("### Note A (id: note-a)"));
    assert!(prompt.contains("First paragraph of A"));
    assert!(prompt.contains("Second paragraph of A"));
    assert!(prompt.contains("### Note B (id: note-b)"));
    assert!(prompt.contains("Content of B"));
    // Note A should appear before Note B (insertion order from chunks)
    let a_pos = prompt.find("Note A").unwrap();
    let b_pos = prompt.find("Note B").unwrap();
    assert!(a_pos < b_pos);
}

#[test]
fn test_build_chunk_context_prompt_empty() {
    let chunks: Vec<ChunkResult> = vec![];
    let prompt = build_chunk_context_prompt(&chunks);
    assert!(prompt.contains("No relevant notes were found"));
}

#[test]
fn twin_context_prompt_separates_approved_candidates_and_advisor_instructions() {
    let approved = vec![TwinContextRecord {
        id: "record-approved".into(),
        kind: crate::models::twin::UserRecordKind::Preference,
        content: "User prefers evidence-backed implementation detail.".into(),
        confidence: 0.9,
        promotion_state: crate::models::twin::PromotionState::Endorsed,
        evidence_count: 4,
        source_label: Some("approved".into()),
    }];
    let candidates = vec![TwinContextRecord {
        id: "record-candidate".into(),
        kind: crate::models::twin::UserRecordKind::ReasoningPattern,
        content: "User may prefer red-team critique before shipping.".into(),
        confidence: 0.62,
        promotion_state: crate::models::twin::PromotionState::Candidate,
        evidence_count: 1,
        source_label: Some("candidate".into()),
    }];
    let constitution = vec![
        build_constitution_item(
            "constitution-active",
            "Prefer mission alignment before opportunistic funding.",
            crate::models::twin::ConstitutionStatus::Active,
            "interview-question",
        ),
        build_constitution_item(
            "constitution-candidate",
            "May prefer negotiation before rejection.",
            crate::models::twin::ConstitutionStatus::Candidate,
            "behavior",
        ),
        build_constitution_item(
            "constitution-rejected",
            "Rejected claims must not leak.",
            crate::models::twin::ConstitutionStatus::Rejected,
            "note",
        ),
        build_constitution_item(
            "constitution-not-me",
            "Not-me claims must not leak.",
            crate::models::twin::ConstitutionStatus::NotMe,
            "note",
        ),
        build_constitution_item(
            "constitution-no-train",
            "No-train claims must not leak.",
            crate::models::twin::ConstitutionStatus::NoTrain,
            "note",
        ),
    ];
    let gaps = vec![build_action_gap("gap-1")];

    let prompt = build_twin_context_prompt(
            &ConstitutionSetup::default(),
            &[],
            &[(
                "note-1".into(),
                "Decision Notes".into(),
                "### Message 2: Interviewee\nExpert says accept grants when partnerships are strategic.".into(),
            )],
            &approved,
            &candidates,
            &constitution,
            &gaps,
            &TwinAnswerMode::Advisor,
            &PromptType::Standard,
            None,
        );

    assert!(prompt.contains("## Twin Operating Contract"));
    assert!(prompt.contains("## Reviewed Constitution"));
    assert!(prompt.contains("Prefer mission alignment before opportunistic funding."));
    assert!(prompt.contains("Interview question"));
    assert!(prompt.contains("## Action Gap Risks"));
    assert!(prompt.contains("May divert faculty from core mission work"));
    assert!(prompt.contains("## Relevant Evidence"));
    assert!(prompt.contains("Expert says accept grants when partnerships are strategic."));
    assert!(prompt.contains("## Approved User Records"));
    assert!(prompt.contains("## Tentative Candidate Records"));
    assert!(prompt.contains("Candidate Constitution Hypotheses"));
    assert!(prompt.contains("May prefer negotiation before rejection."));
    assert!(!prompt.contains("Rejected claims must not leak."));
    assert!(!prompt.contains("Not-me claims must not leak."));
    assert!(!prompt.contains("No-train claims must not leak."));
    assert!(prompt.contains("Use interviewee answers as evidence about the interviewee"));
    assert!(prompt.contains("Do not use evidence to justify a preselected answer"));
    assert!(prompt.contains("Recommended option"));
    assert!(prompt.contains("decision-support assistant"));
}

#[test]
fn legacy_auto_promoted_content_cannot_enter_twin_prompt_sections() {
    let legacy = TwinContextRecord {
        id: "legacy-auto".into(),
        kind: crate::models::twin::UserRecordKind::Preference,
        content: "LEGACY_AUTO_MUST_NOT_ENTER_PROMPT".into(),
        confidence: 1.0,
        promotion_state: crate::models::twin::PromotionState::AutoPromoted,
        evidence_count: 99,
        source_label: Some("legacy".into()),
    };
    let prompt = build_twin_context_prompt(
        &ConstitutionSetup::default(),
        &[],
        &[],
        std::slice::from_ref(&legacy),
        std::slice::from_ref(&legacy),
        &[],
        &[],
        &TwinAnswerMode::Advisor,
        &PromptType::Standard,
        None,
    );
    assert!(!prompt.contains("LEGACY_AUTO_MUST_NOT_ENTER_PROMPT"));
    assert!(!prompt.contains("auto-promoted"));
}

#[test]
fn materialized_legacy_artifacts_cannot_enter_canvas_twin_context() {
    let temp_dir = tempfile::tempdir().unwrap();
    let records_path = temp_dir.path().join("records");
    std::fs::create_dir_all(&records_path).unwrap();
    let now = Utc::now();
    let legacy = crate::models::twin::UserRecord {
        id: "legacy-canvas-record".to_string(),
        kind: crate::models::twin::UserRecordKind::Preference,
        content: "legacy Canvas authority".to_string(),
        evidence_refs: Vec::new(),
        confidence: 1.0,
        origin: crate::models::twin::RecordOrigin::Inferred,
        promotion_state: crate::models::twin::PromotionState::AutoPromoted,
        created_at: now,
        updated_at: now,
        valid_from: None,
        valid_until: None,
        links: Vec::new(),
        metadata: std::collections::HashMap::new(),
    };
    crate::services::atomic_io::write_atomic(
        &records_path.join("legacy-canvas-record.json"),
        serde_json::to_vec_pretty(&legacy).unwrap().as_slice(),
    )
    .unwrap();
    let mut store = crate::services::twin::TwinStore::new(temp_dir.path().to_path_buf());
    store
        .create_constitution_item(crate::models::twin::ConstitutionItemCreate {
            claim: "LEGACY_CONSTITUTION_MUST_NOT_ENTER_CANVAS".to_string(),
            dimension: "values".to_string(),
            scope: Vec::new(),
            priority: 1.0,
            confidence: 1.0,
            status: crate::models::twin::ConstitutionStatus::Active,
            evidence_refs: Vec::new(),
            tensions: Vec::new(),
            linked_record_ids: vec![legacy.id.clone()],
            source: None,
        })
        .unwrap();
    store
        .create_action_gap(crate::models::twin::ActionGapCreate {
            stated_value: "Move quickly".to_string(),
            revealed_behavior: "Waited".to_string(),
            driver_hypothesis: None,
            somatic_taste_signal: None,
            decision_risk: "LEGACY_GAP_MUST_NOT_ENTER_CANVAS".to_string(),
            evidence_refs: Vec::new(),
            linked_record_ids: vec![legacy.id],
            confidence: 1.0,
            status: crate::models::twin::ConstitutionStatus::Active,
        })
        .unwrap();
    let (constitution, gaps) = store
        .select_constitution_context("LEGACY CONSTITUTION GAP CANVAS")
        .unwrap();
    let prompt = build_twin_context_prompt(
        &ConstitutionSetup::default(),
        &[],
        &[],
        &[],
        &[],
        &constitution,
        &gaps,
        &TwinAnswerMode::Advisor,
        &PromptType::Standard,
        None,
    );
    assert!(!prompt.contains("LEGACY_CONSTITUTION_MUST_NOT_ENTER_CANVAS"));
    assert!(!prompt.contains("LEGACY_GAP_MUST_NOT_ENTER_CANVAS"));
}

#[test]
fn twin_context_prompt_labels_simulation_mode() {
    let setup = test_twin_identity_setup();
    let prompt = build_twin_context_prompt(
        &setup,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &TwinAnswerMode::Simulation,
        &PromptType::Standard,
        None,
    );

    assert!(prompt.contains("## Twin Identity"));
    assert!(prompt.contains("I am Alex Chen."));
    assert!(prompt.contains("My role/context is founder deciding from product evidence."));
    assert!(prompt.contains("Use reviewed notes and uploaded interviews only."));
    assert!(prompt.contains("Continue my documented reasoning pattern"));
    assert!(prompt.contains("do not append questions unless the user's request asks for them"));
    assert!(!prompt.contains("reflective questions"));
    assert!(!prompt.contains("next question"));
    assert!(!prompt.contains("not the user's actual view"));
    assert!(prompt.contains("## Reviewed Constitution"));
}

#[test]
fn twin_context_prompt_rejects_simulation_without_identity() {
    let setup = ConstitutionSetup::default();

    let result = validate_twin_identity_for_answer_mode(&setup, &TwinAnswerMode::Simulation);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("Twin Identity requires Name and Role / context"));
}

#[test]
fn decision_advisor_prompt_uses_reflection_card_structure() {
    let metadata = DecisionPromptMetadata {
        decision: "Should Grafyn build Decision Mirror first?".into(),
        options: vec!["Decision Mirror".into(), "Topology layer".into()],
        stakes: Some("Product direction".into()),
        initial_leaning: Some("Decision Mirror first".into()),
        review_date: Some("2026-05-15".into()),
    };

    let prompt = build_twin_context_prompt(
        &ConstitutionSetup::default(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &TwinAnswerMode::Advisor,
        &PromptType::Decision,
        Some(&metadata),
    );

    assert!(prompt.contains("Decision Mirror session"));
    assert!(prompt.contains("Reflection Card"));
    assert!(prompt.contains("Blind Spot Hypothesis"));
    assert!(prompt.contains("Recommendation must be derived after the Constitution Check and Evidence From Grafyn sections"));
    assert!(prompt.contains("Decision: Should Grafyn build Decision Mirror first?"));
    assert!(prompt.contains("Topology layer"));
}

#[test]
fn decision_simulation_prompt_uses_base_simulation_without_decision_style_block() {
    let metadata = DecisionPromptMetadata {
        decision: "Should Grafyn build Decision Mirror first?".into(),
        options: vec!["Decision Mirror".into(), "Topology layer".into()],
        stakes: Some("Product direction".into()),
        initial_leaning: Some("Decision Mirror first".into()),
        review_date: Some("2026-05-15".into()),
    };

    let prompt = build_twin_context_prompt(
        &test_twin_identity_setup(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &TwinAnswerMode::Simulation,
        &PromptType::Decision,
        Some(&metadata),
    );

    assert!(!prompt.contains("Reflection Card"));
    assert!(!prompt.contains("Evidence From Grafyn"));
    assert!(!prompt.contains("Blind Spot Hypothesis"));
    assert!(!prompt.contains("Decision Mirror Simulation Style"));
    assert!(!prompt.contains("Decision Mirror simulation session"));
    assert!(!prompt.contains("natural first-person reflection"));
    assert!(!prompt.contains("numbered headings"));
    assert!(prompt.contains("do not append questions unless the user's request asks for them"));
    assert!(!prompt.contains("not the user's actual view"));
    assert!(prompt.contains("Decision: Should Grafyn build Decision Mirror first?"));
}

fn test_decision_case(
    id: &str,
    decision: &str,
    lesson: Option<&str>,
    note: Option<&str>,
) -> DecisionEpisode {
    let now = Utc::now();
    DecisionEpisode {
        id: id.to_string(),
        session_id: "session-1".to_string(),
        tile_id: format!("tile-{id}"),
        decision: decision.to_string(),
        options: vec!["Ship now".to_string(), "Wait a sprint".to_string()],
        stakes: None,
        initial_leaning: Some("Ship now".to_string()),
        selected_response: None,
        chosen_option: Some("Wait a sprint".to_string()),
        confidence: None,
        review_date: None,
        outcome: None,
        regret_score: None,
        lesson: lesson.map(|text| text.to_string()),
        missed_something: None,
        primitive_assessment: PrimitiveDecisionAssessment::default(),
        twin_prediction: None,
        prediction_status: None,
        agreement: None,
        correction_note: note.map(|text| text.to_string()),
        context_version: None,
        outcome_recorded_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn twin_context_prompt_renders_past_decision_cases_verbatim() {
    let case = test_decision_case(
        "case-1",
        "Ship the importer before polish?",
        Some("I always regret shipping before the empty states are done."),
        Some("Twin assumed I optimize for speed; I optimize for trust."),
    );

    let prompt = build_twin_context_prompt(
        &ConstitutionSetup::default(),
        &[case],
        &[],
        &[],
        &[],
        &[],
        &[],
        &TwinAnswerMode::Advisor,
        &PromptType::Standard,
        None,
    );

    assert!(prompt.contains("## Past Decision Cases"));
    assert!(prompt.contains("Past decision: Ship the importer before polish?"));
    assert!(prompt.contains("Options: Ship now | Wait a sprint"));
    assert!(prompt.contains("Chose: Wait a sprint"));
    assert!(prompt.contains("I always regret shipping before the empty states are done."));
    assert!(prompt.contains("Twin assumed I optimize for speed; I optimize for trust."));
    assert!(prompt.contains("Weight them above abstracted records"));
}

#[test]
fn twin_context_budget_keeps_cases_and_drops_notes_when_tight() {
    let case = test_decision_case("case-1", "Ship the importer before polish?", None, None);
    let big_note = (
        "note-1".to_string(),
        "Big note".to_string(),
        "evidence ".repeat(400),
    );

    let selection = apply_twin_context_budget(
        vec![case],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![big_note],
        60,
    );

    assert_eq!(selection.cases.len(), 1);
    assert!(selection.notes.is_empty());
}

#[test]
fn sealed_prediction_prompt_is_immersed_with_identity_and_never_meta_framed() {
    let setup = test_twin_identity_setup();
    let options = vec!["Ship now".to_string(), "Wait a sprint".to_string()];
    let user_message = build_twin_prediction_user_message(
        &setup,
        "Ship the importer before polish?",
        &options,
        Some("Launch trust"),
    );
    let system_prompt = build_twin_context_prompt(
        &setup,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &TwinAnswerMode::Simulation,
        &PromptType::Decision,
        None,
    );

    assert!(user_message.contains("I am Alex Chen."));
    assert!(user_message.contains("Which option do I choose?"));
    assert!(user_message.contains("1. Ship now"));
    assert!(user_message.contains("2. Wait a sprint"));
    assert!(user_message.contains("predicted_option"));

    // The full model-facing prompt must never meta-frame the twin.
    let full_prompt = format!("{system_prompt}\n{user_message}").to_lowercase();
    for forbidden in [
        "simulate",
        "roleplay",
        "role-play",
        "predict what the user",
        "what would the user",
        "pretend to be",
    ] {
        assert!(
            !full_prompt.contains(forbidden),
            "model-facing prompt contains forbidden meta-framing: {forbidden}"
        );
    }
}

#[test]
fn test_build_selected_parent_chain_returns_root_to_leaf_order() {
    let session = build_session(vec![
        build_tile(
            "tile-1",
            "Root prompt",
            "openai/gpt-4",
            "Root response",
            None,
            None,
        ),
        build_tile(
            "tile-2",
            "Follow-up prompt",
            "openai/gpt-4",
            "Follow-up response",
            Some("tile-1"),
            Some("openai/gpt-4"),
        ),
        build_tile(
            "tile-3",
            "Deep prompt",
            "openai/gpt-4",
            "Deep response",
            Some("tile-2"),
            Some("openai/gpt-4"),
        ),
    ]);
    let request = build_request(
        "Newest prompt",
        "tile-3",
        "openai/gpt-4",
        ContextMode::FullHistory,
    );

    let turns = build_selected_parent_chain(&session, &request).unwrap();

    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0].prompt, "Root prompt");
    assert_eq!(turns[1].prompt, "Follow-up prompt");
    assert_eq!(turns[2].prompt, "Deep prompt");
}

#[test]
fn twin_history_parent_chain_rejects_a_cross_relationship_ancestor() {
    let mut bob_root = build_tile(
        "tile-bob",
        "Bob prompt",
        "openai/gpt-4",
        "Bob response",
        None,
        None,
    );
    bob_root.twin_relationship_variant = relationship_variant("bob");
    let mut alex_parent = build_tile(
        "tile-alex",
        "Alex prompt",
        "openai/gpt-4",
        "Alex response",
        Some("tile-bob"),
        Some("openai/gpt-4"),
    );
    alex_parent.twin_relationship_variant = relationship_variant("alex");
    let session = build_session(vec![bob_root, alex_parent]);
    let mut request = build_request(
        "Newest Alex prompt",
        "tile-alex",
        "openai/gpt-4",
        ContextMode::TwinHistory,
    );
    request.twin_relationship_variant = relationship_variant("alex");

    let error = build_selected_parent_chain(&session, &request).unwrap_err();

    assert!(error.contains("relationship context"));
}

#[test]
fn twin_history_parent_chain_rejects_both_context_version_relationship_mismatches() {
    for (variant, context_version) in [
        (
            crate::models::twin_state::RelationshipVariant::global(),
            "relationship-v4-reviewed-projection-history",
        ),
        (
            relationship_variant("alex"),
            "global-v4-reviewed-projection-history",
        ),
    ] {
        let mut tile = build_tile(
            "tile-version-mismatch",
            "Prior prompt",
            "openai/gpt-4",
            "Prior response",
            None,
            None,
        );
        tile.twin_relationship_variant = variant.clone();
        tile.twin_evidence_snapshot = Some(twin_evidence(variant.clone(), context_version));
        let session = build_session(vec![tile]);
        let mut request = build_request(
            "Do not use mismatched evidence",
            "tile-version-mismatch",
            "openai/gpt-4",
            ContextMode::TwinHistory,
        );
        request.twin_relationship_variant = variant;

        let error = build_selected_parent_chain(&session, &request).unwrap_err();

        assert!(error.contains("context version"));
    }
}

#[test]
fn twin_history_parent_chain_rejects_a_pending_parent_response() {
    let mut tile = build_tile(
        "tile-pending",
        "Pending prompt",
        "openai/gpt-4",
        "Partial response",
        None,
        None,
    );
    tile.responses.get_mut("openai/gpt-4").unwrap().status =
        crate::models::canvas::ResponseStatus::Pending;
    let session = build_session(vec![tile]);
    let request = build_request(
        "Do not use pending history",
        "tile-pending",
        "openai/gpt-4",
        ContextMode::TwinHistory,
    );

    let error = build_selected_parent_chain(&session, &request).unwrap_err();

    assert!(error.contains("completed"));
}

#[test]
fn twin_history_parent_chain_rejects_an_errored_parent_response() {
    let mut tile = build_tile(
        "tile-error",
        "Errored prompt",
        "openai/gpt-4",
        "Provider failed",
        None,
        None,
    );
    let response = tile.responses.get_mut("openai/gpt-4").unwrap();
    response.status = crate::models::canvas::ResponseStatus::Error;
    response.error = Some("Provider failed".into());
    let session = build_session(vec![tile]);
    let request = build_request(
        "Do not use errored history",
        "tile-error",
        "openai/gpt-4",
        ContextMode::TwinHistory,
    );

    let error = build_selected_parent_chain(&session, &request).unwrap_err();

    assert!(error.contains("completed"));
}

#[test]
fn twin_history_parent_chain_rejects_a_completed_empty_parent_response() {
    let tile = build_tile(
        "tile-empty",
        "Empty prompt",
        "openai/gpt-4",
        "   ",
        None,
        None,
    );
    let session = build_session(vec![tile]);
    let request = build_request(
        "Do not use empty history",
        "tile-empty",
        "openai/gpt-4",
        ContextMode::TwinHistory,
    );

    let error = build_selected_parent_chain(&session, &request).unwrap_err();

    assert!(error.contains("non-empty"));
}

#[test]
fn test_build_full_history_messages_interleaves_user_and_assistant_turns() {
    let session = build_session(vec![
        build_tile(
            "tile-1",
            "Root prompt",
            "openai/gpt-4",
            "Root response",
            None,
            None,
        ),
        build_tile(
            "tile-2",
            "Branch prompt",
            "openai/gpt-4",
            "Branch response",
            Some("tile-1"),
            Some("openai/gpt-4"),
        ),
    ]);
    let request = build_request(
        "Final prompt",
        "tile-2",
        "openai/gpt-4",
        ContextMode::FullHistory,
    );

    let messages = build_full_history_messages(&session, &request).unwrap();

    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Root prompt");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "Root response");
    assert_eq!(messages[2].content, "Branch prompt");
    assert_eq!(messages[3].content, "Branch response");
    assert_eq!(messages[4].content, "Final prompt");
}

#[test]
fn test_root_prompt_without_parent_ids_ignores_unrelated_canvas_tiles() {
    let session = build_session(vec![
        build_tile(
            "tile-1",
            "Unrelated root prompt",
            "openai/gpt-4",
            "Unrelated root response",
            None,
            None,
        ),
        build_tile(
            "tile-2",
            "Unrelated branch prompt",
            "openai/gpt-4",
            "Unrelated branch response",
            Some("tile-1"),
            Some("openai/gpt-4"),
        ),
    ]);
    let request = build_root_request("Fresh root prompt", ContextMode::None);

    let messages = build_canvas_messages(&session, &request).unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Fresh root prompt");
}

#[test]
fn test_build_compact_history_messages_summarizes_older_turns() {
    let session = build_session(vec![
        build_tile(
            "tile-1",
            "Prompt 1",
            "openai/gpt-4",
            "Response 1",
            None,
            None,
        ),
        build_tile(
            "tile-2",
            "Prompt 2",
            "openai/gpt-4",
            "Response 2",
            Some("tile-1"),
            Some("openai/gpt-4"),
        ),
        build_tile(
            "tile-3",
            "Prompt 3",
            "openai/gpt-4",
            "Response 3",
            Some("tile-2"),
            Some("openai/gpt-4"),
        ),
        build_tile(
            "tile-4",
            "Prompt 4",
            "openai/gpt-4",
            "Response 4",
            Some("tile-3"),
            Some("openai/gpt-4"),
        ),
    ]);
    let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);

    let messages = build_compact_history_messages(&session, &request).unwrap();

    assert_eq!(messages.len(), 6);
    assert!(messages[0]
        .content
        .contains("Conversation summary before the most recent turns"));
    assert!(messages[0].content.contains("Prompt 1"));
    assert!(messages[1].content.contains("Prompt 3"));
    assert!(messages[2].content.contains("Response 3"));
    assert_eq!(messages[5].content, "Prompt 5");
}

#[test]
fn test_build_selected_parent_chain_errors_when_parent_response_is_missing() {
    let session = build_session(vec![
        build_tile(
            "tile-1",
            "Root prompt",
            "openai/gpt-4",
            "Root response",
            None,
            None,
        ),
        build_tile(
            "tile-2",
            "Branch prompt",
            "anthropic/claude",
            "Branch response",
            Some("tile-1"),
            Some("openai/gpt-4"),
        ),
    ]);
    let request = build_request(
        "Next prompt",
        "tile-2",
        "openai/gpt-4",
        ContextMode::FullHistory,
    );

    let err = build_selected_parent_chain(&session, &request).unwrap_err();

    assert!(err.contains("Parent response not found"));
}

#[test]
fn test_should_use_retrieved_notes_accepts_relevant_matches() {
    let results = vec![build_retrieval_result(
        "note-1",
        "Mirofish architecture ideas",
        "A note about robust social media posting architecture.",
        12.0,
        &["keyword match"],
    )];

    let decision = should_use_retrieved_notes(
        "How can I make the Mirofish social media architecture more robust?",
        &results,
    );

    assert_eq!(decision, RetrievalDecisionReason::UseRetrievedNotes);
}

#[test]
fn test_should_use_retrieved_notes_rejects_off_topic_matches() {
    let results = vec![build_retrieval_result(
        "note-1",
        "Claude skills overview",
        "General AI skills and coding workflow tips.",
        72.0,
        &["keyword match", "hub (5 backlinks)"],
    )];

    let decision = should_use_retrieved_notes(
        "How can I make the Mirofish social media architecture more robust?",
        &results,
    );

    assert_eq!(decision, RetrievalDecisionReason::NoLexicalOverlap);
}

#[test]
fn test_should_use_retrieved_notes_rejects_graph_only_results() {
    let results = vec![build_retrieval_result(
        "note-1",
        "Mirofish architecture",
        "A note about robust social media posting architecture.",
        20.0,
        &["graph neighbor (1 hop)", "hub (4 backlinks)"],
    )];

    let decision = should_use_retrieved_notes(
        "How can I make the Mirofish social media architecture more robust?",
        &results,
    );

    assert_eq!(decision, RetrievalDecisionReason::NoKeywordMatch);
}
