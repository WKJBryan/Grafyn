use crate::models::migration::{
    MarkdownMigrationApplyResult, MarkdownMigrationPreview, MarkdownMigrationRequest,
    MarkdownMigrationStatus, VaultOptimizerDecision, VaultOptimizerInboxEntry,
    VaultOptimizerRollbackResult, VaultOptimizerSettingsUpdate, VaultOptimizerStatus,
};
use crate::models::settings::SettingsUpdate;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn preview_markdown_migration(
    vault_path: String,
    request: MarkdownMigrationRequest,
    state: State<'_, AppState>,
) -> Result<MarkdownMigrationPreview, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let authority = root_ticket.authority().clone();
    let result = {
        let service_handle = state.markdown_migration_service()?;
        let service = service_handle.read().await;
        let mut store = state.knowledge_store.write().await;
        let requested = std::fs::canonicalize(&vault_path).map_err(|error| error.to_string())?;
        let current =
            std::fs::canonicalize(store.vault_path()).map_err(|error| error.to_string())?;
        if requested != current {
            return Err("migration preview must use the active vault".into());
        }
        service
            .preview_scoped(&mut store, authority, request)
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn apply_markdown_migration(
    preview_id: String,
    request: MarkdownMigrationRequest,
    state: State<'_, AppState>,
) -> Result<MarkdownMigrationApplyResult, String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let authority = root_epoch.authority().clone();
    let outcome = {
        let service_handle = state.markdown_migration_service()?;
        let service = service_handle.read().await;
        let mut store = state.knowledge_store.write().await;
        service
            .apply_transaction(&preview_id, request.clone(), &mut store, authority)
            .map_err(|error| error.to_string())?
    };
    drop(root_epoch);
    let (mut result, _mutation_commit, outcome_authority, outcome_warning, _) =
        outcome.into_parts();
    result.warning = outcome_warning;
    let accepted_request = result.accepted_request.clone();

    let repair = if let Some(authority) = outcome_authority.as_ref() {
        crate::commands::repair_after_migration_authority_token(
            state.inner(),
            authority,
            "Markdown migration",
        )
        .await
    } else {
        crate::commands::PostAuthorityRepair::NotRequired
    };
    let repair_ready = matches!(
        repair,
        crate::commands::PostAuthorityRepair::Ready(_)
            | crate::commands::PostAuthorityRepair::NotRequired
    );
    if matches!(repair, crate::commands::PostAuthorityRepair::Unavailable(_)) {
        result.warning =
            Some(crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable());
    }

    if result.status == "applied"
        && accepted_request.as_ref().is_some_and(|request| {
            request.start_optimizer.unwrap_or(true) || request.enable_llm.unwrap_or(false)
        })
    {
        let accepted_request = accepted_request
            .as_ref()
            .expect("applied migration carries its accepted request");
        if let Err(error) = crate::commands::settings::apply_settings_update(
            state.inner(),
            SettingsUpdate {
                vault_path: None,
                openrouter_api_key: None,
                setup_completed: None,
                theme: None,
                mcp_enabled: None,
                llm_model: None,
                twin_llm_provider: None,
                ollama_base_url: None,
                ollama_model: None,
                smart_web_search: None,
                background_link_discovery_enabled: None,
                background_link_discovery_llm_enabled: None,
                background_vault_optimizer_enabled: Some(
                    accepted_request.start_optimizer.unwrap_or(true),
                ),
                background_vault_optimizer_llm_enabled: Some(
                    accepted_request.enable_llm.unwrap_or(false),
                ),
                background_vault_optimizer_budget_monthly: None,
                background_vault_optimizer_max_daily_writes: None,
                background_vault_optimizer_edit_mode: Some(
                    format!("{:?}", accepted_request.mode)
                        .to_lowercase()
                        .replace("sidecarfirst", "sidecar_first")
                        .replace("fullrewrite", "full_rewrite"),
                ),
                background_vault_optimizer_program_enabled: Some(true),
                vault_optimizer_program_path: accepted_request.program_path.clone(),
                canvas_model_presets: None,
            },
        )
        .await
        {
            log::error!("Migration optimizer settings publication failed: {error}");
            if outcome_authority.is_some() {
                result.warning = Some(
                    crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable(
                    ),
                );
            }
        }
    }

    if repair_ready && result.status == "applied" {
        let optimizer_handle = state.vault_optimizer_service()?;
        let mut optimizer = optimizer_handle.write().await;
        for note_id in result
            .touched_note_ids
            .iter()
            .chain(result.overlay_note_ids.iter())
            .chain(result.created_hub_note_ids.iter())
        {
            optimizer.enqueue_note(note_id, "migration_apply");
        }
    }

    Ok(result)
}

#[tauri::command]
pub async fn get_markdown_migration_status(
    run_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<MarkdownMigrationStatus, String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let authority = root_epoch.authority().clone();
    let result = {
        let service_handle = state.markdown_migration_service()?;
        let service = service_handle.read().await;
        service
            .status_scoped(run_id.as_deref(), &authority)
            .map_err(|error| error.to_string())?
    };
    drop(root_epoch);
    Ok(result)
}

#[tauri::command]
pub async fn rollback_markdown_migration(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<crate::models::migration::MarkdownMigrationRollbackResult, String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let authority = root_epoch.authority().clone();
    let outcome = {
        let service_handle = state.markdown_migration_service()?;
        let service = service_handle.read().await;
        let mut store = state.knowledge_store.write().await;
        service
            .rollback_transaction(&run_id, &mut store, authority)
            .map_err(|error| error.to_string())?
    };
    drop(root_epoch);
    let (mut result, _mutation_commit, outcome_authority, outcome_warning, _) =
        outcome.into_parts();
    result.warning = outcome_warning;

    if let Some(authority) = outcome_authority.as_ref() {
        let repair = crate::commands::repair_after_migration_authority_token(
            state.inner(),
            authority,
            "Markdown migration rollback",
        )
        .await;
        if matches!(repair, crate::commands::PostAuthorityRepair::Unavailable(_)) {
            result.warning = Some(
                crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable(),
            );
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_vault_optimizer_status(
    state: State<'_, AppState>,
) -> Result<VaultOptimizerStatus, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let settings = {
            let settings = state.settings_service.read().await;
            settings.get().clone()
        };
        let optimizer_handle = state.vault_optimizer_service()?;
        let mut optimizer = optimizer_handle.write().await;
        optimizer
            .with_locked_fresh_state(|optimizer| Ok(optimizer.status(&settings)))
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn update_vault_optimizer_settings(
    update: VaultOptimizerSettingsUpdate,
    state: State<'_, AppState>,
) -> Result<crate::models::settings::UserSettings, String> {
    crate::commands::settings::apply_settings_update(
        state.inner(),
        SettingsUpdate {
            vault_path: None,
            openrouter_api_key: None,
            setup_completed: None,
            theme: None,
            mcp_enabled: None,
            llm_model: None,
            twin_llm_provider: None,
            ollama_base_url: None,
            ollama_model: None,
            smart_web_search: None,
            background_link_discovery_enabled: None,
            background_link_discovery_llm_enabled: None,
            background_vault_optimizer_enabled: update.background_vault_optimizer_enabled,
            background_vault_optimizer_llm_enabled: update.background_vault_optimizer_llm_enabled,
            background_vault_optimizer_budget_monthly: update
                .background_vault_optimizer_budget_monthly,
            background_vault_optimizer_max_daily_writes: update
                .background_vault_optimizer_max_daily_writes,
            background_vault_optimizer_edit_mode: update.background_vault_optimizer_edit_mode,
            background_vault_optimizer_program_enabled: update
                .background_vault_optimizer_program_enabled,
            vault_optimizer_program_path: update.vault_optimizer_program_path,
            canvas_model_presets: None,
        },
    )
    .await
}

#[tauri::command]
pub async fn list_vault_optimizer_decisions(
    limit: Option<usize>,
    _cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<VaultOptimizerDecision>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let optimizer_handle = state.vault_optimizer_service()?;
        let mut optimizer = optimizer_handle.write().await;
        optimizer
            .with_locked_fresh_state(|optimizer| optimizer.list_decisions(limit.unwrap_or(20)))
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn get_vault_optimizer_inbox(
    status: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<VaultOptimizerInboxEntry>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let optimizer_handle = state.vault_optimizer_service()?;
        let mut optimizer = optimizer_handle.write().await;
        optimizer
            .with_locked_fresh_state(|optimizer| {
                optimizer.inbox(status.as_deref(), limit.unwrap_or(20))
            })
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

fn reconcile_optimizer_rollback_after_repair(
    result: &mut VaultOptimizerRollbackResult,
    recovery_pending: bool,
    repair: &crate::commands::PostAuthorityRepair,
) {
    match repair {
        crate::commands::PostAuthorityRepair::Ready(_)
        | crate::commands::PostAuthorityRepair::NotRequired
            if recovery_pending =>
        {
            result.rolled_back = true;
            result.message = "Optimizer change rolled back".to_string();
            result.warning = None;
        }
        crate::commands::PostAuthorityRepair::Unavailable(warning) if result.rolled_back => {
            result.warning = Some(warning.clone());
            result.message =
                "Optimizer change rolled back; derived state repair is pending".to_string();
        }
        crate::commands::PostAuthorityRepair::Unavailable(_) if recovery_pending => {
            result.message = "Optimizer rollback recovery is pending".to_string();
        }
        _ => {}
    }
}

#[tauri::command]
pub async fn rollback_vault_optimizer_change(
    change_id: String,
    state: State<'_, AppState>,
) -> Result<VaultOptimizerRollbackResult, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let expected = root_ticket.authority().clone();
    let outcome = {
        // Lock order: knowledge_store before vault_optimizer (see commands/mod.rs
        // doc comment) — must match the background worker in main.rs to avoid
        // an ABBA deadlock.
        let mut store = state.knowledge_store.write().await;
        let optimizer_handle = state.vault_optimizer_service()?;
        let mut optimizer = optimizer_handle.write().await;
        optimizer
            .rollback_change_expecting_authority(&change_id, &mut store, expected)
            .map_err(|error| error.to_string())?
    };
    drop(root_ticket);
    let (mut result, commit, outcome_warning, recovery_pending) = match outcome {
        crate::services::vault_optimizer::OptimizerRollbackMutationOutcome::NoWrite(result) => {
            return Ok(result)
        }
        crate::services::vault_optimizer::OptimizerRollbackMutationOutcome::Committed {
            result,
            commit,
            warning,
        } => (result, commit, warning, false),
        crate::services::vault_optimizer::OptimizerRollbackMutationOutcome::Partial {
            result,
            commit,
            warning,
            recovery_pending,
        } => (result, commit, Some(warning), recovery_pending),
    };
    result.warning = outcome_warning;
    let repair = crate::commands::repair_after_authority_mutation(
        state.inner(),
        &commit,
        "vault optimizer rollback",
    )
    .await;
    reconcile_optimizer_rollback_after_repair(&mut result, recovery_pending, &repair);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::services::knowledge_store::KnowledgeStore;
    use crate::services::vault_optimizer::VaultOptimizerService;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::RwLock;
    use tokio::time::{timeout, Duration};

    /// Demonstrates that the canonical `knowledge_store` → `vault_optimizer`
    /// lock order (see the doc comment in `commands/mod.rs`) is deadlock-free
    /// under contention: two concurrent tasks repeatedly acquire both locks
    /// in that shared order, using the real service types over real
    /// tempdir-backed stores.
    ///
    /// Scope note — what this does NOT guard: both tasks below hand-inline
    /// the acquisition pattern; they do not drive the actual production call
    /// sites (`main.rs::start_vault_optimizer_worker` and
    /// `rollback_vault_optimizer_change` above). If a production call site's
    /// order drifts back to `vault_optimizer` → `knowledge_store`, this test
    /// still passes. The production sites are kept in sync by the
    /// canonical-order doc comment in `commands/mod.rs` plus code review,
    /// not by this test.
    ///
    /// Mechanics: the multi-thread runtime plus a `yield_now` between the
    /// two acquisitions in each task are required to force real interleaving
    /// — without both, an uncontended `.write().await` never actually
    /// suspends and the two spawned tasks just run to completion in
    /// sequence, masking any contention. (Verified during development: with
    /// this setup, inverting one task's inlined order reliably made the test
    /// time out; without the yield_now, even the inverted order passed
    /// spuriously.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lock_order_matches_worker_and_never_deadlocks() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");

        let knowledge_store = Arc::new(RwLock::new(KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        )));
        let vault_optimizer = Arc::new(RwLock::new(VaultOptimizerService::new(
            data_dir.path().to_path_buf(),
        )));

        let result = timeout(Duration::from_secs(5), async {
            for _ in 0..100 {
                let ks_a = knowledge_store.clone();
                let vo_a = vault_optimizer.clone();
                let worker_side = tokio::spawn(async move {
                    // Mirrors main.rs::start_vault_optimizer_worker's order.
                    let _store = ks_a.write().await;
                    tokio::task::yield_now().await;
                    let _optimizer = vo_a.write().await;
                });

                let ks_b = knowledge_store.clone();
                let vo_b = vault_optimizer.clone();
                let rollback_side = tokio::spawn(async move {
                    // Mirrors rollback_vault_optimizer_change's (fixed) order.
                    let _store = ks_b.write().await;
                    tokio::task::yield_now().await;
                    let _optimizer = vo_b.write().await;
                });

                let (a, b) = tokio::join!(worker_side, rollback_side);
                a.expect("worker-side task panicked");
                b.expect("rollback-side task panicked");
            }
        })
        .await;

        assert!(
            result.is_ok(),
            "lock acquisitions did not complete within 5s — lock order regressed to ABBA"
        );
    }

    #[test]
    fn successful_repair_reports_pending_rollback_as_applied_but_not_an_aborted_target() {
        let mut pending = crate::models::migration::VaultOptimizerRollbackResult {
            change_id: "pending".into(),
            rolled_back: false,
            message: "Optimizer rollback authority advanced; recovery remains pending".into(),
            warning: Some(
                crate::models::mutation::CommittedMutationWarningV1::
                    optimizer_rollback_recovery_pending(),
            ),
        };
        super::reconcile_optimizer_rollback_after_repair(
            &mut pending,
            true,
            &crate::commands::PostAuthorityRepair::NotRequired,
        );
        assert!(pending.rolled_back);
        assert_eq!(pending.message, "Optimizer change rolled back");
        assert!(pending.warning.is_none());

        let warning =
            crate::models::mutation::CommittedMutationWarningV1::optimizer_rollback_not_applied();
        let mut aborted = crate::models::migration::VaultOptimizerRollbackResult {
            change_id: "aborted".into(),
            rolled_back: false,
            message: "Optimizer rollback was aborted after authority".into(),
            warning: Some(warning.clone()),
        };
        super::reconcile_optimizer_rollback_after_repair(
            &mut aborted,
            false,
            &crate::commands::PostAuthorityRepair::NotRequired,
        );
        assert!(!aborted.rolled_back);
        assert_eq!(aborted.warning, Some(warning));
    }
}
