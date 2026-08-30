//! Tauri commands for user settings management

use crate::models::canvas::AvailableModel;
use crate::models::note::Note;
use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use crate::services::ollama::OllamaStatus;
use crate::AppState;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// Get current settings
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<UserSettings, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let settings = state.settings_service.read().await;
    Ok(redact_sensitive_settings(settings.get()))
}

/// Get settings status (for checking if setup is needed)
#[tauri::command]
pub async fn get_settings_status(state: State<'_, AppState>) -> Result<SettingsStatus, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let settings = state.settings_service.read().await;
    Ok(settings.status())
}

/// Update settings
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    let updated = apply_settings_update(state.inner(), update).await?;
    Ok(redact_sensitive_settings(&updated))
}

pub(crate) async fn apply_settings_update(
    state: &AppState,
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    apply_settings_update_inner(state, update, None).await
}

async fn apply_settings_update_inner(
    state: &AppState,
    update: SettingsUpdate,
    fault: Option<crate::services::root_transition::RootTransitionFaultPoint>,
) -> Result<UserSettings, String> {
    let _transition_gate = state.vault_transition.write().await;
    crate::commands::ensure_root_healthy(state).await?;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
        .clone();
    let (transition_store, environment_runtime_secret) = {
        let settings = state.settings_service.read().await;
        (
            settings
                .root_transition_store()
                .map_err(|error| error.to_string())?,
            settings.environment_runtime_secret(),
        )
    };
    #[cfg(test)]
    if let Some(point) = fault {
        transition_store.fail_once_at(point);
    }
    #[cfg(not(test))]
    let _ = fault;

    let root_guard = coordinator
        .begin_root_transition()
        .map_err(|error| error.to_string())?;
    let durable_before = transition_store
        .read_authority_locked(root_guard.process_lock())
        .map_err(|error| error.to_string())?;
    let before = durable_before.settings.clone();
    let mut candidate = before.clone();
    crate::services::settings::apply_update_fields(&mut candidate, &update)
        .map_err(|error| error.to_string())?;
    let old_vault = std::path::PathBuf::from(&durable_before.authority.canonical_vault_path);
    let candidate_vault = std::fs::canonicalize(candidate.effective_vault_path())
        .map_err(|error| error.to_string())?;
    if root_guard
        .current_vault_path()
        .map_err(|error| error.to_string())?
        != old_vault
    {
        return Err("coordinator and durable settings roots disagree; restart required".into());
    }

    let mut knowledge = state.knowledge_store.write().await;
    let mut twin = state.twin_store.write().await;
    if std::fs::canonicalize(knowledge.vault_path()).map_err(|error| error.to_string())?
        != old_vault
    {
        return Err("Knowledge and durable settings roots disagree; restart required".into());
    }
    let old_twin = twin.root_path().to_path_buf();
    let old_notes = knowledge
        .list_full_notes()
        .map_err(|error| error.to_string())?;
    let data_path = twin
        .target_root_path()
        .parent()
        .ok_or_else(|| "Twin target root has no data parent".to_string())?
        .to_path_buf();
    crate::models::settings::twin_data_path_for_vault(&data_path, &candidate_vault)
        .map_err(|error| error.to_string())?;

    let (after_key_source, after_key_version) = match update.openrouter_api_key.as_deref() {
        None => (
            durable_before.authority.openrouter_key_source,
            durable_before.authority.openrouter_key_version.clone(),
        ),
        Some("") => (
            crate::services::root_transition::OpenRouterKeySource::Cleared,
            None,
        ),
        Some(_) => (
            crate::services::root_transition::OpenRouterKeySource::Versioned,
            Some(uuid::Uuid::new_v4().to_string()),
        ),
    };
    let after_scope = crate::services::twin_events::root_identity_for_path(&candidate_vault)
        .map_err(|error| error.to_string())?;
    let after_lease = if after_scope == durable_before.authority.lease.root_scope {
        durable_before.authority.lease.clone()
    } else {
        crate::services::twin_events::ActiveMarkdownRootLeaseV1::new(after_scope)
    };
    let after_authority = crate::services::root_transition::RootAuthorityV1::new_with_key_source(
        &candidate_vault,
        candidate.clone(),
        after_lease,
        after_key_source,
        after_key_version.clone(),
    )
    .map_err(|error| error.to_string())?;
    let mut transition = crate::services::root_transition::RootTransitionV1::prepared(
        durable_before.authority.clone(),
        after_authority,
        transition_store.authority_binding(),
    )
    .map_err(|error| error.to_string())?;

    let mut commit_uncertain = false;
    let mut rebuilt_authority = None;
    let result = async {
        transition_store
            .prepare_transition_cas_locked(
                root_guard.process_lock(),
                &durable_before,
                &transition,
            )
            .map_err(|error| error.to_string())?;
        transition_store
            .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterPrepared)
            .map_err(|error| error.to_string())?;

        if transition.after.openrouter_key_version != transition.before.openrouter_key_version {
            if let (Some(version), Some(secret)) = (
                transition.after.openrouter_key_version.as_deref(),
                update
                    .openrouter_api_key
                    .as_deref()
                    .filter(|secret| !secret.is_empty()),
            ) {
                transition_store
                    .stage_secret(version, secret)
                    .map_err(|error| error.to_string())?;
            }
        }
        transition_store
            .checkpoint(
                crate::services::root_transition::RootTransitionFaultPoint::AfterSecretStage,
            )
            .map_err(|error| error.to_string())?;

        if transition.root_changed {
            *state.loaded_authority.write().await = None;
            transition_store
                .write_lease(&transition.after.lease)
                .map_err(|error| error.to_string())?;
            root_guard
                .adopt_durable_root(&candidate_vault, &transition.after.lease)
                .map_err(|error| error.to_string())?;
            transition_store
                .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterLease)
                .map_err(|error| error.to_string())?;
            let candidate_namespace = root_guard
                .initialize_namespace(&transition.after.lease)
                .map_err(|error| error.to_string())?;
            root_guard
                .invalidate_namespace(&transition.after.lease)
                .map_err(|error| error.to_string())?;
            knowledge
                .adopt_coordinated_vault_path(candidate_vault.clone(), &candidate_namespace)
                .map_err(|error| error.to_string())?;
            let candidate_twin = root_guard
                .prepare_twin_data_path(&candidate_vault, &transition.after.lease)
                .map_err(|error| error.to_string())?;
            twin.replace_root_path(candidate_twin)
                .map_err(|error| error.to_string())?;
            rebuilt_authority = Some(
                root_guard
                    .capture_authority_token(&transition.after.lease)
                    .map_err(|error| error.to_string())?,
            );
            twin.rebuild_mutation_caches()
                .map_err(|error| error.to_string())?;
            let new_notes = knowledge
                .list_full_notes()
                .map_err(|error| error.to_string())?;
            rebuild_indexes_from_notes(state, &candidate_namespace, &new_notes).await?;
        }

        transition_store
            .write_settings(&transition.after.nonsecret_settings)
            .map_err(|error| error.to_string())?;
        transition_store
            .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterSettings)
            .map_err(|error| error.to_string())?;
        transition_store
            .write_key_authority(
                transition.after.openrouter_key_source,
                transition.after.openrouter_key_version.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        let durable_resolved_secret = transition_store
            .resolve_secret(transition.after.openrouter_key_version.as_deref())
            .map_err(|error| error.to_string())?;
        let resolved_secret = runtime_secret_for_authority(
            transition.after.openrouter_key_source,
            durable_resolved_secret,
            environment_runtime_secret.clone(),
        );
        transition_store
            .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterKeyRef)
            .map_err(|error| error.to_string())?;

        {
            let mut settings = state.settings_service.write().await;
            settings.publish_runtime_authority(
                candidate.clone(),
                transition.after.openrouter_key_source,
                transition.after.openrouter_key_version.clone(),
                resolved_secret.clone(),
            );
            if transition.after.openrouter_key_source
                == crate::services::root_transition::OpenRouterKeySource::Unset
            {
                if let Some(secret) = resolved_secret.clone() {
                    settings.adopt_environment_runtime_secret(secret);
                }
            }
        }
        state
            .openrouter
            .write()
            .await
            .set_api_key(resolved_secret.clone().unwrap_or_default());
        state
            .ollama
            .write()
            .await
            .set_base_url(candidate.ollama_base_url.clone());

        match transition_store.mark_committed(&transition) {
            crate::services::root_transition::MarkCommittedResult::Committed(committed) => {
                transition = *committed;
            }
            crate::services::root_transition::MarkCommittedResult::DefinitelyPrepared(error) => {
                return Err(error.to_string());
            }
            crate::services::root_transition::MarkCommittedResult::Uncertain(error) => {
                commit_uncertain = true;
                return Err(format!("root transition commit durability is uncertain: {error}"));
            }
        }
        transition_store
            .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterCommitted)
            .map_err(|error| error.to_string())?;
        if transition.after.openrouter_key_version != transition.before.openrouter_key_version {
            if let Some(version) = transition.before.openrouter_key_version.as_deref() {
                transition_store
                    .delete_secret(version)
                    .map_err(|error| error.to_string())?;
            }
        }
        transition_store
            .checkpoint(
                crate::services::root_transition::RootTransitionFaultPoint::AfterOldKeyDelete,
            )
            .map_err(|error| error.to_string())?;
        transition_store
            .remove_transition()
            .map_err(|error| error.to_string())?;
        transition_store
            .checkpoint(crate::services::root_transition::RootTransitionFaultPoint::AfterWalDelete)
            .map_err(|error| error.to_string())?;
        if transition.root_changed {
            let token = rebuilt_authority.as_ref().ok_or_else(|| {
                "root transition rebuilt without capturing its authority token".to_string()
            })?;
            root_guard
                .publish_namespace_ready(token)
                .map_err(|error| error.to_string())?;
            *state.loaded_authority.write().await = Some(token.clone());
        }
        Ok::<_, String>(candidate.clone())
    }
    .await;

    if let Err(error) = result {
        if commit_uncertain {
            let recovery = transition_store.recover_while_process_locked();
            let runtime_recovery = match recovery {
                Ok(crate::services::root_transition::RecoveryWork::RolledBack) => async {
                    *state.loaded_authority.write().await = None;
                    root_guard
                        .adopt_durable_root(&old_vault, &transition.rollback_lease)
                        .map_err(|error| error.to_string())?;
                    let old_namespace = root_guard
                        .initialize_namespace(&transition.rollback_lease)
                        .map_err(|error| error.to_string())?;
                    root_guard
                        .invalidate_namespace(&transition.rollback_lease)
                        .map_err(|error| error.to_string())?;
                    knowledge
                        .adopt_coordinated_vault_path(old_vault.clone(), &old_namespace)
                        .map_err(|error| error.to_string())?;
                    twin.replace_root_path(old_twin.clone())
                        .map_err(|error| error.to_string())?;
                    let rebuilt_token = root_guard
                        .capture_authority_token(&transition.rollback_lease)
                        .map_err(|error| error.to_string())?;
                    twin.rebuild_mutation_caches()
                        .map_err(|error| error.to_string())?;
                    rebuild_indexes_from_notes(state, &old_namespace, &old_notes).await?;
                    root_guard
                        .publish_namespace_ready(&rebuilt_token)
                        .map_err(|error| error.to_string())?;
                    *state.loaded_authority.write().await = Some(rebuilt_token);
                    let durable_old_secret = transition_store
                        .resolve_secret(transition.before.openrouter_key_version.as_deref())
                        .map_err(|error| error.to_string())?;
                    let old_secret = runtime_secret_for_authority(
                        transition.before.openrouter_key_source,
                        durable_old_secret,
                        environment_runtime_secret.clone(),
                    );
                    state.settings_service.write().await.publish_runtime_authority(
                        before.clone(),
                        transition.before.openrouter_key_source,
                        transition.before.openrouter_key_version.clone(),
                        old_secret.clone(),
                    );
                    if transition.before.openrouter_key_source
                        == crate::services::root_transition::OpenRouterKeySource::Unset
                    {
                        if let Some(secret) = old_secret.clone() {
                            state
                                .settings_service
                                .write()
                                .await
                                .adopt_environment_runtime_secret(secret);
                        }
                    }
                    state
                        .openrouter
                        .write()
                        .await
                        .set_api_key(old_secret.unwrap_or_default());
                    state
                        .ollama
                        .write()
                        .await
                        .set_base_url(before.ollama_base_url.clone());
                    Ok::<(), String>(())
                }
                .await,
                Ok(crate::services::root_transition::RecoveryWork::RolledForward) => async {
                    *state.loaded_authority.write().await = None;
                    root_guard
                        .adopt_durable_root(&candidate_vault, &transition.after.lease)
                        .map_err(|error| error.to_string())?;
                    let candidate_namespace = root_guard
                        .initialize_namespace(&transition.after.lease)
                        .map_err(|error| error.to_string())?;
                    root_guard
                        .invalidate_namespace(&transition.after.lease)
                        .map_err(|error| error.to_string())?;
                    knowledge
                        .adopt_coordinated_vault_path(candidate_vault.clone(), &candidate_namespace)
                        .map_err(|error| error.to_string())?;
                    let candidate_twin = root_guard
                        .prepare_twin_data_path(&candidate_vault, &transition.after.lease)
                        .map_err(|error| error.to_string())?;
                    twin.replace_root_path(candidate_twin)
                        .map_err(|error| error.to_string())?;
                    let rebuilt_token = root_guard
                        .capture_authority_token(&transition.after.lease)
                        .map_err(|error| error.to_string())?;
                    twin.rebuild_mutation_caches()
                        .map_err(|error| error.to_string())?;
                    let new_notes = knowledge
                        .list_full_notes()
                        .map_err(|error| error.to_string())?;
                    rebuild_indexes_from_notes(state, &candidate_namespace, &new_notes).await?;
                    root_guard
                        .publish_namespace_ready(&rebuilt_token)
                        .map_err(|error| error.to_string())?;
                    *state.loaded_authority.write().await = Some(rebuilt_token);
                    let durable_new_secret = transition_store
                        .resolve_secret(transition.after.openrouter_key_version.as_deref())
                        .map_err(|error| error.to_string())?;
                    let new_secret = runtime_secret_for_authority(
                        transition.after.openrouter_key_source,
                        durable_new_secret,
                        environment_runtime_secret.clone(),
                    );
                    state.settings_service.write().await.publish_runtime_authority(
                        candidate.clone(),
                        transition.after.openrouter_key_source,
                        transition.after.openrouter_key_version.clone(),
                        new_secret.clone(),
                    );
                    if transition.after.openrouter_key_source
                        == crate::services::root_transition::OpenRouterKeySource::Unset
                    {
                        if let Some(secret) = new_secret.clone() {
                            state
                                .settings_service
                                .write()
                                .await
                                .adopt_environment_runtime_secret(secret);
                        }
                    }
                    state
                        .openrouter
                        .write()
                        .await
                        .set_api_key(new_secret.unwrap_or_default());
                    state
                        .ollama
                        .write()
                        .await
                        .set_base_url(candidate.ollama_base_url.clone());
                    Ok::<(), String>(())
                }
                .await,
                Ok(crate::services::root_transition::RecoveryWork::None) => Err(
                    "uncertain root transition disappeared before recovery".to_string(),
                ),
                Err(recovery_error) => Err(recovery_error.to_string()),
            };
            let failure = match runtime_recovery {
                Ok(()) => format!("{error}; durable recovery completed; restart required"),
                Err(recovery_error) => {
                    format!("{error}; durable recovery failed: {recovery_error}; restart required")
                }
            };
            *state.mutation_startup_error.write().await = Some(failure.clone());
            return Err(failure);
        }
        if transition.decision == crate::services::root_transition::RootTransitionDecision::Prepared
        {
            let durable_rollback = transition_store.restore_prepared_authorities(&transition);
            let runtime_rollback = async {
                *state.loaded_authority.write().await = None;
                root_guard
                    .adopt_durable_root(&old_vault, &transition.rollback_lease)
                    .map_err(|error| error.to_string())?;
                let old_namespace = root_guard
                    .initialize_namespace(&transition.rollback_lease)
                    .map_err(|error| error.to_string())?;
                root_guard
                    .invalidate_namespace(&transition.rollback_lease)
                    .map_err(|error| error.to_string())?;
                knowledge
                    .adopt_coordinated_vault_path(old_vault.clone(), &old_namespace)
                    .map_err(|error| error.to_string())?;
                twin.replace_root_path(old_twin.clone())
                    .map_err(|error| error.to_string())?;
                let rebuilt_token = root_guard
                    .capture_authority_token(&transition.rollback_lease)
                    .map_err(|error| error.to_string())?;
                twin.rebuild_mutation_caches()
                    .map_err(|error| error.to_string())?;
                rebuild_indexes_from_notes(state, &old_namespace, &old_notes).await?;
                root_guard
                    .publish_namespace_ready(&rebuilt_token)
                    .map_err(|error| error.to_string())?;
                *state.loaded_authority.write().await = Some(rebuilt_token);
                let durable_old_secret = transition_store
                    .resolve_secret(transition.before.openrouter_key_version.as_deref())
                    .map_err(|error| error.to_string())?;
                let old_secret = runtime_secret_for_authority(
                    transition.before.openrouter_key_source,
                    durable_old_secret,
                    environment_runtime_secret.clone(),
                );
                state.settings_service.write().await.publish_runtime_authority(
                    before.clone(),
                    transition.before.openrouter_key_source,
                    transition.before.openrouter_key_version.clone(),
                    old_secret.clone(),
                );
                if transition.before.openrouter_key_source
                    == crate::services::root_transition::OpenRouterKeySource::Unset
                {
                    if let Some(secret) = old_secret.clone() {
                        state
                            .settings_service
                            .write()
                            .await
                            .adopt_environment_runtime_secret(secret);
                    }
                }
                state
                    .openrouter
                    .write()
                    .await
                    .set_api_key(old_secret.unwrap_or_default());
                state
                    .ollama
                    .write()
                    .await
                    .set_base_url(before.ollama_base_url.clone());
                Ok::<(), String>(())
            }
            .await;
            if let Err(rollback) = durable_rollback {
                let failure = format!("{error}; durable rollback failed: {rollback}");
                *state.mutation_startup_error.write().await = Some(failure.clone());
                return Err(failure);
            }
            if let Err(rollback) = runtime_rollback {
                let failure = format!("{error}; runtime rollback failed: {rollback}");
                *state.mutation_startup_error.write().await = Some(failure.clone());
                return Err(failure);
            }
            if let Err(rollback) = transition_store.finalize_prepared_rollback(&transition) {
                let failure = format!("{error}; rollback cleanup failed: {rollback}");
                *state.mutation_startup_error.write().await = Some(failure.clone());
                return Err(failure);
            }
        } else {
            *state.mutation_startup_error.write().await = Some(format!(
                "root transition committed but cleanup requires restart: {error}"
            ));
        }
        return Err(error);
    }
    result
}

async fn rebuild_indexes_from_notes(
    state: &AppState,
    derived_data_path: &std::path::Path,
    notes: &[Note],
) -> Result<(), String> {
    let search_matches = state
        .search_service
        .read()
        .await
        .uses_data_path(derived_data_path);
    let chunks_match = state
        .chunk_index
        .read()
        .await
        .uses_data_path(derived_data_path);
    let links_match = state
        .link_discovery
        .read()
        .await
        .uses_data_path(derived_data_path);
    let optimizer_matches = state
        .vault_optimizer
        .read()
        .await
        .uses_data_path(derived_data_path);
    let migration_matches = state
        .markdown_migration
        .read()
        .await
        .uses_data_path(derived_data_path);
    let reuse_current = search_matches
        && chunks_match
        && links_match
        && optimizer_matches
        && migration_matches;
    if reuse_current {
        state
            .search_service
            .write()
            .await
            .reindex_all(notes)
            .map_err(|error| error.to_string())?;
        state
            .chunk_index
            .write()
            .await
            .reindex_all(notes)
            .map_err(|error| error.to_string())?;
        state.graph_index.write().await.build_from_notes(notes);
        state
            .link_discovery
            .write()
            .await
            .bootstrap_checked(notes)
            .map_err(|error| error.to_string())?;
        state
            .vault_optimizer
            .write()
            .await
            .reset_for_vault_checked(notes)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    let mut search = crate::services::search::SearchService::new(derived_data_path.to_path_buf())
        .map_err(|error| error.to_string())?;
    search
        .reindex_all(notes)
        .map_err(|error| error.to_string())?;
    let mut chunks = crate::services::chunk_index::ChunkIndex::new(derived_data_path.to_path_buf())
        .map_err(|error| error.to_string())?;
    chunks
        .reindex_all(notes)
        .map_err(|error| error.to_string())?;
    let mut graph = crate::services::graph_index::GraphIndex::new();
    graph.build_from_notes(notes);
    let mut links = crate::services::link_discovery::LinkDiscoveryService::try_new(
        derived_data_path.to_path_buf(),
    )
    .map_err(|error| error.to_string())?;
    links
        .bootstrap_checked(notes)
        .map_err(|error| error.to_string())?;
    let mut optimizer = crate::services::vault_optimizer::VaultOptimizerService::try_new(
        derived_data_path.to_path_buf(),
    )
    .map_err(|error| error.to_string())?;
    optimizer
        .reset_for_vault_checked(notes)
        .map_err(|error| error.to_string())?;
    let migration = crate::services::markdown_migration::MarkdownMigrationService::try_new(
        derived_data_path.to_path_buf(),
    )
    .map_err(|error| error.to_string())?;

    *state.search_service.write().await = search;
    *state.chunk_index.write().await = chunks;
    *state.graph_index.write().await = graph;
    *state.link_discovery.write().await = links;
    *state.vault_optimizer.write().await = optimizer;
    *state.markdown_migration.write().await = migration;
    Ok(())
}

fn runtime_secret_for_authority(
    source: crate::services::root_transition::OpenRouterKeySource,
    durable_secret: Option<String>,
    environment_secret: Option<String>,
) -> Option<String> {
    match source {
        crate::services::root_transition::OpenRouterKeySource::Versioned => durable_secret,
        crate::services::root_transition::OpenRouterKeySource::Unset => environment_secret,
        crate::services::root_transition::OpenRouterKeySource::Cleared => None,
    }
}

/// Complete initial setup
#[tauri::command]
pub async fn complete_setup(state: State<'_, AppState>) -> Result<(), String> {
    complete_setup_inner(state.inner()).await
}

async fn complete_setup_inner(state: &AppState) -> Result<(), String> {
    let mut update = vault_update_for_setup();
    update.setup_completed = Some(true);
    apply_settings_update(state, update).await.map(|_| ())
}


fn vault_update_for_setup() -> SettingsUpdate {
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
        background_vault_optimizer_enabled: None,
        background_vault_optimizer_llm_enabled: None,
        background_vault_optimizer_budget_monthly: None,
        background_vault_optimizer_max_daily_writes: None,
        background_vault_optimizer_edit_mode: None,
        background_vault_optimizer_program_enabled: None,
        vault_optimizer_program_path: None,
        canvas_model_presets: None,
    }
}

/// Open folder picker dialog for vault selection
#[tauri::command]
pub async fn pick_vault_folder(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(desktop)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Select Vault Folder")
            .set_directory(dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
            .pick_folder(move |folder_path| {
                let _ = tx.send(folder_path.map(|path| path.to_string()));
            });

        return rx.await.map_err(|e| format!("Dialog error: {}", e));
    }

    #[cfg(mobile)]
    {
        let _ = app;
        Err("Vault folder selection is unavailable on mobile".to_string())
    }
}

/// Check if OpenRouter API key is valid by making a test request
#[tauri::command]
pub async fn validate_openrouter_key(api_key: String) -> Result<bool, String> {
    if api_key.is_empty() {
        return Ok(false);
    }

    let client = reqwest::Client::new();
    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    Ok(response.status().is_success())
}

/// Get OpenRouter API key status (configured or not, without exposing the key)
#[tauri::command]
pub async fn get_openrouter_status(state: State<'_, AppState>) -> Result<OpenRouterStatus, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let settings = state.settings_service.read().await;
    let has_key = settings.get().has_openrouter_key();

    // Check if the service is actually working
    let openrouter = &state.openrouter;
    let is_configured = openrouter.read().await.is_configured();

    Ok(OpenRouterStatus {
        has_key,
        is_configured,
    })
}

#[tauri::command]
pub async fn get_ollama_status(state: State<'_, AppState>) -> Result<OllamaStatus, String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let selected_model = {
        let settings = state.settings_service.read().await;
        settings.get().ollama_model.clone()
    };
    drop(root_epoch);
    let ollama = state.ollama.read().await;
    ollama
        .status(Some(&selected_model))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_ollama_models(state: State<'_, AppState>) -> Result<Vec<AvailableModel>, String> {
    let ollama = state.ollama.read().await;
    ollama
        .list_models()
        .await
        .map_err(|error| error.to_string())
}

#[derive(serde::Serialize)]
pub struct OpenRouterStatus {
    pub has_key: bool,
    pub is_configured: bool,
}

fn redact_sensitive_settings(settings: &UserSettings) -> UserSettings {
    let mut redacted = settings.clone();
    redacted.openrouter_api_key = None;
    redacted
}

#[cfg(test)]
fn vault_path_update_changed(settings: &UserSettings, candidate: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };

    comparable_path(settings.effective_vault_path())
        != comparable_path(std::path::PathBuf::from(candidate))
}

#[cfg(test)]
fn comparable_path(path: std::path::PathBuf) -> String {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    fn vault_update(path: &std::path::Path) -> SettingsUpdate {
        SettingsUpdate {
            vault_path: Some(path.to_string_lossy().into_owned()),
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
            background_vault_optimizer_enabled: None,
            background_vault_optimizer_llm_enabled: None,
            background_vault_optimizer_budget_monthly: None,
            background_vault_optimizer_max_daily_writes: None,
            background_vault_optimizer_edit_mode: None,
            background_vault_optimizer_program_enabled: None,
            vault_optimizer_program_path: None,
            canvas_model_presets: None,
        }
    }

    fn vault_and_key_update(path: &std::path::Path) -> SettingsUpdate {
        let mut update = vault_update(path);
        update.openrouter_api_key = Some("new-versioned-secret".into());
        update
    }

    fn root_switch_state(
        settings_path_is_directory: bool,
        readonly_search: bool,
    ) -> (AppState, TempDir, std::path::PathBuf, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let old_vault = root.path().join("vault-a");
        let new_vault = root.path().join("vault-b");
        let data = root.path().join("data");
        std::fs::create_dir(&old_vault).unwrap();
        std::fs::create_dir(&new_vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &old_vault,
                events.clone(),
                Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let derived_data = coordinator.current_namespace_path().unwrap();
        let user_settings = UserSettings {
            vault_path: Some(old_vault.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let config_path = if settings_path_is_directory {
            let path = root.path().join("settings-as-directory");
            std::fs::create_dir(&path).unwrap();
            path
        } else {
            root.path().join("settings.json")
        };
        let settings =
            crate::services::settings::SettingsService::for_test(config_path, user_settings);
        if !settings_path_is_directory {
            settings
                .root_transition_store()
                .unwrap()
                .write_settings_guarded(
                    &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                        settings.get().clone(),
                    ),
                )
                .unwrap();
        }
        if readonly_search {
            let candidate_scope =
                crate::services::twin_events::root_identity_for_path(&new_vault).unwrap();
            let candidate_derived = crate::services::vault_namespace::scoped_data_path(
                &data,
                &candidate_scope,
            );
            std::fs::create_dir_all(&candidate_derived).unwrap();
            std::fs::write(candidate_derived.join("search_index"), b"not-a-directory").unwrap();
        }
        let search = crate::services::search::SearchService::new(derived_data.clone()).unwrap();
        let twin_root =
            crate::models::settings::twin_data_path_for_vault(&data, &old_vault).unwrap();
        let loaded_authority = coordinator.current_authority_token().unwrap();
        let state = AppState {
            knowledge_store: Arc::new(RwLock::new(
                crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                    old_vault.clone(),
                    derived_data.clone(),
                    coordinator.clone(),
                ),
            )),
            graph_index: Arc::new(RwLock::new(crate::services::graph_index::GraphIndex::new())),
            search_service: Arc::new(RwLock::new(search)),
            canvas_store: Arc::new(RwLock::new(
                crate::services::canvas_store::CanvasStore::with_event_recorder(
                    data.join("canvas"),
                    coordinator.clone(),
                ),
            )),
            openrouter: Arc::new(RwLock::new(
                crate::services::openrouter::OpenRouterService::new(String::new()),
            )),
            ollama: Arc::new(RwLock::new(crate::services::ollama::OllamaService::new(
                "http://localhost:11434".into(),
            ))),
            feedback_service: Arc::new(RwLock::new(
                crate::services::feedback::FeedbackService::new(data.join("feedback")),
            )),
            settings_service: Arc::new(RwLock::new(settings)),
            priority_service: Arc::new(RwLock::new(
                crate::services::priority::PriorityScoringService::new(data.clone()),
            )),
            retrieval_service: Arc::new(RwLock::new(
                crate::services::retrieval::RetrievalService::new(data.clone()),
            )),
            chunk_index: Arc::new(RwLock::new(
                crate::services::chunk_index::ChunkIndex::new(derived_data.clone()).unwrap(),
            )),
            link_discovery: Arc::new(RwLock::new(
                crate::services::link_discovery::LinkDiscoveryService::new(derived_data.clone()),
            )),
            markdown_migration: Arc::new(RwLock::new(
                crate::services::markdown_migration::MarkdownMigrationService::new(
                    derived_data.clone(),
                ),
            )),
            vault_optimizer: Arc::new(RwLock::new(
                crate::services::vault_optimizer::VaultOptimizerService::new(derived_data),
            )),
            twin_store: Arc::new(RwLock::new(
                crate::services::twin::TwinStore::with_event_recorder(
                    twin_root,
                    data.join("twin"),
                    coordinator.clone(),
                ),
            )),
            twin_event_store: events,
            mutation_coordinator: Some(coordinator),
            mutation_startup_error: Arc::new(RwLock::new(None)),
            loaded_authority: Arc::new(RwLock::new(Some(loaded_authority))),
            vault_transition: Arc::new(tokio::sync::RwLock::new(())),
            memory_service: Arc::new(crate::services::memory::MemoryService::new()),
            boot_state: Arc::new(RwLock::new(crate::models::boot::BootStatus::default())),
        };
        (state, root, old_vault, new_vault)
    }

    #[test]
    fn same_vault_path_update_is_not_changed() {
        let settings = UserSettings {
            vault_path: Some("C:\\Vault".to_string()),
            ..Default::default()
        };

        assert!(!vault_path_update_changed(&settings, Some("C:/Vault")));
    }

    #[test]
    fn different_vault_path_update_is_changed() {
        let settings = UserSettings {
            vault_path: Some("C:\\Vault".to_string()),
            ..Default::default()
        };

        assert!(vault_path_update_changed(&settings, Some("C:/OtherVault")));
    }

    #[tokio::test]
    async fn vault_switch_publishes_settings_after_knowledge_twin_and_indexes() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
        let updated = apply_settings_update(&state, vault_update(&new_vault))
            .await
            .unwrap();
        assert_eq!(
            comparable_path(updated.effective_vault_path()),
            comparable_path(new_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(new_vault.clone())
        );
        let twin = state.twin_store.read().await;
        assert_eq!(
            comparable_path(twin.root_path().to_path_buf()),
            comparable_path(
                crate::models::settings::twin_data_path_for_vault(
                    twin.target_root_path().parent().unwrap(),
                    &new_vault,
                )
                .unwrap()
            )
        );
        drop(twin);
        let namespace = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_namespace_path()
            .unwrap();
        assert!(state.search_service.read().await.uses_data_path(&namespace));
        assert!(state.chunk_index.read().await.uses_data_path(&namespace));
        assert!(state.link_discovery.read().await.uses_data_path(&namespace));
        assert!(state.vault_optimizer.read().await.uses_data_path(&namespace));
        assert!(state
            .markdown_migration
            .read()
            .await
            .uses_data_path(&namespace));
        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .require_namespace_ready()
            .unwrap();
        assert_ne!(comparable_path(old_vault), comparable_path(new_vault));
    }

    #[tokio::test]
    async fn settings_persist_failure_rolls_back_live_knowledge_twin_and_lease() {
        let (state, _root, old_vault, new_vault) = root_switch_state(true, false);
        let old_twin = state.twin_store.read().await.root_path().to_path_buf();
        assert!(apply_settings_update(&state, vault_update(&new_vault))
            .await
            .is_err());
        assert_eq!(
            comparable_path(state.settings_service.read().await.vault_path()),
            comparable_path(old_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(old_vault)
        );
        assert_eq!(state.twin_store.read().await.root_path(), old_twin);
        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .require_namespace_ready()
            .unwrap();
    }

    #[tokio::test]
    async fn index_rebuild_failure_rolls_back_roots_before_settings_publish() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, true);
        let old_twin = state.twin_store.read().await.root_path().to_path_buf();
        assert!(apply_settings_update(&state, vault_update(&new_vault))
            .await
            .is_err());
        assert_eq!(
            comparable_path(state.settings_service.read().await.vault_path()),
            comparable_path(old_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(old_vault)
        );
        assert_eq!(state.twin_store.read().await.root_path(), old_twin);
    }

    #[tokio::test]
    async fn concurrent_note_command_observes_exactly_old_or_new_vault_boundary() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let switch_state = state.clone();
        let switch_barrier = barrier.clone();
        let switch_path = new_vault.clone();
        let switch = tokio::spawn(async move {
            switch_barrier.wait().await;
            apply_settings_update(&switch_state, vault_update(&switch_path)).await
        });
        let note_state = state.clone();
        let note =
            tokio::spawn(async move {
                barrier.wait().await;
                note_state.knowledge_store.write().await.create_note(
                    crate::models::note::NoteCreate {
                        title: "Root boundary marker".into(),
                        content: "root-switch-boundary-marker".into(),
                        relative_path: None,
                        aliases: Vec::new(),
                        status: crate::models::note::NoteStatus::Draft,
                        tags: Vec::new(),
                        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: None,
                        optimizer_managed: false,
                        properties: Default::default(),
                    },
                )
            });
        switch.await.unwrap().unwrap();
        note.await.unwrap().unwrap();

        let contains_marker = |root: &std::path::Path| {
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .any(|entry| {
                    std::fs::read_to_string(entry.path())
                        .is_ok_and(|value| value.contains("root-switch-boundary-marker"))
                })
        };
        assert_ne!(contains_marker(&old_vault), contains_marker(&new_vault));
    }

    #[tokio::test]
    async fn long_canvas_twin_result_is_rejected_after_root_epoch_changes() {
        let (state, _root, _old_vault, new_vault) = root_switch_state(false, false);
        let captured = {
            let _root_guard = crate::commands::acquire_root_epoch(&state).await.unwrap();
            crate::commands::capture_root_epoch(&state).unwrap()
        };

        apply_settings_update(&state, vault_update(&new_vault))
            .await
            .unwrap();
        let error = crate::commands::acquire_expected_root_epoch(&state, &captured)
            .await
            .expect_err("an in-flight Canvas/Twin result must not cross a root switch");
        assert!(error.contains("root authority changed"));
    }

    #[tokio::test]
    async fn authoritative_read_ticket_rejects_a_peer_generation_change() {
        let (state, _root, _old_vault, _new_vault) = root_switch_state(false, false);
        let ticket = crate::commands::acquire_root_epoch(&state).await.unwrap();
        let _snapshot = state.knowledge_store.read().await.list_notes().unwrap();

        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .commit_local(
                crate::models::twin_event::CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "peer-write.md",
                    "peer generation",
                )],
                Vec::new(),
            )
            .unwrap();

        let error = ticket
            .finish(&state)
            .await
            .expect_err("a peer mutation must invalidate the captured authoritative snapshot");
        assert!(error.contains("authority"));
    }

    #[tokio::test]
    async fn every_precommit_fault_rolls_back_and_restart_has_zero_work() {
        use crate::services::root_transition::{RecoveryWork, RootTransitionFaultPoint};
        for point in [
            RootTransitionFaultPoint::AfterPrepared,
            RootTransitionFaultPoint::AfterSecretStage,
            RootTransitionFaultPoint::AfterLease,
            RootTransitionFaultPoint::AfterSettings,
            RootTransitionFaultPoint::AfterKeyRef,
        ] {
            let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
            let old_twin = state.twin_store.read().await.root_path().to_path_buf();
            assert!(apply_settings_update_inner(
                &state,
                vault_and_key_update(&new_vault),
                Some(point),
            )
            .await
            .is_err());
            assert_eq!(
                crate::services::twin_events::root_identity_for_path(
                    state.knowledge_store.read().await.vault_path()
                )
                .unwrap(),
                crate::services::twin_events::root_identity_for_path(&old_vault).unwrap(),
                "precommit fault {point:?} must restore Knowledge"
            );
            assert_eq!(state.twin_store.read().await.root_path(), old_twin);
            assert_eq!(
                state
                    .settings_service
                    .read()
                    .await
                    .get()
                    .effective_vault_path(),
                old_vault
            );
            let store = state
                .settings_service
                .read()
                .await
                .root_transition_store()
                .unwrap();
            assert_eq!(store.recover().unwrap(), RecoveryWork::None);
            assert_eq!(store.recover().unwrap(), RecoveryWork::None);
        }
    }

    #[tokio::test]
    async fn committed_phase_faults_preserve_new_authority_and_restart_rolls_forward() {
        use crate::services::root_transition::{RecoveryWork, RootTransitionFaultPoint};
        for point in [
            RootTransitionFaultPoint::AfterCommitted,
            RootTransitionFaultPoint::AfterOldKeyDelete,
            RootTransitionFaultPoint::AfterWalDelete,
        ] {
            let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
            assert!(apply_settings_update_inner(
                &state,
                vault_and_key_update(&new_vault),
                Some(point),
            )
            .await
            .is_err());
            assert_eq!(
                crate::services::twin_events::root_identity_for_path(
                    state.knowledge_store.read().await.vault_path()
                )
                .unwrap(),
                crate::services::twin_events::root_identity_for_path(&new_vault).unwrap(),
                "committed fault {point:?} must retain new Knowledge authority"
            );
            assert_eq!(
                crate::services::twin_events::root_identity_for_path(
                    &state
                        .settings_service
                        .read()
                        .await
                        .get()
                        .effective_vault_path()
                )
                .unwrap(),
                crate::services::twin_events::root_identity_for_path(&new_vault).unwrap()
            );
            let store = state
                .settings_service
                .read()
                .await
                .root_transition_store()
                .unwrap();
            let work = store.recover().unwrap();
            if point == RootTransitionFaultPoint::AfterWalDelete {
                assert_eq!(work, RecoveryWork::None);
            } else {
                assert_eq!(work, RecoveryWork::RolledForward);
            }
            assert_eq!(store.recover().unwrap(), RecoveryWork::None);
            assert_ne!(
                crate::services::twin_events::root_identity_for_path(&old_vault).unwrap(),
                crate::services::twin_events::root_identity_for_path(&new_vault).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn uncertain_commit_is_recovered_by_durable_wal_and_latches_restart_required() {
        use crate::services::root_transition::{RecoveryWork, RootTransitionFaultPoint};
        let (state, _root, _old_vault, new_vault) = root_switch_state(false, false);
        let result = apply_settings_update_inner(
            &state,
            vault_update(&new_vault),
            Some(RootTransitionFaultPoint::CommitWalReadbackUnavailable),
        )
        .await;
        assert!(result.is_err());
        assert!(state.mutation_startup_error.read().await.is_some());
        assert_eq!(
            crate::services::twin_events::root_identity_for_path(
                state.knowledge_store.read().await.vault_path()
            )
            .unwrap(),
            crate::services::twin_events::root_identity_for_path(&new_vault).unwrap()
        );
        let store = state
            .settings_service
            .read()
            .await
            .root_transition_store()
            .unwrap();
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }

    #[tokio::test]
    async fn complete_setup_waits_for_root_transition_gate_before_settings_lock() {
        let (state, _root, _old_vault, _new_vault) = root_switch_state(false, false);
        let transition = state.vault_transition.write().await;
        let task_state = state.clone();
        let task = tokio::spawn(async move { complete_setup_inner(&task_state).await });
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "setup must wait behind the root transition gate");
        drop(transition);
        task.await.unwrap().unwrap();
        assert!(state.settings_service.read().await.get().setup_completed);
    }

    #[tokio::test]
    async fn omitted_key_patch_preserves_environment_runtime_authority() {
        let (state, root, _old_vault, _new_vault) = root_switch_state(false, false);
        let runtime_secret = "environment-only-openrouter-secret".to_string();
        {
            let mut settings = state.settings_service.write().await;
            settings.adopt_environment_runtime_secret(runtime_secret.clone());
        }
        state
            .openrouter
            .write()
            .await
            .set_api_key(runtime_secret.clone());
        let mut update = vault_update(state.knowledge_store.read().await.vault_path());
        update.vault_path = None;
        update.theme = Some("dark".into());

        apply_settings_update(&state, update).await.unwrap();

        assert_eq!(
            state
                .settings_service
                .read()
                .await
                .openrouter_api_key(),
            Some(runtime_secret.as_str())
        );
        assert!(state
            .openrouter
            .read()
            .await
            .get_api_key_masked()
            .is_some());
        let persisted = std::fs::read_to_string(root.path().join("settings.json")).unwrap();
        assert!(!persisted.contains(&runtime_secret));
    }

    #[tokio::test]
    async fn omitted_key_patch_rollback_restores_environment_runtime_authority() {
        use crate::services::root_transition::RootTransitionFaultPoint;
        let (state, _root, _old_vault, new_vault) = root_switch_state(false, false);
        let runtime_secret = "environment-only-rollback-secret".to_string();
        {
            let mut settings = state.settings_service.write().await;
            settings.adopt_environment_runtime_secret(runtime_secret.clone());
        }
        state
            .openrouter
            .write()
            .await
            .set_api_key(runtime_secret.clone());

        assert!(apply_settings_update_inner(
            &state,
            vault_update(&new_vault),
            Some(RootTransitionFaultPoint::AfterSettings),
        )
        .await
        .is_err());
        assert_eq!(
            state
                .settings_service
                .read()
                .await
                .openrouter_api_key(),
            Some(runtime_secret.as_str())
        );
        assert!(state
            .openrouter
            .read()
            .await
            .get_api_key_masked()
            .is_some());
    }

    #[tokio::test]
    async fn stale_process_root_switch_uses_fresh_peer_key_and_settings_authority() {
        use crate::services::root_transition::{
            MarkCommittedResult, OpenRouterKeySource, RecoveryWork, RootAuthorityV1,
            RootTransitionV1,
        };
        let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
        let mut initial_key = vault_update(&old_vault);
        initial_key.vault_path = None;
        initial_key.openrouter_api_key = Some("process-one-key".into());
        apply_settings_update(&state, initial_key).await.unwrap();

        let transition_store = state
            .settings_service
            .read()
            .await
            .root_transition_store()
            .unwrap();
        let peer_events = Arc::new(crate::services::twin_events::TwinEventStore::new(
            old_vault.parent().unwrap().join("data"),
        ));
        peer_events.initialize().unwrap();
        let peer = crate::services::twin_events::MutationCoordinator::new(
            old_vault.parent().unwrap().join("data"),
            &old_vault,
            peer_events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let peer_guard = peer.begin_root_transition().unwrap();
        let peer_before = transition_store
            .read_authority_locked(peer_guard.process_lock())
            .unwrap();
        let old_key_version = peer_before
            .authority
            .openrouter_key_version
            .clone()
            .unwrap();
        let new_key_version = uuid::Uuid::new_v4().to_string();
        let mut peer_settings = peer_before.settings.clone();
        peer_settings.mcp_enabled = true;
        let peer_after = RootAuthorityV1::new_with_key_source(
            &old_vault,
            peer_settings,
            peer_before.authority.lease.clone(),
            OpenRouterKeySource::Versioned,
            Some(new_key_version.clone()),
        )
        .unwrap();
        let peer_transition = RootTransitionV1::prepared(
            peer_before.authority.clone(),
            peer_after,
            transition_store.authority_binding(),
        )
        .unwrap();
        transition_store
            .prepare_transition_cas_locked(
                peer_guard.process_lock(),
                &peer_before,
                &peer_transition,
            )
            .unwrap();
        transition_store
            .stage_secret(&new_key_version, "process-two-key")
            .unwrap();
        transition_store
            .write_settings(&peer_transition.after.nonsecret_settings)
            .unwrap();
        transition_store
            .write_key_authority(OpenRouterKeySource::Versioned, Some(&new_key_version))
            .unwrap();
        assert!(matches!(
            transition_store.mark_committed(&peer_transition),
            MarkCommittedResult::Committed(_)
        ));
        transition_store.delete_secret(&old_key_version).unwrap();
        transition_store.remove_transition().unwrap();
        drop(peer_guard);

        let updated = apply_settings_update(&state, vault_update(&new_vault))
            .await
            .unwrap();
        assert!(updated.mcp_enabled);
        assert_eq!(
            state.settings_service.read().await.openrouter_api_key(),
            Some("process-two-key")
        );
        assert_eq!(
            transition_store.active_key_version().unwrap().as_deref(),
            Some(new_key_version.as_str())
        );
        assert!(transition_store
            .resolve_secret(Some(&old_key_version))
            .unwrap()
            .is_none());
        assert_eq!(transition_store.recover().unwrap(), RecoveryWork::None);
        assert_eq!(transition_store.recover().unwrap(), RecoveryWork::None);
    }
}
