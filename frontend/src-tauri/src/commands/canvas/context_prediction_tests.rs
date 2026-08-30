use super::super::prediction_terminality::{
    fail_requested_prediction_if_same_root, is_same_prediction_root,
};
use super::TWIN_CONTEXT_VERSION;
use crate::models::twin::{DecisionEpisodeCreate, PrimitiveDecisionAssessment};
use crate::models::twin_event::ContentDigest;
use crate::services::twin::TwinStore;
use crate::services::vault_namespace::VaultAuthorityTokenV1;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

fn build_prediction_test_state() -> (crate::AppState, TempDir, TempDir) {
    let (mut state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    state.twin_store = Arc::new(RwLock::new(TwinStore::with_event_recorder(
        crate::models::settings::twin_data_path_for_vault(data.path(), vault.path()).unwrap(),
        data.path().join("twin"),
        coordinator,
    )));
    (state, vault, data)
}

#[test]
fn prediction_failure_can_follow_same_root_generation_but_not_a_root_switch() {
    let request = VaultAuthorityTokenV1 {
        root_scope: ContentDigest::parse("0".repeat(64)).unwrap(),
        lease_epoch_uuid: "lease-a".to_string(),
        authority_generation: 7,
    };
    let mut current = request.clone();
    current.authority_generation += 1;
    assert!(is_same_prediction_root(&request, &current));

    current.root_scope = ContentDigest::parse("1".repeat(64)).unwrap();
    assert!(!is_same_prediction_root(&request, &current));

    current = request.clone();
    current.lease_epoch_uuid = "lease-b".to_string();
    assert!(!is_same_prediction_root(&request, &current));
}

#[test]
fn sealed_prediction_prompt_uses_advisor_framing_without_identity() {
    let options = vec!["Ship now".to_string(), "Wait a sprint".to_string()];
    let user_message = super::build_twin_prediction_user_message(
        &crate::models::twin::ConstitutionSetup::default(),
        "Ship the importer before polish?",
        &options,
        None,
    );

    assert!(!user_message.contains("I am "));
    assert!(user_message.contains("best fits this decision-maker's"));
    assert!(user_message.contains("predicted_option"));
}

#[tokio::test]
async fn requested_prediction_terminalization_is_idempotent_on_the_live_store() {
    let (state, _vault, _data) = build_prediction_test_state();
    let episode_id = "decision-terminal";
    state
        .twin_store
        .write()
        .await
        .record_decision_episode(DecisionEpisodeCreate {
            id: episode_id.to_string(),
            session_id: "session-terminal".to_string(),
            tile_id: "tile-terminal".to_string(),
            decision: "Choose one".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: PrimitiveDecisionAssessment::default(),
            context_version: Some(TWIN_CONTEXT_VERSION.to_string()),
        })
        .unwrap();
    let request_root = crate::commands::capture_root_epoch(&state).unwrap();

    fail_requested_prediction_if_same_root(
        &state,
        &state.twin_store,
        episode_id,
        &request_root,
        "test abandonment",
    )
    .await;
    let terminal_generation = crate::commands::capture_root_epoch(&state)
        .unwrap()
        .authority_generation;
    assert_eq!(
        state
            .twin_store
            .read()
            .await
            .get_decision_episode(episode_id)
            .unwrap()
            .prediction_status
            .as_deref(),
        Some("failed")
    );

    fail_requested_prediction_if_same_root(
        &state,
        &state.twin_store,
        episode_id,
        &request_root,
        "duplicate abandonment",
    )
    .await;
    assert_eq!(
        crate::commands::capture_root_epoch(&state)
            .unwrap()
            .authority_generation,
        terminal_generation
    );
}

#[tokio::test]
async fn requested_prediction_terminalization_replans_after_same_root_peer_write() {
    let (state, vault, _data) = build_prediction_test_state();
    let episode_id = "decision-peer-write";
    state
        .twin_store
        .write()
        .await
        .record_decision_episode(DecisionEpisodeCreate {
            id: episode_id.to_string(),
            session_id: "session-peer-write".to_string(),
            tile_id: "tile-peer-write".to_string(),
            decision: "Choose one".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: PrimitiveDecisionAssessment::default(),
            context_version: Some(TWIN_CONTEXT_VERSION.to_string()),
        })
        .unwrap();
    let request_root = crate::commands::capture_root_epoch(&state).unwrap();

    let _peer_commit = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            crate::models::twin_event::CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "unrelated-peer.md",
                "# Unrelated peer",
            )],
            Vec::new(),
        )
        .unwrap();

    fail_requested_prediction_if_same_root(
        &state,
        &state.twin_store,
        episode_id,
        &request_root,
        "test abandonment",
    )
    .await;

    assert!(vault.path().join("unrelated-peer.md").exists());
    assert_eq!(
        state
            .twin_store
            .read()
            .await
            .get_decision_episode(episode_id)
            .unwrap()
            .prediction_status
            .as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn requested_prediction_terminalization_abstains_after_root_switch() {
    let (state, _vault, _data) = build_prediction_test_state();
    let replacement_vault = TempDir::new().unwrap();
    let episode_id = "decision-root-switch";
    state
        .twin_store
        .write()
        .await
        .record_decision_episode(DecisionEpisodeCreate {
            id: episode_id.to_string(),
            session_id: "session-root-switch".to_string(),
            tile_id: "tile-root-switch".to_string(),
            decision: "Choose one".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: PrimitiveDecisionAssessment::default(),
            context_version: Some(TWIN_CONTEXT_VERSION.to_string()),
        })
        .unwrap();
    let request_root = crate::commands::capture_root_epoch(&state).unwrap();
    state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .retarget_markdown_root(replacement_vault.path())
        .unwrap();

    fail_requested_prediction_if_same_root(
        &state,
        &state.twin_store,
        episode_id,
        &request_root,
        "test abandonment",
    )
    .await;

    assert_eq!(
        state
            .twin_store
            .read()
            .await
            .get_decision_episode(episode_id)
            .unwrap()
            .prediction_status
            .as_deref(),
        Some("requested")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_authority_prediction_abort_repairs_readiness_without_claiming_failure() {
    let (state, vault, data) = build_prediction_test_state();
    let state = Arc::new(state);
    let episode_id = "decision-guard-drift";
    state
        .twin_store
        .write()
        .await
        .record_decision_episode(DecisionEpisodeCreate {
            id: episode_id.to_string(),
            session_id: "session-guard-drift".to_string(),
            tile_id: "tile-guard-drift".to_string(),
            decision: "Choose one".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: PrimitiveDecisionAssessment::default(),
            context_version: Some(TWIN_CONTEXT_VERSION.to_string()),
        })
        .unwrap();
    let request_root = crate::commands::capture_root_epoch(&state).unwrap();
    let decision_path =
        crate::models::settings::twin_data_path_for_vault(data.path(), vault.path())
            .unwrap()
            .join("decisions")
            .join(format!("{episode_id}.json"));
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .pause_after_authority_advance_once(entered.clone(), resume.clone());

    let worker_state = state.clone();
    let worker_request_root = request_root.clone();
    let worker = tokio::spawn(async move {
        fail_requested_prediction_if_same_root(
            &worker_state,
            &worker_state.twin_store,
            episode_id,
            &worker_request_root,
            "test guard drift",
        )
        .await;
    });
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();
    let mut external: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&decision_path).unwrap()).unwrap();
    external["decision"] = serde_json::Value::String("Externally changed".to_string());
    std::fs::write(
        &decision_path,
        serde_json::to_vec_pretty(&external).unwrap(),
    )
    .unwrap();
    tokio::task::spawn_blocking(move || resume.wait())
        .await
        .unwrap();
    worker.await.unwrap();

    let current = crate::commands::capture_root_epoch(&state).unwrap();
    assert_eq!(
        current.authority_generation,
        request_root.authority_generation + 1
    );
    assert_eq!(*state.loaded_authority.read().await, Some(current));
    let episode = state
        .twin_store
        .write()
        .await
        .get_decision_episode(episode_id)
        .unwrap();
    assert_eq!(episode.prediction_status.as_deref(), Some("requested"));
    assert_eq!(episode.decision, "Externally changed");
}

#[test]
fn prediction_terminality_uses_one_fresh_serialized_plan() {
    let source = include_str!("prediction_terminality.rs");
    assert!(!source.contains("for _ in 0..2"));
    assert!(source.contains("acquire_root_epoch"));
    assert!(source.contains("mark_twin_prediction_failed_with_commit"));
}
