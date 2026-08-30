use super::super::prediction_terminality::{
    fail_requested_prediction_if_same_root, is_same_prediction_root,
};
use super::TWIN_CONTEXT_VERSION;
use crate::models::twin::{DecisionEpisodeCreate, PrimitiveDecisionAssessment};
use crate::models::twin_event::ContentDigest;
use crate::services::vault_namespace::VaultAuthorityTokenV1;

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

#[tokio::test]
async fn requested_prediction_terminalization_is_idempotent_on_the_live_store() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
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
