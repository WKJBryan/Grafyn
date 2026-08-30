use crate::services::twin::TwinStore;
use crate::services::vault_namespace::VaultAuthorityTokenV1;
use crate::AppState;
use std::sync::Arc;
use tokio::sync::RwLock;

pub(super) fn is_same_prediction_root(
    request: &VaultAuthorityTokenV1,
    current: &VaultAuthorityTokenV1,
) -> bool {
    request.root_scope == current.root_scope && request.lease_epoch_uuid == current.lease_epoch_uuid
}

/// Best-effort terminalization for a durable `requested` prediction. A content
/// generation change within the same vault is recoverable: re-read the episode
/// under the latest exact authority and make the transition idempotently. A
/// root/lease change is a genuine vault switch, so the old vault is untouched.
pub(super) async fn fail_requested_prediction_if_same_root(
    root_state: &AppState,
    twin_store: &Arc<RwLock<TwinStore>>,
    episode_id: &str,
    request_root: &VaultAuthorityTokenV1,
    reason: &str,
) {
    let root_guard = match crate::commands::acquire_root_epoch(root_state).await {
        Ok(guard) => guard,
        Err(error) => {
            log::warn!(
                "Could not terminalize sealed prediction {episode_id} after {reason}: {error}"
            );
            return;
        }
    };
    if !is_same_prediction_root(request_root, root_guard.authority()) {
        log::warn!(
            "Sealed prediction {episode_id} remains in its former vault after root transition"
        );
        return;
    }

    let commit = {
        let mut store = twin_store.write().await;
        store.mark_twin_prediction_failed_with_commit(episode_id)
    };
    drop(root_guard);
    match commit {
        Ok(commit) => {
            crate::commands::acknowledge_reported_repair(
                crate::commands::repair_after_authority_mutation(
                    root_state,
                    &commit,
                    "sealed prediction failure",
                )
                .await,
            );
        }
        Err(error) => {
            let repair_commit = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .and_then(|error| error.authority_advanced_commit());
            if let Some(commit) = repair_commit {
                crate::commands::acknowledge_reported_repair(
                    crate::commands::repair_after_authority_mutation(
                        root_state,
                        &commit,
                        "aborted sealed prediction failure",
                    )
                    .await,
                );
            }
            log::warn!(
                "Failed to terminalize sealed prediction {episode_id} after {reason}: {error}"
            );
        }
    }
}
