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
    for _ in 0..2 {
        let current = match crate::commands::capture_root_epoch(root_state) {
            Ok(current) => current,
            Err(error) => {
                log::warn!(
                    "Could not terminalize sealed prediction {episode_id} after {reason}: {error}"
                );
                return;
            }
        };
        if !is_same_prediction_root(request_root, &current) {
            log::warn!(
                "Sealed prediction {episode_id} remains in its former vault after root transition"
            );
            return;
        }
        let root_guard =
            match crate::commands::acquire_expected_root_epoch(root_state, &current).await {
                Ok(guard) => guard,
                Err(_) => continue,
            };
        let commit = {
            let mut store = twin_store.write().await;
            store.mark_twin_prediction_failed_expecting_authority(episode_id, current.clone())
        };
        drop(root_guard);
        match commit {
            Ok(commit) => {
                let _ = crate::commands::repair_after_authority_mutation(
                    root_state,
                    &commit,
                    "sealed prediction failure",
                )
                .await;
                return;
            }
            Err(error) if error.to_string().contains("authority") => continue,
            Err(error) => {
                log::warn!(
                    "Failed to terminalize sealed prediction {episode_id} after {reason}: {error}"
                );
                return;
            }
        }
    }
    log::warn!(
        "Could not terminalize sealed prediction {episode_id} after {reason}: authority kept changing"
    );
}
