use crate::models::mutation::CommittedMutationWarningV1;
use crate::services::sync::engine::{SyncConflict, SyncDrainReport, SyncStatus};
use crate::services::sync::operation_store::{MAX_BATCH_ENVELOPE_BYTES, MAX_OPERATIONS_PER_BATCH};
use crate::AppState;
use grafyn_sync_protocol::MAX_ENVELOPE_JSON_BYTES;
use serde::{Deserialize, Serialize};
use tauri::State;

const SYNC_BUNDLE_SCHEMA_VERSION: u16 = 1;
const SYNC_STATUS_UNAVAILABLE: &str = "Sync foundation is unavailable.";
const SYNC_RECOVERY_PENDING: &str = "Sync recovery is pending.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatusKind {
    NotProvisioned,
    LocalOnly,
    Pending,
    Conflict,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncStatusView {
    pub status: SyncStatusKind,
    pub provisioned: bool,
    pub outbox_operations: usize,
    pub pending_operations: usize,
    pub conflicts: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncConflictView {
    pub note_id: String,
    pub head_operation_ids: Vec<String>,
    pub selected_operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncCiphertextBundleV1 {
    pub schema_version: u16,
    pub envelopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncImportResult {
    pub received: usize,
    pub duplicates: usize,
    pub applied: usize,
    pub deferred: usize,
    pub recovery_pending: bool,
    pub warning: Option<CommittedMutationWarningV1>,
    pub status: SyncStatusView,
}

#[tauri::command]
pub async fn get_sync_status(state: State<'_, AppState>) -> Result<SyncStatusView, String> {
    let Some(engine) = state.sync_engine.as_ref().cloned() else {
        return Ok(unavailable_status());
    };
    let root_ticket = match crate::commands::acquire_root_epoch(state.inner()).await {
        Ok(ticket) => ticket,
        Err(_) => return Ok(unavailable_status()),
    };
    let engine_status = match engine.status() {
        Ok(status) => status_view(status),
        Err(_) => return Ok(unavailable_status()),
    };
    let ready = namespace_is_ready(state.inner(), root_ticket.authority()).await;
    if root_ticket.finish(state.inner()).await.is_err() {
        return Ok(unavailable_status());
    }
    Ok(if ready {
        engine_status
    } else {
        recovery_pending_status_view(engine_status)
    })
}

#[tauri::command]
pub async fn list_sync_conflicts(
    state: State<'_, AppState>,
) -> Result<Vec<SyncConflictView>, String> {
    let engine = require_engine(state.inner())?;
    let root_ticket = crate::commands::acquire_root_epoch(state.inner())
        .await
        .map_err(|_| "sync conflicts are unavailable".to_string())?;
    let conflicts = engine
        .conflicts()
        .map_err(|_| "sync conflicts are unavailable".to_string())?
        .into_iter()
        .map(conflict_view)
        .collect();
    root_ticket
        .finish(state.inner())
        .await
        .map_err(|_| "sync conflicts are unavailable".to_string())?;
    Ok(conflicts)
}

#[tauri::command]
pub async fn export_sync_outbox(
    state: State<'_, AppState>,
) -> Result<SyncCiphertextBundleV1, String> {
    let engine = require_engine(state.inner())?;
    let root_ticket = crate::commands::acquire_root_epoch(state.inner())
        .await
        .map_err(|_| "encrypted sync export is unavailable".to_string())?;
    let bundle = build_export_bundle(
        engine
            .export_outbox()
            .map_err(|_| "encrypted sync export is unavailable".to_string())?,
    )?;
    root_ticket
        .finish(state.inner())
        .await
        .map_err(|_| "encrypted sync export is unavailable".to_string())?;
    Ok(bundle)
}

fn build_export_bundle(envelope_bytes: Vec<Vec<u8>>) -> Result<SyncCiphertextBundleV1, String> {
    validate_envelope_bounds(envelope_bytes.iter().map(Vec::len))
        .map_err(|reason| format!("encrypted sync outbox {reason}; no bundle was exported"))?;
    let envelopes = envelope_bytes
        .into_iter()
        .map(|bytes| {
            String::from_utf8(bytes)
                .map_err(|_| "stored sync ciphertext is not valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SyncCiphertextBundleV1 {
        schema_version: SYNC_BUNDLE_SCHEMA_VERSION,
        envelopes,
    })
}

#[tauri::command]
pub async fn import_sync_envelopes(
    bundle: SyncCiphertextBundleV1,
    state: State<'_, AppState>,
) -> Result<SyncImportResult, String> {
    let envelopes = validate_bundle(bundle)?;
    drain_sync_envelopes(state.inner(), &envelopes).await
}

#[tauri::command]
pub async fn rebuild_sync_state(state: State<'_, AppState>) -> Result<SyncImportResult, String> {
    drain_sync_envelopes(state.inner(), &[]).await
}

async fn drain_sync_envelopes(
    state: &AppState,
    envelopes: &[Vec<u8>],
) -> Result<SyncImportResult, String> {
    let engine = require_engine(state)?;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .cloned()
        .ok_or_else(|| "sync foundation is unavailable".to_string())?;
    let root_ticket = crate::commands::acquire_root_epoch(state)
        .await
        .map_err(|_| "sync foundation is unavailable".to_string())?;
    let current_authority = root_ticket.authority().clone();
    let report = engine
        .receive_envelopes(&coordinator, envelopes)
        .map_err(|_| "encrypted sync bundle could not be imported".to_string())?;

    let mut recovery_pending = false;
    let mut warning = None;
    let namespace_ready = if report.authority_token.is_some() {
        false
    } else {
        namespace_is_ready(state, &current_authority).await
    };
    if drain_requires_repair(report.authority_token.is_some(), namespace_ready) {
        let authority = report
            .authority_token
            .as_ref()
            .unwrap_or(&current_authority);
        if crate::commands::rebuild_and_publish_remote_authority(state, authority)
            .await
            .is_err()
        {
            log::error!("Remote sync committed but derived-state repair failed");
            recovery_pending = true;
            warning = Some(crate::commands::publish_committed_warning(state));
        }
        drop(root_ticket);
    } else {
        root_ticket
            .finish(state)
            .await
            .map_err(|_| "sync foundation changed while importing".to_string())?;
    }

    let status = match engine.status() {
        Ok(status) if recovery_pending => recovery_pending_status(status),
        Ok(status) => status_view(status),
        Err(_) => {
            log::error!("Failed to read sync status after import");
            recovery_pending = recovery_pending || report.authority_token.is_some();
            if recovery_pending && warning.is_none() {
                warning = Some(crate::commands::publish_committed_warning(state));
            }
            unavailable_status()
        }
    };
    Ok(import_result(report, recovery_pending, warning, status))
}

async fn namespace_is_ready(
    state: &AppState,
    authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> bool {
    if state.loaded_authority.read().await.as_ref() != Some(authority) {
        return false;
    }
    state
        .mutation_coordinator
        .as_ref()
        .is_some_and(|coordinator| {
            coordinator
                .validate_authority_token(authority, true)
                .is_ok()
        })
}

fn drain_requires_repair(authority_advanced: bool, namespace_ready: bool) -> bool {
    authority_advanced || !namespace_ready
}

fn require_engine(
    state: &AppState,
) -> Result<std::sync::Arc<crate::services::sync::engine::SyncEngine>, String> {
    state
        .sync_engine
        .as_ref()
        .cloned()
        .ok_or_else(|| "sync foundation is unavailable".to_string())
}

fn validate_envelope_bounds(
    envelope_lengths: impl IntoIterator<Item = usize>,
) -> Result<(), String> {
    let mut total_bytes = 0usize;
    for (index, envelope_bytes) in envelope_lengths.into_iter().enumerate() {
        if index >= MAX_OPERATIONS_PER_BATCH {
            return Err(format!(
                "contains more than {MAX_OPERATIONS_PER_BATCH} envelopes"
            ));
        }
        if envelope_bytes > MAX_ENVELOPE_JSON_BYTES {
            return Err(format!(
                "contains an envelope larger than {MAX_ENVELOPE_JSON_BYTES} bytes"
            ));
        }
        total_bytes += envelope_bytes;
        if total_bytes > MAX_BATCH_ENVELOPE_BYTES {
            return Err(format!(
                "exceeds the {MAX_BATCH_ENVELOPE_BYTES}-byte aggregate limit"
            ));
        }
    }
    Ok(())
}

fn validate_bundle(bundle: SyncCiphertextBundleV1) -> Result<Vec<Vec<u8>>, String> {
    if bundle.schema_version != SYNC_BUNDLE_SCHEMA_VERSION {
        return Err("unsupported sync ciphertext bundle version".to_string());
    }
    validate_envelope_bounds(bundle.envelopes.iter().map(String::len))
        .map_err(|reason| format!("sync ciphertext bundle {reason}"))?;
    Ok(bundle
        .envelopes
        .into_iter()
        .map(String::into_bytes)
        .collect())
}

fn status_view(status: SyncStatus) -> SyncStatusView {
    let kind = if !status.provisioned {
        SyncStatusKind::NotProvisioned
    } else if status.conflicts > 0 {
        SyncStatusKind::Conflict
    } else if status.pending_operations > 0 || status.outbox_operations > 0 {
        SyncStatusKind::Pending
    } else {
        SyncStatusKind::LocalOnly
    };
    SyncStatusView {
        status: kind,
        provisioned: status.provisioned,
        outbox_operations: status.outbox_operations,
        pending_operations: status.pending_operations,
        conflicts: status.conflicts,
        error: None,
    }
}

fn unavailable_status() -> SyncStatusView {
    SyncStatusView {
        status: SyncStatusKind::Error,
        provisioned: false,
        outbox_operations: 0,
        pending_operations: 0,
        conflicts: 0,
        error: Some(SYNC_STATUS_UNAVAILABLE.to_string()),
    }
}

fn recovery_pending_status(status: SyncStatus) -> SyncStatusView {
    recovery_pending_status_view(status_view(status))
}

fn recovery_pending_status_view(mut status: SyncStatusView) -> SyncStatusView {
    status.status = SyncStatusKind::Error;
    status.error = Some(SYNC_RECOVERY_PENDING.to_string());
    status
}

fn conflict_view(conflict: SyncConflict) -> SyncConflictView {
    SyncConflictView {
        note_id: conflict.note_key,
        head_operation_ids: conflict
            .head_ids
            .into_iter()
            .map(|operation_id| operation_id.to_string())
            .collect(),
        selected_operation_id: conflict.selected_id.to_string(),
    }
}

fn import_result(
    report: SyncDrainReport,
    recovery_pending: bool,
    warning: Option<CommittedMutationWarningV1>,
    status: SyncStatusView,
) -> SyncImportResult {
    SyncImportResult {
        received: report.received,
        duplicates: report.duplicates,
        applied: report.applied,
        deferred: report.deferred,
        recovery_pending,
        warning,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grafyn_sync_protocol::OperationId;

    fn operation_id(byte: u8) -> OperationId {
        OperationId::parse_hex(&format!("{byte:02x}").repeat(32)).unwrap()
    }

    #[test]
    fn status_precedence_is_not_provisioned_then_conflict_then_pending_then_local_only() {
        let not_provisioned = status_view(SyncStatus {
            provisioned: false,
            outbox_operations: 4,
            pending_operations: 3,
            conflicts: 2,
        });
        assert_eq!(not_provisioned.status, SyncStatusKind::NotProvisioned);

        let conflict = status_view(SyncStatus {
            provisioned: true,
            outbox_operations: 4,
            pending_operations: 3,
            conflicts: 2,
        });
        assert_eq!(conflict.status, SyncStatusKind::Conflict);

        let pending = status_view(SyncStatus {
            provisioned: true,
            outbox_operations: 1,
            pending_operations: 0,
            conflicts: 0,
        });
        assert_eq!(pending.status, SyncStatusKind::Pending);

        let local_only = status_view(SyncStatus {
            provisioned: true,
            outbox_operations: 0,
            pending_operations: 0,
            conflicts: 0,
        });
        assert_eq!(local_only.status, SyncStatusKind::LocalOnly);
    }

    #[test]
    fn unavailable_status_is_fixed_and_redacted() {
        let status = unavailable_status();
        let json = serde_json::to_value(&status).unwrap();

        assert_eq!(status.status, SyncStatusKind::Error);
        assert_eq!(status.error.as_deref(), Some(SYNC_STATUS_UNAVAILABLE));
        assert_eq!(json["outboxOperations"], 0);
        assert_eq!(json["pendingOperations"], 0);
        let encoded = json.to_string();
        assert!(!encoded.contains("root_key"));
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("vault_path"));
    }

    #[test]
    fn recovery_pending_status_is_sanitized_and_preserves_engine_counts() {
        let status = recovery_pending_status(SyncStatus {
            provisioned: true,
            outbox_operations: 4,
            pending_operations: 2,
            conflicts: 1,
        });

        assert_eq!(status.status, SyncStatusKind::Error);
        assert!(status.provisioned);
        assert_eq!(status.outbox_operations, 4);
        assert_eq!(status.pending_operations, 2);
        assert_eq!(status.conflicts, 1);
        assert_eq!(status.error.as_deref(), Some(SYNC_RECOVERY_PENDING));
    }

    #[test]
    fn exact_or_empty_drain_repairs_whenever_current_readiness_is_missing() {
        assert!(drain_requires_repair(true, true));
        assert!(drain_requires_repair(true, false));
        assert!(drain_requires_repair(false, false));
        assert!(!drain_requires_repair(false, true));
    }

    #[tokio::test]
    async fn empty_command_drain_repairs_current_authority_with_missing_readiness() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        std::fs::create_dir(&vault_path).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
        let durable_settings = crate::models::settings::UserSettings {
            vault_path: Some(vault_path.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        let settings = crate::services::settings::SettingsService::for_test(
            temp.path().join("settings.json"),
            durable_settings.clone(),
        );
        settings
            .root_transition_store()
            .unwrap()
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    durable_settings,
                ),
            )
            .unwrap();
        crate::services::sync::vault_keys::provision_vault_root_key(
            settings.secret_store().as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &grafyn_sync_protocol::VaultRootKey::from_bytes([0x63; 32]),
        )
        .unwrap();

        let state = crate::build_app_state(settings, None).unwrap();
        let startup_error = state.mutation_startup_error.read().await.clone();
        let coordinator = state.mutation_coordinator.as_ref().unwrap_or_else(|| {
            panic!("test app state must have a mutation coordinator: {startup_error:?}")
        });
        let engine = state
            .sync_engine
            .as_ref()
            .unwrap_or_else(|| panic!("test app state must have a sync engine: {startup_error:?}"));
        let authority = coordinator.current_authority_token().unwrap();
        let before = engine.status().unwrap();
        coordinator
            .invalidate_namespace_before_recovery(&authority)
            .unwrap();
        *state.loaded_authority.write().await = None;
        assert!(coordinator
            .validate_authority_token(&authority, true)
            .is_err());

        let result = drain_sync_envelopes(&state, &[]).await.unwrap();

        assert_eq!(result.received, 0);
        assert_eq!(result.duplicates, 0);
        assert_eq!(result.applied, 0);
        assert!(!result.recovery_pending);
        assert_ne!(result.status.status, SyncStatusKind::Error);
        assert_eq!(engine.status().unwrap(), before);
        assert_eq!(
            state.loaded_authority.read().await.as_ref(),
            Some(&authority)
        );
        coordinator
            .validate_authority_token(&authority, true)
            .unwrap();
    }

    #[test]
    fn bundle_validation_rejects_unsupported_oversized_and_overcount_inputs() {
        assert!(validate_bundle(SyncCiphertextBundleV1 {
            schema_version: 2,
            envelopes: vec![],
        })
        .is_err());
        assert!(validate_bundle(SyncCiphertextBundleV1 {
            schema_version: 1,
            envelopes: vec!["x".repeat(MAX_ENVELOPE_JSON_BYTES + 1)],
        })
        .is_err());
        assert!(validate_bundle(SyncCiphertextBundleV1 {
            schema_version: 1,
            envelopes: vec![
                String::new();
                crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH + 1
            ],
        })
        .is_err());

        let valid = validate_bundle(SyncCiphertextBundleV1 {
            schema_version: 1,
            envelopes: vec!["{\"ciphertext\":\"opaque\"}".to_string()],
        })
        .unwrap();
        assert_eq!(valid, vec![b"{\"ciphertext\":\"opaque\"}".to_vec()]);
    }

    #[test]
    fn bundle_validation_rejects_engine_incompatible_aggregate_bytes() {
        let envelope_bytes = crate::services::sync::operation_store::MAX_BATCH_ENVELOPE_BYTES
            / crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
            + 1;
        assert!(envelope_bytes < MAX_ENVELOPE_JSON_BYTES);

        assert!(validate_bundle(SyncCiphertextBundleV1 {
            schema_version: SYNC_BUNDLE_SCHEMA_VERSION,
            envelopes: vec![
                "x".repeat(envelope_bytes);
                crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
            ],
        })
        .is_err());
    }

    #[test]
    fn envelope_bounds_accept_the_engine_limits_exactly() {
        let envelope_bytes = crate::services::sync::operation_store::MAX_BATCH_ENVELOPE_BYTES
            / crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH;
        assert_eq!(
            envelope_bytes * crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH,
            crate::services::sync::operation_store::MAX_BATCH_ENVELOPE_BYTES
        );

        assert!(validate_envelope_bounds(vec![
            envelope_bytes;
            crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
        ])
        .is_ok());
        assert!(validate_envelope_bounds([MAX_ENVELOPE_JSON_BYTES]).is_ok());
    }

    #[test]
    fn outbox_export_rejects_engine_incompatible_operation_count() {
        assert!(build_export_bundle(vec![
            Vec::new();
            crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
                + 1
        ])
        .is_err());
    }

    #[test]
    fn outbox_export_rejects_engine_incompatible_aggregate_bytes() {
        let envelope_bytes = crate::services::sync::operation_store::MAX_BATCH_ENVELOPE_BYTES
            / crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
            + 1;

        assert!(build_export_bundle(vec![
            vec![b'x'; envelope_bytes];
            crate::services::sync::operation_store::MAX_OPERATIONS_PER_BATCH
        ])
        .is_err());
    }

    #[test]
    fn conflict_view_contains_only_note_and_operation_identifiers() {
        let first = operation_id(0x11);
        let second = operation_id(0x22);
        let view = conflict_view(SyncConflict {
            note_key: "stable-note-id".to_string(),
            head_ids: vec![first, second],
            selected_id: second,
        });
        let json = serde_json::to_string(&view).unwrap();

        assert_eq!(view.note_id, "stable-note-id");
        assert_eq!(
            view.head_operation_ids,
            vec![first.to_string(), second.to_string()]
        );
        assert_eq!(view.selected_operation_id, second.to_string());
        assert!(!json.contains("markdown"));
        assert!(!json.contains("ciphertext"));
        assert!(!json.contains("public_key"));
    }
}
