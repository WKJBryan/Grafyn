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
    let epoch = crate::commands::capture_root_epoch(state.inner())?;
    let result = {
        let service = state.markdown_migration.read().await;
        let store = state.knowledge_store.read().await;
        let requested = std::fs::canonicalize(&vault_path).map_err(|error| error.to_string())?;
        let current =
            std::fs::canonicalize(store.vault_path()).map_err(|error| error.to_string())?;
        if requested != current {
            return Err("migration preview must use the active vault".into());
        }
        service
            .preview_scoped(&store, epoch.root_scope, request)
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
    let expected_epoch = crate::commands::capture_root_epoch(state.inner())?;
    let (apply_result, mutation_commit) = {
        let service = state.markdown_migration.read().await;
        let mut store = state.knowledge_store.write().await;
        store.clear_last_mutation_commit();
        let result = service
            .apply_scoped(
                &preview_id,
                request.clone(),
                &mut store,
                &expected_epoch.root_scope,
            )
            .map_err(|error| error.to_string());
        let commit = store.take_last_mutation_commit();
        (result, commit)
    };
    drop(root_epoch);

    let mut result = match apply_result {
        Ok(result) => result,
        Err(error) => {
            if let Some(commit) = mutation_commit.as_ref() {
                crate::commands::repair_after_authority_mutation(
                    state.inner(),
                    commit,
                    "Markdown migration",
                )
                .await;
            }
            return Err(error);
        }
    };

    let repair = if let Some(commit) = mutation_commit.as_ref() {
        crate::commands::repair_after_authority_mutation(
            state.inner(),
            commit,
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
    if let crate::commands::PostAuthorityRepair::Unavailable(warning) = &repair {
        result.message = format!("{}; {warning}", result.message);
    }

    if request.start_optimizer.unwrap_or(true) || request.enable_llm.unwrap_or(false) {
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
                background_vault_optimizer_enabled: Some(request.start_optimizer.unwrap_or(true)),
                background_vault_optimizer_llm_enabled: Some(request.enable_llm.unwrap_or(false)),
                background_vault_optimizer_budget_monthly: None,
                background_vault_optimizer_max_daily_writes: None,
                background_vault_optimizer_edit_mode: Some(
                    format!("{:?}", request.mode)
                        .to_lowercase()
                        .replace("sidecarfirst", "sidecar_first")
                        .replace("fullrewrite", "full_rewrite"),
                ),
                background_vault_optimizer_program_enabled: Some(true),
                vault_optimizer_program_path: request.program_path.clone(),
                canvas_model_presets: None,
            },
        )
        .await
        {
            result.message = format!(
                "{}; migration was committed but optimizer settings were not updated: {error}",
                result.message
            );
        }
    }

    if repair_ready {
        let mut optimizer = state.vault_optimizer.write().await;
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
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let epoch = crate::commands::capture_root_epoch(state.inner())?;
    let result = {
        let service = state.markdown_migration.read().await;
        service
            .status_scoped(run_id.as_deref(), &epoch.root_scope)
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn rollback_markdown_migration(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let epoch = crate::commands::capture_root_epoch(state.inner())?;
    // Rollback can fail after having already restored some files to disk. If we
    // `?`-return before rebuilding, the search/graph/chunk indexes stay pointed at
    // the pre-rollback state and disagree with the (partially) restored files. So:
    // capture the rollback result, ALWAYS rebuild the indexes to match whatever is
    // now on disk, then propagate the original rollback error (a rebuild error is
    // only surfaced when the rollback itself succeeded).
    let (rollback_result, mutation_commit) = {
        let service = state.markdown_migration.read().await;
        let mut store = state.knowledge_store.write().await;
        store.clear_last_mutation_commit();
        let result = service
            .rollback_scoped(&run_id, &mut store, &epoch.root_scope)
            .map_err(|error| error.to_string());
        (result, store.take_last_mutation_commit())
    };

    if let Some(commit) = mutation_commit.as_ref() {
        crate::commands::repair_after_authority_mutation(
            state.inner(),
            commit,
            "Markdown migration rollback",
        )
        .await;
    }
    rollback_result
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
        let mut optimizer = state.vault_optimizer.write().await;
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
        let mut optimizer = state.vault_optimizer.write().await;
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
        let mut optimizer = state.vault_optimizer.write().await;
        optimizer
            .with_locked_fresh_state(|optimizer| {
                optimizer.inbox(status.as_deref(), limit.unwrap_or(20))
            })
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn rollback_vault_optimizer_change(
    change_id: String,
    state: State<'_, AppState>,
) -> Result<VaultOptimizerRollbackResult, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let expected = root_ticket.authority().clone();
    let (result, mutation_commit) = {
        // Lock order: knowledge_store before vault_optimizer (see commands/mod.rs
        // doc comment) — must match the background worker in main.rs to avoid
        // an ABBA deadlock.
        let mut store = state.knowledge_store.write().await;
        store.clear_last_mutation_commit();
        let mut optimizer = state.vault_optimizer.write().await;
        let result = optimizer
            .with_locked_fresh_state(|optimizer| {
                optimizer.rollback_change_expecting_authority(
                    &change_id,
                    &mut store,
                    expected,
                )
            })
            .map_err(|error| error.to_string());
        (result, store.take_last_mutation_commit())
    };
    drop(root_ticket);
    let repair = if let Some(commit) = mutation_commit.as_ref() {
        crate::commands::repair_after_authority_mutation(
            state.inner(),
            commit,
            "vault optimizer rollback",
        )
        .await
    } else {
        crate::commands::PostAuthorityRepair::NotRequired
    };
    let mut result = match result {
        Ok(result) => result,
        Err(error) if mutation_commit.is_some() => VaultOptimizerRollbackResult {
            change_id,
            rolled_back: true,
            message: format!(
                "Optimizer authority rollback committed, but audit-state publication failed: {error}"
            ),
        },
        Err(error) => return Err(error),
    };
    if let crate::commands::PostAuthorityRepair::Unavailable(warning) = repair {
        result.message = format!("{}; {warning}", result.message);
    }
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
}
