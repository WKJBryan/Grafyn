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
    let service = state.markdown_migration.read().await;
    service
        .preview(std::path::PathBuf::from(vault_path), request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn apply_markdown_migration(
    preview_id: String,
    request: MarkdownMigrationRequest,
    state: State<'_, AppState>,
) -> Result<MarkdownMigrationApplyResult, String> {
    let result = {
        let service = state.markdown_migration.read().await;
        let mut store = state.knowledge_store.write().await;
        service
            .apply(&preview_id, request.clone(), &mut store)
            .map_err(|error| error.to_string())?
    };

    if request.start_optimizer.unwrap_or(true) || request.enable_llm.unwrap_or(false) {
        let mut settings = state.settings_service.write().await;
        settings
            .update(SettingsUpdate {
                vault_path: None,
                openrouter_api_key: None,
                setup_completed: None,
                theme: None,
                mcp_enabled: None,
                llm_model: None,
                smart_web_search: None,
                background_link_discovery_enabled: None,
                background_link_discovery_llm_enabled: None,
                background_vault_optimizer_enabled: Some(request.start_optimizer.unwrap_or(true)),
                background_vault_optimizer_llm_enabled: Some(request.enable_llm.unwrap_or(false)),
                background_vault_optimizer_budget_monthly: None,
                background_vault_optimizer_max_daily_writes: None,
                background_vault_optimizer_edit_mode: Some(format!("{:?}", request.mode).to_lowercase().replace("sidecarfirst", "sidecar_first").replace("fullrewrite", "full_rewrite")),
                background_vault_optimizer_program_enabled: Some(true),
                vault_optimizer_program_path: request.program_path.clone(),
                canvas_model_presets: None,
            })
            .map_err(|error| error.to_string())?;
    }

    crate::commands::rebuild_all_indexes(state.inner()).await?;

    {
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
    let service = state.markdown_migration.read().await;
    service
        .status(run_id.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn rollback_markdown_migration(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let service = state.markdown_migration.read().await;
        let mut store = state.knowledge_store.write().await;
        service
            .rollback(&run_id, &mut store)
            .map_err(|error| error.to_string())?;
    }

    crate::commands::rebuild_all_indexes(state.inner()).await?;
    Ok(())
}

#[tauri::command]
pub async fn get_vault_optimizer_status(
    state: State<'_, AppState>,
) -> Result<VaultOptimizerStatus, String> {
    let settings = state.settings_service.read().await;
    let optimizer = state.vault_optimizer.read().await;
    Ok(optimizer.status(settings.get()))
}

#[tauri::command]
pub async fn update_vault_optimizer_settings(
    update: VaultOptimizerSettingsUpdate,
    state: State<'_, AppState>,
) -> Result<crate::models::settings::UserSettings, String> {
    let mut settings = state.settings_service.write().await;
    settings
        .update(SettingsUpdate {
            vault_path: None,
            openrouter_api_key: None,
            setup_completed: None,
            theme: None,
            mcp_enabled: None,
            llm_model: None,
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
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_vault_optimizer_decisions(
    limit: Option<usize>,
    _cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<VaultOptimizerDecision>, String> {
    let optimizer = state.vault_optimizer.read().await;
    optimizer
        .list_decisions(limit.unwrap_or(20))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_vault_optimizer_inbox(
    status: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<VaultOptimizerInboxEntry>, String> {
    let optimizer = state.vault_optimizer.read().await;
    optimizer
        .inbox(status.as_deref(), limit.unwrap_or(20))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn rollback_vault_optimizer_change(
    change_id: String,
    state: State<'_, AppState>,
) -> Result<VaultOptimizerRollbackResult, String> {
    let result = {
        let mut optimizer = state.vault_optimizer.write().await;
        let mut store = state.knowledge_store.write().await;
        optimizer
            .rollback_change(&change_id, &mut store)
            .map_err(|error| error.to_string())?
    };
    crate::commands::rebuild_all_indexes(state.inner()).await?;
    Ok(result)
}
