#![cfg(feature = "tauri-app")]

mod commands;
pub mod models;
pub mod services;

use models::boot::BootStatus;
#[cfg(test)]
use services::twin_events::NoopMutationLifecycle;
use services::{
    canvas_store::CanvasStore,
    chunk_index::ChunkIndex,
    feedback::FeedbackService,
    graph_index::GraphIndex,
    knowledge_store::KnowledgeStore,
    link_discovery::LinkDiscoveryService,
    markdown_migration::MarkdownMigrationService,
    memory::MemoryService,
    ollama::OllamaService,
    openrouter::OpenRouterService,
    priority::PriorityScoringService,
    retrieval::RetrievalService,
    search::SearchService,
    settings::SettingsService,
    sync::engine::SyncEngine,
    twin::TwinStore,
    twin_events::{EventRecorder, MutationCoordinator, TwinEventStore, UnavailableEventRecorder},
    vault_optimizer::{OptimizerTick, VaultOptimizerService},
};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

/// Application state holding all services
#[derive(Clone)]
pub struct AppState {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub canvas_store: Arc<RwLock<CanvasStore>>,
    pub openrouter: Arc<RwLock<OpenRouterService>>,
    pub ollama: Arc<RwLock<OllamaService>>,
    pub feedback_service: Arc<RwLock<FeedbackService>>,
    pub settings_service: Arc<RwLock<SettingsService>>,
    pub priority_service: Arc<RwLock<PriorityScoringService>>,
    pub retrieval_service: Arc<RwLock<RetrievalService>>,
    pub chunk_index: Arc<RwLock<ChunkIndex>>,
    pub link_discovery: Arc<RwLock<LinkDiscoveryService>>,
    pub markdown_migration: Arc<RwLock<MarkdownMigrationService>>,
    pub vault_optimizer: Arc<RwLock<VaultOptimizerService>>,
    pub twin_store: Arc<RwLock<TwinStore>>,
    pub twin_event_store: Arc<TwinEventStore>,
    pub mutation_coordinator: Option<Arc<MutationCoordinator>>,
    pub(crate) sync_engine: Option<Arc<SyncEngine>>,
    pub mutation_startup_error: Arc<RwLock<Option<String>>>,
    pub(crate) loaded_authority:
        Arc<RwLock<Option<crate::services::vault_namespace::VaultAuthorityTokenV1>>>,
    /// Serializes complete derived-state repairs inside this desktop process.
    /// Cross-process serialization is provided by the coordinator guard that
    /// every repair holds for its entire capture/reload/build/publish window.
    pub(crate) authority_repair: Arc<tokio::sync::Mutex<()>>,
    pub(crate) committed_warning_app: Option<tauri::AppHandle>,
    pub vault_transition: Arc<RwLock<()>>,
    /// MemoryService is stateless — no lock needed, just Arc for shared ownership
    pub memory_service: Arc<MemoryService>,
    pub boot_state: Arc<RwLock<BootStatus>>,
    // Declared last so recovery files are removed only after the service
    // handles that use them have been dropped.
    _recovery_runtime: Option<Arc<tempfile::TempDir>>,
}

struct ServiceRuntimeRoots {
    data_path: std::path::PathBuf,
    vault_path: std::path::PathBuf,
    derived_data_path: std::path::PathBuf,
    active_scope: crate::models::twin_event::ContentDigest,
    recovery_runtime: Option<Arc<tempfile::TempDir>>,
}

fn canonical_path_components(path: &std::path::Path) -> Result<Vec<String>, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("Failed to canonicalize {}: {error}", path.display()))?;
    Ok(canonical
        .components()
        .map(|component| {
            let encoded = component.as_os_str().to_string_lossy();
            #[cfg(windows)]
            {
                encoded.to_lowercase()
            }
            #[cfg(not(windows))]
            {
                encoded.into_owned()
            }
        })
        .collect())
}

fn canonical_paths_overlap(
    left: &std::path::Path,
    right: &std::path::Path,
) -> Result<bool, String> {
    let left = canonical_path_components(left)?;
    let right = canonical_path_components(right)?;
    Ok(path_components_are_prefix(&left, &right) || path_components_are_prefix(&right, &left))
}

fn canonical_path_is_within(
    path: &std::path::Path,
    root: &std::path::Path,
) -> Result<bool, String> {
    let path = canonical_path_components(path)?;
    let root = canonical_path_components(root)?;
    Ok(path_components_are_prefix(&root, &path))
}

fn path_components_are_prefix(prefix: &[String], path: &[String]) -> bool {
    prefix.len() <= path.len()
        && prefix
            .iter()
            .zip(path.iter())
            .all(|(left, right)| left == right)
}

fn isolated_recovery_runtime_roots(
    active_data_path: &std::path::Path,
    active_vault_path: &std::path::Path,
) -> Result<ServiceRuntimeRoots, String> {
    isolated_recovery_runtime_roots_in(&std::env::temp_dir(), active_data_path, active_vault_path)
}

fn isolated_recovery_runtime_roots_in(
    temporary_base: &std::path::Path,
    active_data_path: &std::path::Path,
    active_vault_path: &std::path::Path,
) -> Result<ServiceRuntimeRoots, String> {
    for (label, active_root) in [
        ("active data root", active_data_path),
        ("active vault root", active_vault_path),
    ] {
        match std::fs::symlink_metadata(active_root) {
            Ok(_) => {
                if canonical_path_is_within(temporary_base, active_root)? {
                    return Err(format!(
                        "Isolated recovery temporary base overlaps the {label}: {}",
                        active_root.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to inspect {label} {}: {error}",
                    active_root.display()
                ));
            }
        }
    }
    let recovery_runtime = Arc::new(
        tempfile::Builder::new()
            .prefix("grafyn-recovery-runtime-v1-")
            .tempdir_in(temporary_base)
            .map_err(|error| format!("Failed to create isolated recovery runtime: {error}"))?,
    );
    for (label, active_root) in [
        ("active data root", active_data_path),
        ("active vault root", active_vault_path),
    ] {
        match std::fs::symlink_metadata(active_root) {
            Ok(_) => {
                if canonical_paths_overlap(recovery_runtime.path(), active_root)? {
                    return Err(format!(
                        "Isolated recovery runtime overlaps the {label}: {}",
                        active_root.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to inspect {label} {}: {error}",
                    active_root.display()
                ));
            }
        }
    }
    let data_path = recovery_runtime.path().join("data");
    let vault_path = recovery_runtime.path().join("vault");
    std::fs::create_dir(&data_path)
        .map_err(|error| format!("Failed to create recovery data root: {error}"))?;
    std::fs::create_dir(&vault_path)
        .map_err(|error| format!("Failed to create recovery vault root: {error}"))?;
    let active_scope = crate::services::twin_events::root_identity_for_path(&vault_path)
        .map_err(|error| error.to_string())?;
    let derived_data_path =
        crate::services::vault_namespace::scoped_data_path(&data_path, &active_scope);
    std::fs::create_dir_all(&derived_data_path)
        .map_err(|error| format!("Failed to create recovery derived root: {error}"))?;
    Ok(ServiceRuntimeRoots {
        data_path,
        vault_path,
        derived_data_path,
        active_scope,
        recovery_runtime: Some(recovery_runtime),
    })
}

fn initialize_attached_namespace(
    coordinator: &MutationCoordinator,
) -> Result<(std::path::PathBuf, crate::models::twin_event::ContentDigest), String> {
    let guard = coordinator
        .begin_root_transition()
        .map_err(|error| error.to_string())?;
    let lease = guard.current_lease().map_err(|error| error.to_string())?;
    let path = guard
        .initialize_namespace(&lease)
        .map_err(|error| error.to_string())?;
    guard
        .invalidate_namespace(&lease)
        .map_err(|error| error.to_string())?;
    Ok((path, lease.root_scope))
}

struct MutationStartupRuntime {
    coordinator: Arc<MutationCoordinator>,
    sync_engine: Arc<SyncEngine>,
}

fn initialize_stable_mutation_runtime(
    data_path: impl AsRef<std::path::Path>,
    vault_path: impl AsRef<std::path::Path>,
    twin_event_store: Arc<TwinEventStore>,
    secret_store: Arc<dyn crate::services::sync::secrets::SecretStore>,
) -> Result<MutationStartupRuntime, String> {
    let data_path = data_path.as_ref();
    let vault_path = vault_path.as_ref();
    let identity = crate::services::sync::identity::load_or_create_vault_identity(vault_path)
        .map_err(|error| error.to_string())?;
    let root_key = crate::services::sync::vault_keys::load_vault_root_key(
        secret_store.as_ref(),
        &identity.descriptor.vault_id().to_string(),
    )
    .map_err(|error| error.to_string())?;
    let sync_engine = Arc::new(
        SyncEngine::open_core(
            data_path,
            vault_path,
            identity,
            root_key,
            secret_store.clone(),
            twin_event_store.clone(),
        )
        .map_err(|error| error.to_string())?,
    );
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(
            data_path,
            vault_path,
            twin_event_store,
            sync_engine.clone(),
        )
        .map_err(|error| error.to_string())?,
    );
    let device = coordinator
        .load_or_create_device_signing_identity(secret_store)
        .map_err(|error| error.to_string())?;
    sync_engine
        .attach_device_identity(device)
        .map_err(|error| error.to_string())?;
    coordinator
        .recover_pending()
        .map_err(|error| error.to_string())?;
    Ok(MutationStartupRuntime {
        coordinator,
        sync_engine,
    })
}

fn bootstrap_sync_engine_before_service(
    sync_engine: Option<&Arc<SyncEngine>>,
    coordinator: Option<&Arc<MutationCoordinator>>,
    knowledge_store: &KnowledgeStore,
) -> Result<(), String> {
    if let Some(sync_engine) = sync_engine {
        let coordinator = coordinator.ok_or_else(|| {
            "sync engine is unavailable without its mutation coordinator".to_string()
        })?;
        sync_engine
            .recover_pending_inbox(coordinator)
            .map_err(|error| error.to_string())?;
        sync_engine
            .bootstrap_existing_vault(coordinator, knowledge_store)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn build_app_state(
    mut settings_service: SettingsService,
    committed_warning_app: Option<tauri::AppHandle>,
) -> Result<AppState, String> {
    let vault_path = settings_service.vault_path();
    let data_path = settings_service.data_path();
    let root_transition_store = settings_service
        .root_transition_store()
        .map_err(|error| error.to_string())?;
    let (detached_stable_vault, detached_validation_error) = match root_transition_store
        .detached_stable_vault()
    {
        Ok(detached) => (detached, None),
        Err(error) => {
            let recoverable_validation = matches!(
                &error,
                crate::services::twin_events::MutationError::RecoveryConflict(reason)
                    if reason == "configured stable vault path is not a real directory"
                        || reason == "configured stable vault descriptor was replaced"
            );
            if !recoverable_validation {
                return Err(error.to_string());
            }
            let recovery_guidance = format!(
                "{error}. Restore the configured vault, or select the original vault's correct location in Settings to reattach it."
            );
            (None, Some(recovery_guidance))
        }
    };

    log::info!("Vault path: {:?}", vault_path);
    log::info!("Data path: {:?}", data_path);

    let mut validation_recovery_roots = if detached_validation_error.is_some() {
        Some(isolated_recovery_runtime_roots(&data_path, &vault_path)?)
    } else {
        None
    };
    let runtime_vault_path = if let Some(error) = detached_validation_error {
        Err(error)
    } else if let Some(detached) = detached_stable_vault.as_ref() {
        Err(format!(
            "Configured stable vault {} is unavailable; select its new location to reattach vault {}",
            detached.configured_path.display(),
            detached.root_scope.as_str()
        ))
    } else {
        root_transition_store
            .prepare_runtime_vault_path(&vault_path)
            .map_err(|error| error.to_string())
    };

    // Initialize canonical mutation capture before any command can mutate user bytes.
    let twin_event_store = Arc::new(if let Some(recovery) = validation_recovery_roots.as_ref() {
        TwinEventStore::new_scoped(&recovery.data_path, recovery.active_scope.clone())
    } else {
        match detached_stable_vault.as_ref() {
            Some(detached) => TwinEventStore::new_scoped(&data_path, detached.root_scope.clone()),
            None => TwinEventStore::new(data_path.clone()),
        }
    });
    let mutation_runtime = (|| -> Result<MutationStartupRuntime, String> {
        let runtime_vault_path = runtime_vault_path.as_ref().map_err(Clone::clone)?;
        initialize_stable_mutation_runtime(
            &data_path,
            runtime_vault_path,
            twin_event_store.clone(),
            settings_service.secret_store(),
        )
    })();
    let (event_recorder, mutation_coordinator, sync_engine, mutation_startup_error): (
        Arc<dyn EventRecorder>,
        Option<Arc<MutationCoordinator>>,
        Option<Arc<SyncEngine>>,
        Option<String>,
    ) = match mutation_runtime {
        Ok(runtime) => (
            runtime.coordinator.clone(),
            Some(runtime.coordinator),
            Some(runtime.sync_engine),
            None,
        ),
        Err(error) => (
            Arc::new(UnavailableEventRecorder::new(error.clone())),
            None,
            None,
            Some(error),
        ),
    };
    let service_roots = if let Some(coordinator) = mutation_coordinator.as_ref() {
        let (derived_data_path, active_scope) = initialize_attached_namespace(coordinator)?;
        ServiceRuntimeRoots {
            data_path: data_path.clone(),
            vault_path: runtime_vault_path
                .as_ref()
                .expect("coordinator success requires runtime preparation")
                .clone(),
            derived_data_path,
            active_scope,
            recovery_runtime: None,
        }
    } else if let Some(recovery) = validation_recovery_roots.take() {
        recovery
    } else {
        isolated_recovery_runtime_roots(&data_path, &vault_path)?
    };
    let ServiceRuntimeRoots {
        data_path: service_data_path,
        vault_path: service_vault_path,
        derived_data_path,
        active_scope,
        recovery_runtime,
    } = service_roots;

    // Initialize services
    let knowledge_store = KnowledgeStore::with_event_recorder(
        service_vault_path.clone(),
        derived_data_path.clone(),
        event_recorder.clone(),
    );
    bootstrap_sync_engine_before_service(
        sync_engine.as_ref(),
        mutation_coordinator.as_ref(),
        &knowledge_store,
    )?;
    let graph_index = GraphIndex::new();
    let search_service = match SearchService::new(derived_data_path.clone()) {
        Ok(s) => s,
        Err(e) if e.is_corrupt_or_incompatible() => {
            log::error!(
                "Search index is explicitly corrupt or incompatible: {}. Attempting rebuild.",
                e
            );
            // Try deleting corrupted index and retrying
            let index_path = derived_data_path.join("search_index");
            if index_path.exists() {
                if let Err(rm_err) = std::fs::remove_dir_all(&index_path) {
                    log::error!("Failed to remove corrupted index: {}", rm_err);
                }
            }
            SearchService::new(derived_data_path.clone()).unwrap_or_else(|e2| {
                log::error!("Search service initialization failed after rebuild: {}", e2);
                std::process::exit(1);
            })
        }
        Err(e) => return Err(e.to_string()),
    };
    // Initialize chunk index (parallel to search index)
    let chunk_index = match ChunkIndex::new(derived_data_path.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                "Failed to initialize chunk index: {}. Attempting rebuild.",
                e
            );
            let chunk_path = derived_data_path.join("chunk_index");
            if chunk_path.exists() {
                let _ = std::fs::remove_dir_all(&chunk_path);
            }
            ChunkIndex::new(derived_data_path.clone()).unwrap_or_else(|e2| {
                log::error!("Chunk index initialization failed: {}", e2);
                std::process::exit(1);
            })
        }
    };

    let canvas_store = CanvasStore::with_event_recorder(
        crate::services::canvas_store::scoped_canvas_path(&service_data_path, &active_scope),
        event_recorder.clone(),
    );
    let twin_data_path = if let Some(coordinator) = mutation_coordinator.as_ref() {
        let guard = coordinator
            .begin_root_transition()
            .map_err(|error| error.to_string())?;
        let lease = guard.current_lease().map_err(|error| error.to_string())?;
        guard
            .prepare_twin_data_path(&vault_path, &lease)
            .map_err(|error| error.to_string())?
    } else {
        crate::models::settings::twin_data_path_for_scope(&service_data_path, &active_scope)
    };
    let twin_store = TwinStore::with_event_recorder_scoped(
        twin_data_path,
        service_data_path.join("twin"),
        active_scope,
        event_recorder.clone(),
    );

    // Get OpenRouter API key from settings, fall back to environment
    let environment_api_key = settings_service
        .allows_environment_fallback()
        .then(|| std::env::var("OPENROUTER_API_KEY").ok())
        .flatten();
    if settings_service.openrouter_api_key().is_none() {
        if let Some(secret) = environment_api_key {
            settings_service.adopt_environment_runtime_secret(secret);
        }
    }
    let api_key = settings_service
        .openrouter_api_key()
        .map(str::to_string)
        .unwrap_or_default();
    let openrouter = OpenRouterService::new(api_key);
    let ollama = OllamaService::new(settings_service.get().ollama_base_url.clone());

    // Initialize priority scoring service
    let priority_service = PriorityScoringService::new(service_data_path.clone());

    // Initialize retrieval service
    let retrieval_service = RetrievalService::new(service_data_path.clone());
    let link_discovery = LinkDiscoveryService::try_new(derived_data_path.clone())
        .map_err(|error| error.to_string())?;
    let markdown_migration = MarkdownMigrationService::try_new(derived_data_path.clone())
        .map_err(|error| error.to_string())?;
    let vault_optimizer =
        VaultOptimizerService::try_new(derived_data_path).map_err(|error| error.to_string())?;

    // Initialize feedback service using runtime environment only.
    // Release builds must not embed repository credentials.
    let feedback_service = FeedbackService::new(service_data_path.join("feedback"));
    let boot_state = Arc::new(RwLock::new(match mutation_startup_error.as_ref() {
        Some(error) => BootStatus::failed("failed", "Startup failed", error.clone()),
        None => BootStatus::default(),
    }));

    // Create app state (MemoryService is stateless — no RwLock needed)
    Ok(AppState {
        knowledge_store: Arc::new(RwLock::new(knowledge_store)),
        graph_index: Arc::new(RwLock::new(graph_index)),
        search_service: Arc::new(RwLock::new(search_service)),
        canvas_store: Arc::new(RwLock::new(canvas_store)),
        openrouter: Arc::new(RwLock::new(openrouter)),
        ollama: Arc::new(RwLock::new(ollama)),
        feedback_service: Arc::new(RwLock::new(feedback_service)),
        settings_service: Arc::new(RwLock::new(settings_service)),
        priority_service: Arc::new(RwLock::new(priority_service)),
        retrieval_service: Arc::new(RwLock::new(retrieval_service)),
        chunk_index: Arc::new(RwLock::new(chunk_index)),
        link_discovery: Arc::new(RwLock::new(link_discovery)),
        markdown_migration: Arc::new(RwLock::new(markdown_migration)),
        vault_optimizer: Arc::new(RwLock::new(vault_optimizer)),
        twin_store: Arc::new(RwLock::new(twin_store)),
        twin_event_store,
        mutation_coordinator,
        sync_engine,
        mutation_startup_error: Arc::new(RwLock::new(mutation_startup_error)),
        loaded_authority: Arc::new(RwLock::new(None)),
        authority_repair: Arc::new(tokio::sync::Mutex::new(())),
        committed_warning_app,
        vault_transition: Arc::new(RwLock::new(())),
        memory_service: Arc::new(MemoryService::new()),
        boot_state,
        _recovery_runtime: recovery_runtime,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init());

    #[cfg(all(desktop, feature = "desktop-updater"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(all(desktop, feature = "desktop-process"))]
    let builder = builder.plugin(tauri_plugin_process::init());

    builder
        .setup(|app| {
            // Root/settings recovery is part of SettingsService::load and must fail closed
            // before any store constructs itself from a possibly split authority.
            let state = build_app_state(SettingsService::load()?, Some(app.handle().clone()))?;

            app.manage(state);

            let app_handle = app.handle().clone();
            let state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                match warm_start_services(app_handle.clone(), state.clone()).await {
                    Ok(()) => {
                        start_link_discovery_worker(state.clone());
                        start_vault_optimizer_worker(state.clone());
                    }
                    Err(error) => {
                        publish_boot_status(
                            &app_handle,
                            &state,
                            BootStatus::failed("failed", "Startup failed", error),
                        )
                        .await;
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::boot::get_boot_status,
            // Note commands
            commands::notes::list_notes,
            commands::notes::get_note,
            commands::notes::create_note,
            commands::notes::update_note,
            commands::notes::delete_note,
            // Search commands
            commands::search::search_notes,
            commands::search::find_similar,
            commands::search::reindex,
            // Graph commands
            commands::graph::get_backlinks,
            commands::graph::get_outgoing,
            commands::graph::get_neighbors,
            commands::graph::get_unlinked,
            commands::graph::get_full_graph,
            commands::graph::rebuild_graph,
            // Canvas commands
            commands::canvas::list_sessions,
            commands::canvas::get_session,
            commands::canvas::create_session,
            commands::canvas::update_session,
            commands::canvas::delete_session,
            commands::canvas::get_available_models,
            commands::canvas::send_prompt,
            commands::canvas::update_tile_position,
            commands::canvas::delete_tile,
            commands::canvas::delete_response,
            commands::canvas::update_viewport,
            commands::canvas::update_llm_node_position,
            commands::canvas::auto_arrange,
            commands::canvas::export_to_note,
            commands::canvas::start_debate,
            commands::canvas::continue_debate,
            commands::canvas::add_models_to_tile,
            commands::canvas::regenerate_response,
            // Twin collector commands
            commands::twin::list_user_records,
            commands::twin::get_user_record,
            commands::twin::create_user_record,
            commands::twin::update_user_record,
            commands::twin::get_session_trace,
            commands::twin::run_twin_inference,
            commands::twin::get_twin_review,
            commands::twin::resolve_user_record_evidence,
            commands::twin::set_user_record_promotion,
            commands::twin::record_canvas_feedback,
            commands::twin::export_twin_data,
            commands::twin::list_decision_episodes,
            commands::twin::update_decision_outcome,
            commands::twin::get_decision_mirror_config,
            commands::twin::update_decision_mirror_config,
            commands::twin::reset_decision_mirror_config,
            commands::twin::list_memory_digest,
            commands::twin::review_memory_digest_item,
            commands::twin::list_constitution_items,
            commands::twin::create_constitution_item,
            commands::twin::update_constitution_item,
            commands::twin::review_constitution_item,
            commands::twin::list_action_gaps,
            commands::twin::review_action_gap,
            commands::twin::get_constitution_setup,
            commands::twin::save_constitution_setup,
            commands::twin::run_constitution_inference,
            // Twin state commands
            commands::twin_state::list_twin_observations,
            commands::twin_state::list_twin_proposals,
            commands::twin_state::create_companion_capture,
            commands::twin_state::review_twin_proposal,
            commands::twin_state::get_twin_state_projection,
            commands::twin_state::rank_twin_attention,
            commands::twin_state::get_twin_event_timeline,
            // Governed one-shot image generation commands
            commands::image_generation::discover_image_models,
            commands::image_generation::get_image_model_capability,
            commands::image_generation::generate_image,
            commands::image_generation::discard_generated_image_receipt,
            commands::image_generation::export_generated_image,
            commands::image_generation::save_generated_image,
            commands::image_generation::load_generated_image,
            #[cfg(feature = "twin-eval-lab")]
            // Twin evaluation commands
            commands::twin_eval::get_twin_eval_model_matrix,
            #[cfg(feature = "twin-eval-lab")]
            commands::twin_eval::preview_twin_eval_input,
            #[cfg(feature = "twin-eval-lab")]
            commands::twin_eval::preview_twin_eval_context,
            #[cfg(feature = "twin-eval-lab")]
            commands::twin_eval::run_twin_eval_lab,
            #[cfg(feature = "twin-eval-lab")]
            commands::twin_eval::run_twin_eval_lab_stream,
            #[cfg(feature = "twin-eval-lab")]
            commands::twin_eval::export_twin_eval_results,
            // Feedback commands
            commands::feedback::submit_feedback,
            commands::feedback::get_system_info,
            commands::feedback::feedback_status,
            commands::feedback::get_pending_feedback,
            commands::feedback::retry_pending_feedback,
            commands::feedback::clear_pending_feedback,
            // Settings commands
            commands::settings::get_settings,
            commands::settings::get_settings_status,
            commands::settings::update_settings,
            commands::settings::complete_setup,
            commands::settings::pick_vault_folder,
            commands::settings::validate_openrouter_key,
            commands::settings::get_openrouter_status,
            commands::settings::get_ollama_status,
            commands::settings::list_ollama_models,
            commands::sync::get_sync_status,
            commands::sync::list_sync_conflicts,
            commands::sync::export_sync_outbox,
            commands::sync::import_sync_envelopes,
            commands::sync::rebuild_sync_state,
            // Migration + optimizer commands
            commands::migration::preview_markdown_migration,
            commands::migration::apply_markdown_migration,
            commands::migration::get_markdown_migration_status,
            commands::migration::rollback_markdown_migration,
            commands::migration::get_vault_optimizer_status,
            commands::migration::update_vault_optimizer_settings,
            commands::migration::list_vault_optimizer_decisions,
            commands::migration::get_vault_optimizer_inbox,
            commands::migration::rollback_vault_optimizer_change,
            // Distill commands
            commands::distill::distill_note,
            commands::distill::normalize_tags,
            // MCP commands
            #[cfg(desktop)]
            commands::mcp::get_mcp_status,
            #[cfg(desktop)]
            commands::mcp::get_mcp_config_snippet,
            // Memory commands
            commands::memory::recall_relevant,
            commands::memory::find_contradictions,
            commands::memory::extract_claims,
            // Priority commands
            commands::priority::get_priority_settings,
            commands::priority::update_priority_settings,
            commands::priority::reset_priority_settings,
            // Zettelkasten commands
            commands::zettelkasten::discover_links,
            commands::zettelkasten::apply_links,
            commands::zettelkasten::create_link,
            commands::zettelkasten::get_link_types,
            commands::zettelkasten::list_link_suggestion_queue,
            commands::zettelkasten::dismiss_link_suggestion,
            commands::zettelkasten::get_link_discovery_status,
            // Import commands
            commands::import::preview_import,
            commands::import::apply_import,
            commands::import::get_supported_formats,
            // Retrieval commands
            commands::retrieval::retrieve_relevant,
            commands::retrieval::get_retrieval_config,
            commands::retrieval::update_retrieval_config,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            log::error!("Error while running tauri application: {}", e);
            std::process::exit(1);
        });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WarmStartComponent {
    Migration,
    Overlay,
    Graph,
    Search,
    Chunk,
    LinkDiscovery,
    Optimizer,
    TwinCaches,
}

fn warm_start_component(
    component: WarmStartComponent,
    injected_failure: Option<WarmStartComponent>,
) -> Result<(), String> {
    if injected_failure == Some(component) {
        return Err(format!("injected warm-start {component:?} failure"));
    }
    Ok(())
}

async fn maybe_publish_boot_phase(
    app_handle: Option<&tauri::AppHandle>,
    state: &AppState,
    boot_started: &Instant,
    status: BootStatus,
) {
    if let Some(app_handle) = app_handle {
        publish_boot_phase(app_handle, state, boot_started, status).await;
    }
}

async fn warm_start_services(app_handle: tauri::AppHandle, state: AppState) -> Result<(), String> {
    warm_start_services_inner(Some(&app_handle), &state, None).await
}

async fn warm_start_services_inner(
    app_handle: Option<&tauri::AppHandle>,
    state: &AppState,
    injected_failure: Option<WarmStartComponent>,
) -> Result<(), String> {
    warm_start_services_inner_with_gap(app_handle, state, injected_failure, || Ok(())).await
}

async fn warm_start_services_inner_with_gap(
    app_handle: Option<&tauri::AppHandle>,
    state: &AppState,
    injected_failure: Option<WarmStartComponent>,
    after_namespace: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _root_epoch = acquire_warm_start_root_gate(state).await?;
    let _authority_repair = state.authority_repair.lock().await;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
    let recovered_sync_authority = state
        .sync_engine
        .as_ref()
        .map(|engine| engine.recover_pending_inbox(coordinator))
        .transpose()
        .map_err(|error| error.to_string())?
        .and_then(|report| report.authority_token);
    let (namespace_lease, namespace_path) = {
        let namespace_guard = coordinator
            .begin_root_transition()
            .map_err(|error| error.to_string())?;
        let namespace_lease = namespace_guard
            .current_lease()
            .map_err(|error| error.to_string())?;
        let namespace_path = namespace_guard
            .initialize_namespace(&namespace_lease)
            .map_err(|error| error.to_string())?;
        namespace_guard
            .invalidate_namespace(&namespace_lease)
            .map_err(|error| error.to_string())?;
        (namespace_lease, namespace_path)
    };
    after_namespace()?;
    drop(
        coordinator
            .begin_root_transition()
            .map_err(|error| error.to_string())?,
    );
    let boot_started = Instant::now();

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("opening_twin_events", "Opening governed Twin history"),
    )
    .await;
    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("opening_store", "Loading notes from your vault"),
    )
    .await;

    if recovered_sync_authority.is_none() {
        warm_start_component(WarmStartComponent::Migration, injected_failure)?;
        {
            let migration = state.markdown_migration.read().await;
            let mut store = state.knowledge_store.write().await;
            migration
                .backfill_legacy_grafyn_notes(&mut store)
                .map_err(|error| error.to_string())?;
        }

        warm_start_component(WarmStartComponent::Overlay, injected_failure)?;
        // Finish every authoritative normalization before selecting the rebuild
        // generation. `sync_topic_hubs` may itself commit canonical Markdown.
        crate::commands::sync_topic_hubs(state).await?;
    }
    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_graph", "Building graph from your notes"),
    )
    .await;

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_search_index", "Building search index"),
    )
    .await;

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_chunk_index", "Building chunk index"),
    )
    .await;

    // Acquire every local service guard in canonical order before retaining
    // the cross-process guard. The shared inner seam then performs the exact
    // capture/reload/build/ready publication without another local lock await.
    let mut repair_guards = crate::commands::acquire_authority_repair_guards(state).await?;
    let rebuild_guard = coordinator
        .begin_root_transition()
        .map_err(|error| error.to_string())?;
    let current_lease = rebuild_guard
        .current_lease()
        .map_err(|error| error.to_string())?;
    if current_lease != namespace_lease
        || crate::services::vault_namespace::scoped_data_path(
            coordinator.data_path(),
            &current_lease.root_scope,
        ) != namespace_path
    {
        return Err("vault authority changed before derived rebuild".into());
    }
    let expected = rebuild_guard
        .capture_authority_token(&current_lease)
        .map_err(|error| error.to_string())?;
    if recovered_sync_authority
        .as_ref()
        .is_some_and(|recovered| recovered != &expected)
    {
        return Err("vault authority changed after pending sync recovery".into());
    }
    let repair_mode = if recovered_sync_authority.is_some() {
        crate::commands::AuthorityRepairMode::RemoteSync
    } else {
        crate::commands::AuthorityRepairMode::Local
    };
    crate::commands::rebuild_authority_with_retained_guard(
        &mut repair_guards,
        &rebuild_guard,
        &expected,
        repair_mode,
        |step| {
            let component = match step {
                crate::commands::AuthorityRepairStep::Normalized
                | crate::commands::AuthorityRepairStep::Captured => return Ok(()),
                crate::commands::AuthorityRepairStep::TwinCaches => WarmStartComponent::TwinCaches,
                crate::commands::AuthorityRepairStep::Search => WarmStartComponent::Search,
                crate::commands::AuthorityRepairStep::Chunk => WarmStartComponent::Chunk,
                crate::commands::AuthorityRepairStep::Graph => WarmStartComponent::Graph,
                crate::commands::AuthorityRepairStep::LinkDiscovery => {
                    WarmStartComponent::LinkDiscovery
                }
                crate::commands::AuthorityRepairStep::Optimizer => WarmStartComponent::Optimizer,
            };
            warm_start_component(component, injected_failure)
        },
    )?;

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::ready("Grafyn is ready"),
    )
    .await;

    Ok(())
}

async fn acquire_warm_start_root_gate(
    state: &AppState,
) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, String> {
    let guard = state.vault_transition.clone().read_owned().await;
    crate::commands::ensure_root_healthy(state).await?;
    Ok(guard)
}

#[cfg(test)]
fn initialize_twin_event_store_for_boot(store: &TwinEventStore) -> Result<(), String> {
    store.initialize().map_err(|error| error.to_string())
}

async fn publish_boot_phase(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    boot_started: &Instant,
    status: BootStatus,
) {
    log::info!(
        "Boot phase '{}' at {:.2?}: {}",
        status.phase,
        boot_started.elapsed(),
        status.message
    );
    publish_boot_status(app_handle, state, status).await;
}

async fn update_boot_state(boot_state: &Arc<RwLock<BootStatus>>, status: &BootStatus) {
    let mut current = boot_state.write().await;
    *current = status.clone();
}

async fn publish_boot_status(app_handle: &tauri::AppHandle, state: &AppState, status: BootStatus) {
    update_boot_state(&state.boot_state, &status).await;

    let _ = app_handle.emit("boot-status", status);
}

fn start_link_discovery_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;

            let root_guard = match crate::commands::acquire_derived_root_epoch(&state).await {
                Ok(guard) => guard,
                Err(error) => {
                    log::warn!("Background link discovery paused: {error}");
                    continue;
                }
            };
            let root_epoch = root_guard.authority().clone();

            let settings = {
                let settings = state.settings_service.read().await;
                settings.get().clone()
            };

            let job = {
                let mut discovery = state.link_discovery.write().await;
                state
                    .mutation_coordinator
                    .as_ref()
                    .expect("coordinator was required by the root ticket")
                    .with_locked_derived_state(&root_epoch, true, || {
                        discovery.reload_from_disk_checked().map_err(|error| {
                            crate::services::twin_events::MutationError::Invalid(error.to_string())
                        })?;
                        discovery
                            .next_background_job_checked(&settings)
                            .map_err(|error| {
                                crate::services::twin_events::MutationError::Invalid(
                                    error.to_string(),
                                )
                            })
                    })
            };

            let Some(job) = (match job {
                Ok(job) => job,
                Err(error) => {
                    log::warn!("Background link discovery state unavailable: {error}");
                    continue;
                }
            }) else {
                continue;
            };
            drop(root_guard);

            let result = services::link_discovery::discover_for_note_at_epoch(
                &state,
                &job.note_id,
                job.mode,
                10,
                false,
                Some(&root_epoch),
            )
            .await;

            let completion_ticket =
                match crate::commands::acquire_expected_derived_root_epoch(&state, &root_epoch)
                    .await
                {
                    Ok(guard) => guard,
                    Err(error) => {
                        log::warn!("Background link discovery discarded stale result: {error}");
                        continue;
                    }
                };

            let requeue = match &result {
                Ok(_) => None,
                Err(error) => {
                    log::warn!(
                        "Background link discovery failed for '{}' ({}): {}",
                        job.note_id,
                        job.priority,
                        error
                    );
                    Some("stale")
                }
            };
            let completion = {
                let mut discovery = state.link_discovery.write().await;
                state
                    .mutation_coordinator
                    .as_ref()
                    .expect("coordinator was required by the root ticket")
                    .with_locked_derived_state(&root_epoch, true, || {
                        discovery.reload_from_disk_checked().map_err(|error| {
                            crate::services::twin_events::MutationError::Invalid(error.to_string())
                        })?;
                        discovery
                            .complete_background_job_checked(&job.note_id, requeue)
                            .map_err(|error| {
                                crate::services::twin_events::MutationError::Invalid(
                                    error.to_string(),
                                )
                            })
                    })
            };
            if let Err(error) = completion {
                log::warn!("Background link discovery completion was not persisted: {error}");
                continue;
            }
            if let Err(error) = completion_ticket.finish(&state).await {
                log::warn!("Background link discovery completion became stale: {error}");
            }
        }
    });
}

fn optimizer_worker_failure_commit(
    error: &anyhow::Error,
) -> Option<crate::services::twin_events::MutationCommit> {
    error
        .downcast_ref::<crate::services::twin_events::MutationError>()
        .and_then(|error| error.authority_advanced_commit())
}

fn start_vault_optimizer_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;

            let root_guard = match crate::commands::acquire_derived_root_epoch(&state).await {
                Ok(guard) => guard,
                Err(error) => {
                    log::warn!("Background vault optimizer paused: {error}");
                    continue;
                }
            };

            let settings = {
                let settings = state.settings_service.read().await;
                settings.get().clone()
            };

            // Lock order: knowledge_store before vault_optimizer (see
            // commands/mod.rs doc comment). `prepare_next` only needs *read*
            // access to the vault — it resolves the sidecar_first write path
            // (and every no-op/error/cap-deferred case) entirely under a read
            // lock, so LLM/network work (once added) and disk I/O here never
            // block other note/search/canvas commands that need
            // `knowledge_store.write()`.
            let tick = {
                let store = state.knowledge_store.read().await;
                let mut optimizer = state.vault_optimizer.write().await;
                optimizer.with_locked_fresh_state(|optimizer| {
                    optimizer.prepare_next_expecting_authority(
                        &store,
                        &settings,
                        root_guard.authority().clone(),
                    )
                })
            };

            // Whichever branch below reindexes a note, it does so only AFTER
            // the block that produced `applied_note_id` has ended and its
            // `knowledge_store`/`vault_optimizer` guards have dropped —
            // `commit_note_index_refresh` reacquires `knowledge_store` itself
            // (see its doc comment in `commands/mod.rs` for why this can't
            // just be a call to `commit_note_write`).
            let mut apply_failed = false;
            let mut failure_commit = None;
            let committed =
                match tick {
                    Ok(OptimizerTick::NoWrite) => None,
                    Ok(OptimizerTick::Committed {
                        result,
                        commit,
                        warning,
                    }) => Some((result, commit, warning)),
                    Ok(OptimizerTick::RetryFenced(pending)) => {
                        let result = {
                            let mut store = state.knowledge_store.write().await;
                            let mut optimizer = state.vault_optimizer.write().await;
                            optimizer.apply_retry_fenced(&mut store, *pending)
                        };
                        match result {
                        Ok(crate::services::vault_optimizer::OptimizerMutationResult::Committed {
                            result,
                            commit,
                            warning,
                        }) => Some((result, commit, warning)),
                        Ok(crate::services::vault_optimizer::OptimizerMutationResult::NoWrite) => {
                            None
                        }
                        Err(error) => {
                            log::warn!(
                                "Background vault optimizer failed to resume retry fence: {}",
                                error
                            );
                            failure_commit = optimizer_worker_failure_commit(&error);
                            apply_failed = true;
                            None
                        }
                    }
                    }
                    Ok(OptimizerTick::Pending(pending)) => {
                        // Only non-`sidecar_first` edit modes reach here, and only
                        // for the narrow `update_note` write itself — acquire the
                        // write lock just for this, in the same canonical order.
                        let result = {
                            let mut store = state.knowledge_store.write().await;
                            let mut optimizer = state.vault_optimizer.write().await;
                            optimizer.apply_pending(&mut store, *pending)
                        };
                        match result {
                        Ok(crate::services::vault_optimizer::OptimizerMutationResult::Committed {
                            result,
                            commit,
                            warning,
                        }) => Some((result, commit, warning)),
                        Ok(crate::services::vault_optimizer::OptimizerMutationResult::NoWrite) => {
                            None
                        }
                        Err(error) => {
                            log::warn!("Background vault optimizer failed to apply: {}", error);
                            failure_commit = optimizer_worker_failure_commit(&error);
                            apply_failed = true;
                            None
                        }
                    }
                    }
                    Err(error) => {
                        log::warn!("Background vault optimizer failed to prepare: {}", error);
                        failure_commit = optimizer_worker_failure_commit(&error);
                        apply_failed = true;
                        None
                    }
                };

            if apply_failed {
                // `apply_pending` may have durably published Prepared before
                // returning an uncertain coordinator error. Release the root
                // read ticket, then force a complete authority rebuild so the
                // witness is classified before this queue job can run again.
                drop(root_guard);
                let token = failure_commit
                    .and_then(|commit| commit.authority_token)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        state
                            .mutation_coordinator
                            .as_ref()
                            .ok_or_else(|| "mutation coordinator is unavailable".to_string())
                            .and_then(|coordinator| {
                                coordinator
                                    .current_authority_token()
                                    .map_err(|error| error.to_string())
                            })
                    });
                match token {
                    Ok(token) => match crate::commands::repair_after_authority_token(
                        &state,
                        &token,
                        "vault optimizer failed apply",
                    )
                    .await
                    {
                        crate::commands::PostAuthorityRepair::NotRequired
                        | crate::commands::PostAuthorityRepair::Ready(_) => {}
                        crate::commands::PostAuthorityRepair::Unavailable(_) => {}
                    },
                    Err(error) => log::error!(
                        "Vault optimizer failure could not capture recovery authority: {error}"
                    ),
                }
                continue;
            }

            if let Some((result, commit, service_warning)) = committed {
                let repair = crate::commands::repair_after_authority_mutation(
                    &state,
                    &commit,
                    "vault optimizer",
                )
                .await;
                if service_warning.is_some()
                    && !matches!(repair, crate::commands::PostAuthorityRepair::Unavailable(_))
                {
                    crate::commands::publish_committed_warning(&state);
                }
                log::debug!(
                    "Vault optimizer committed change {} for note {}",
                    result.change_id(),
                    result.note_id()
                );
                continue;
            }
            if let Err(error) = root_guard.finish(&state).await {
                log::warn!("Background vault optimizer result became stale: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_tree(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut snapshot = walkdir::WalkDir::new(root)
            .min_depth(1)
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let contents = entry.file_type().is_file().then(|| {
                    std::fs::read(entry.path()).unwrap_or_else(|error| {
                        format!("unreadable:{:?}", error.kind()).into_bytes()
                    })
                });
                (relative, contents)
            })
            .collect::<Vec<_>>();
        snapshot.sort_by(|left, right| left.0.cmp(&right.0));
        snapshot
    }

    fn stable_boot_settings(
        temp: &tempfile::TempDir,
        vault_path: &std::path::Path,
    ) -> (SettingsService, crate::models::twin_event::ContentDigest) {
        let data_path = temp.path().join("data");
        std::fs::create_dir(&data_path).unwrap();
        std::fs::create_dir(vault_path).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(vault_path).unwrap();
        let durable_settings = crate::models::settings::UserSettings {
            vault_path: Some(vault_path.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        let settings =
            SettingsService::for_test(temp.path().join("settings.json"), durable_settings.clone());
        let transition_store = settings.root_transition_store().unwrap();
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    durable_settings,
                ),
            )
            .unwrap();
        transition_store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope.clone(),
                ),
            )
            .unwrap();
        (settings, identity.root_scope)
    }

    #[test]
    fn provisioned_desktop_startup_promotes_local_markdown_to_the_sync_outbox() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        std::fs::create_dir(&vault_path).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
        let root_scope = identity.root_scope.clone();
        let durable_settings = crate::models::settings::UserSettings {
            vault_path: Some(vault_path.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        let settings =
            SettingsService::for_test(temp.path().join("settings.json"), durable_settings.clone());
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
            &grafyn_sync_protocol::VaultRootKey::from_bytes([0x51; 32]),
        )
        .unwrap();
        std::fs::write(vault_path.join("startup-sync.md"), b"startup lifecycle\r\n").unwrap();

        let runtime = initialize_stable_mutation_runtime(
            temp.path().join("data"),
            &vault_path,
            Arc::new(TwinEventStore::new(temp.path().join("data"))),
            settings.secret_store(),
        )
        .unwrap();
        let knowledge_store = KnowledgeStore::with_event_recorder(
            vault_path.clone(),
            temp.path().join("derived"),
            runtime.coordinator.clone(),
        );

        bootstrap_sync_engine_before_service(
            Some(&runtime.sync_engine),
            Some(&runtime.coordinator),
            &knowledge_store,
        )
        .unwrap();

        assert_eq!(runtime.sync_engine.status().unwrap().outbox_operations, 1);
        assert_eq!(
            std::fs::read(vault_path.join("startup-sync.md")).unwrap(),
            b"startup lifecycle\r\n"
        );
        assert_eq!(
            runtime.coordinator.current_root_epoch().unwrap().root_scope,
            root_scope
        );
    }

    #[tokio::test]
    async fn provisioned_companion_capture_seals_inherited_group_and_excludes_local_only_group() {
        use crate::commands::twin_state::{
            create_companion_capture_inner, CompanionCaptureContextInput, CompanionCaptureKind,
            CompanionSyncPolicy, CreateCompanionCaptureRequest,
        };
        use crate::models::twin_event::CausalStream;
        use chrono::TimeZone;

        for (policy, expected_outbox, expected_stream) in [
            (CompanionSyncPolicy::Inherit, 3, CausalStream::SyncEligible),
            (CompanionSyncPolicy::LocalOnly, 0, CausalStream::LocalOnly),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let vault_path = temp.path().join("vault");
            let (settings, _) = stable_boot_settings(&temp, &vault_path);
            crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(
                temp.path().join("data"),
            )
            .unwrap();
            let identity =
                crate::services::sync::identity::load_or_create_vault_identity(&vault_path)
                    .unwrap();
            crate::services::sync::vault_keys::provision_vault_root_key(
                settings.secret_store().as_ref(),
                &identity.descriptor.vault_id().to_string(),
                &grafyn_sync_protocol::VaultRootKey::from_bytes([0x61; 32]),
            )
            .unwrap();
            let state = build_app_state(settings, None).unwrap();
            let sync_engine = state.sync_engine.as_ref().unwrap_or_else(|| {
                panic!(
                    "provisioned test runtime failed: {:?}",
                    state.mutation_startup_error.try_read().unwrap().as_deref()
                )
            });
            assert_eq!(sync_engine.status().unwrap().outbox_operations, 0);

            create_companion_capture_inner(
                &state,
                CreateCompanionCaptureRequest {
                    content: "Private or inherited thought".into(),
                    capture_kind: CompanionCaptureKind::Text,
                    context: CompanionCaptureContextInput::default(),
                    attachment_digests: Vec::new(),
                    grafyn_sync: policy,
                },
                chrono::Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap(),
            )
            .await
            .unwrap();

            assert_eq!(
                sync_engine.status().unwrap().outbox_operations,
                expected_outbox
            );
            let events = state.twin_event_store.ordered_events().unwrap();
            assert_eq!(events.len(), 2);
            assert!(events
                .iter()
                .all(|event| event.causal_stream == expected_stream));
        }
    }

    #[tokio::test]
    async fn provisioned_generated_image_save_seals_one_inherited_attachment_group() {
        use crate::commands::image_generation::{
            load_generated_image_inner, save_generated_image_inner,
        };
        use crate::models::image_generation::{
            GeneratedImageSyncPolicy, ImageMetadataRetentionPolicy, LoadGeneratedImageRequest,
            SaveGeneratedImageRequest,
        };
        use crate::models::twin_event::{CausalStream, EvidenceType};
        use crate::services::openrouter::{GeneratedImageReceipt, OpenRouterService};
        use chrono::TimeZone;

        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let (settings, _) = stable_boot_settings(&temp, &vault_path);
        crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(
            temp.path().join("data"),
        )
        .unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
        crate::services::sync::vault_keys::provision_vault_root_key(
            settings.secret_store().as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &grafyn_sync_protocol::VaultRootKey::from_bytes([0x62; 32]),
        )
        .unwrap();
        let state = build_app_state(settings, None).unwrap();
        let sync_engine = state.sync_engine.as_ref().unwrap();
        assert_eq!(sync_engine.status().unwrap().outbox_operations, 0);
        let image = image::DynamicImage::new_rgba8(2, 2);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let service = OpenRouterService::new(String::new());
        let receipt_id = service
            .insert_generated_image_for_tests(GeneratedImageReceipt {
                bytes: bytes.clone(),
                media_type: "image/png".into(),
                width: 2,
                height: 2,
                prompt: "Inherited companion image".into(),
                model_id: "author/model".into(),
                resolution: "1024x1024".into(),
                aspect_ratio: "1:1".into(),
                vault_scope: state
                    .mutation_coordinator
                    .as_ref()
                    .unwrap()
                    .current_authority_token()
                    .unwrap()
                    .root_scope,
                gateway: "openrouter".into(),
                provider_tag: "test-provider".into(),
            })
            .unwrap();
        *state.openrouter.write().await = service;

        let saved = save_generated_image_inner(
            &state,
            SaveGeneratedImageRequest {
                receipt_id,
                annotation: Some("Inherited image evidence".into()),
                retention_policy: ImageMetadataRetentionPolicy::RetainOriginal,
                grafyn_sync: GeneratedImageSyncPolicy::Inherit,
            },
            chrono::Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(sync_engine.status().unwrap().outbox_operations, 5);
        assert_eq!(
            serde_json::to_value(saved.sync_disposition).unwrap(),
            serde_json::json!({
                "status": "queued",
                "manifestCount": 1,
                "chunkCount": 1,
                "operationCount": 2
            })
        );
        let events = state.twin_event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|event| event.causal_stream == CausalStream::SyncEligible));
        assert_eq!(
            events
                .iter()
                .flat_map(|event| &event.evidence)
                .filter(|evidence| evidence.evidence_type == EvidenceType::Attachment)
                .count(),
            1
        );
        let loaded = load_generated_image_inner(
            &state,
            LoadGeneratedImageRequest {
                attachment_digest: saved.attachment_digest,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                loaded.base64_data
            )
            .unwrap(),
            bytes
        );
    }

    fn assert_recoverable_detached_validation_state(
        state: &AppState,
        active_data_path: &std::path::Path,
        source_error: &str,
    ) {
        assert!(state.mutation_coordinator.is_none());
        let startup_error = state
            .mutation_startup_error
            .try_read()
            .unwrap()
            .clone()
            .expect("detached validation must keep authoritative commands unavailable");
        assert!(startup_error.contains(source_error));
        assert!(startup_error.contains("reattach"));
        let boot = state.boot_state.try_read().unwrap().clone();
        assert_eq!(boot.phase, "failed");
        assert!(!boot.ready);
        assert!(boot.error.as_deref().unwrap().contains(source_error));
        assert!(boot.error.as_deref().unwrap().contains("reattach"));
        let recovery_root = state
            ._recovery_runtime
            .as_ref()
            .expect("recoverable boot must retain isolated service roots")
            .path();
        assert!(state
            .twin_event_store
            .data_path()
            .starts_with(recovery_root));
        assert!(!state
            .twin_event_store
            .data_path()
            .starts_with(active_data_path));
        assert!(state
            .knowledge_store
            .try_read()
            .unwrap()
            .vault_path()
            .starts_with(recovery_root));
    }

    #[test]
    fn descriptor_mismatch_constructs_failed_app_state_on_isolated_roots() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("configured-vault");
        let (settings, expected_scope) = stable_boot_settings(&temp, &vault_path);
        let replacement_vault = temp.path().join("replacement-vault");
        std::fs::create_dir(&replacement_vault).unwrap();
        let replacement =
            crate::services::sync::identity::load_or_create_vault_identity(&replacement_vault)
                .unwrap();
        assert_ne!(replacement.root_scope, expected_scope);
        std::fs::copy(
            replacement_vault.join("_grafyn/vault.json"),
            vault_path.join("_grafyn/vault.json"),
        )
        .unwrap();
        let before = snapshot_tree(temp.path());

        let state = build_app_state(settings, None)
            .expect("descriptor replacement must leave the reattach UI available");

        assert_recoverable_detached_validation_state(
            &state,
            &temp.path().join("data"),
            "configured stable vault descriptor was replaced",
        );
        assert_eq!(snapshot_tree(temp.path()), before);
    }

    #[test]
    fn configured_vault_file_constructs_failed_app_state_on_isolated_roots() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("configured-vault");
        let (settings, _scope) = stable_boot_settings(&temp, &vault_path);
        std::fs::remove_dir_all(&vault_path).unwrap();
        std::fs::write(&vault_path, b"not a vault directory").unwrap();
        let before = snapshot_tree(temp.path());

        let state = build_app_state(settings, None)
            .expect("invalid configured path must leave the reattach UI available");

        assert_recoverable_detached_validation_state(
            &state,
            &temp.path().join("data"),
            "configured stable vault path is not a real directory",
        );
        assert_eq!(snapshot_tree(temp.path()), before);
    }

    #[test]
    fn invalid_root_transition_still_aborts_app_state_construction() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("configured-vault");
        let (settings, _scope) = stable_boot_settings(&temp, &vault_path);
        std::fs::write(
            temp.path().join("data/twin/events/root-transition-v1.json"),
            b"not a root transition",
        )
        .unwrap();
        let before = snapshot_tree(temp.path());

        let error = match build_app_state(settings, None) {
            Ok(_) => panic!("root-transition recovery errors must remain strict"),
            Err(error) => error,
        };

        assert!(error.contains("invalid root transition"));
        assert_eq!(snapshot_tree(temp.path()), before);
    }

    #[test]
    fn optimizer_worker_failure_preserves_exact_authority_for_repair() {
        let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
        let authority = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap();
        let mutation_id = crate::models::twin_event::ContentDigest::parse("b".repeat(64)).unwrap();
        let error = anyhow::Error::new(
            crate::services::twin_events::MutationError::AuthorityAdvanced {
                mutation_id: mutation_id.clone(),
                authority_token: authority.clone(),
                target_aborted: false,
                reason: "injected optimizer publication failure".into(),
            },
        );

        let commit = optimizer_worker_failure_commit(&error)
            .expect("the worker must retain the direct non-retryable commit");
        assert_eq!(commit.mutation_id, Some(mutation_id));
        assert_eq!(commit.authority_token, Some(authority));
    }

    fn build_warm_start_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let (mut state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
        let discarded = data.path().join("discarded-derived");
        state.search_service = Arc::new(RwLock::new(
            crate::services::search::SearchService::new(discarded.clone()).unwrap(),
        ));
        state.chunk_index = Arc::new(RwLock::new(
            crate::services::chunk_index::ChunkIndex::new(discarded.clone()).unwrap(),
        ));
        state.link_discovery = Arc::new(RwLock::new(
            crate::services::link_discovery::LinkDiscoveryService::new(discarded.clone()),
        ));
        state.markdown_migration = Arc::new(RwLock::new(
            crate::services::markdown_migration::MarkdownMigrationService::new(discarded.clone()),
        ));
        state.vault_optimizer = Arc::new(RwLock::new(
            crate::services::vault_optimizer::VaultOptimizerService::new(discarded),
        ));
        let coordinator = state
            .mutation_coordinator
            .as_ref()
            .expect("test state should include a mutation coordinator")
            .clone();
        let namespace = coordinator.current_namespace_path().unwrap();
        state.knowledge_store = Arc::new(RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                namespace.clone(),
                coordinator.clone(),
            ),
        ));
        state.search_service = Arc::new(RwLock::new(
            crate::services::search::SearchService::new(namespace.clone()).unwrap(),
        ));
        state.chunk_index = Arc::new(RwLock::new(
            crate::services::chunk_index::ChunkIndex::new(namespace.clone()).unwrap(),
        ));
        state.link_discovery = Arc::new(RwLock::new(
            crate::services::link_discovery::LinkDiscoveryService::try_new(namespace.clone())
                .unwrap(),
        ));
        state.markdown_migration = Arc::new(RwLock::new(
            crate::services::markdown_migration::MarkdownMigrationService::try_new(
                namespace.clone(),
            )
            .unwrap(),
        ));
        state.vault_optimizer = Arc::new(RwLock::new(
            crate::services::vault_optimizer::VaultOptimizerService::try_new(namespace).unwrap(),
        ));
        state.twin_store = Arc::new(RwLock::new(
            crate::services::twin::TwinStore::with_event_recorder(
                crate::models::settings::twin_data_path_for_vault(data.path(), vault.path())
                    .unwrap(),
                data.path().join("twin"),
                coordinator.clone(),
            ),
        ));
        state.mutation_coordinator = Some(coordinator);
        (state, vault, data)
    }

    fn build_stable_warm_start_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let vault = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(vault.path()).unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            data.path(),
            data.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(
            data.path(),
        )
        .unwrap();
        let durable_settings = crate::models::settings::UserSettings {
            vault_path: Some(vault.path().to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        transition_store
            .write_settings(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    durable_settings.clone(),
                ),
            )
            .unwrap();
        transition_store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope,
                ),
            )
            .unwrap();
        transition_store
            .write_key_authority(
                crate::services::root_transition::OpenRouterKeySource::Unset,
                None,
            )
            .unwrap();
        let event_store = Arc::new(TwinEventStore::new(data.path()));
        let coordinator = Arc::new(
            MutationCoordinator::new_stable(
                data.path(),
                vault.path(),
                event_store.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let scope = coordinator.current_root_epoch().unwrap().root_scope;
        let twin_path = {
            let guard = coordinator.begin_root_transition().unwrap();
            let lease = guard.current_lease().unwrap();
            guard.prepare_twin_data_path(vault.path(), &lease).unwrap()
        };
        let settings = crate::services::settings::SettingsService::for_test(
            data.path().join("settings.json"),
            durable_settings,
        );
        let state = AppState {
            knowledge_store: Arc::new(RwLock::new(KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                namespace.clone(),
                coordinator.clone(),
            ))),
            graph_index: Arc::new(RwLock::new(GraphIndex::new())),
            search_service: Arc::new(RwLock::new(SearchService::new(namespace.clone()).unwrap())),
            canvas_store: Arc::new(RwLock::new(CanvasStore::with_event_recorder(
                crate::services::canvas_store::scoped_canvas_path(data.path(), &scope),
                coordinator.clone(),
            ))),
            openrouter: Arc::new(RwLock::new(OpenRouterService::new(String::new()))),
            ollama: Arc::new(RwLock::new(OllamaService::new(String::new()))),
            feedback_service: Arc::new(RwLock::new(FeedbackService::new(
                data.path().join("feedback"),
            ))),
            settings_service: Arc::new(RwLock::new(settings)),
            priority_service: Arc::new(RwLock::new(PriorityScoringService::new(
                data.path().to_path_buf(),
            ))),
            retrieval_service: Arc::new(RwLock::new(RetrievalService::new(
                data.path().to_path_buf(),
            ))),
            chunk_index: Arc::new(RwLock::new(ChunkIndex::new(namespace.clone()).unwrap())),
            link_discovery: Arc::new(RwLock::new(
                LinkDiscoveryService::try_new(namespace.clone()).unwrap(),
            )),
            markdown_migration: Arc::new(RwLock::new(
                MarkdownMigrationService::try_new(namespace.clone()).unwrap(),
            )),
            vault_optimizer: Arc::new(RwLock::new(
                VaultOptimizerService::try_new(namespace).unwrap(),
            )),
            twin_store: Arc::new(RwLock::new(TwinStore::with_event_recorder_scoped(
                twin_path,
                data.path().join("twin"),
                scope,
                coordinator.clone(),
            ))),
            twin_event_store: event_store,
            mutation_coordinator: Some(coordinator),
            sync_engine: None,
            mutation_startup_error: Arc::new(RwLock::new(None)),
            loaded_authority: Arc::new(RwLock::new(None)),
            authority_repair: Arc::new(tokio::sync::Mutex::new(())),
            committed_warning_app: None,
            vault_transition: Arc::new(RwLock::new(())),
            memory_service: Arc::new(MemoryService::new()),
            boot_state: Arc::new(RwLock::new(BootStatus::default())),
            _recovery_runtime: None,
        };
        (state, vault, data)
    }

    #[tokio::test]
    async fn update_boot_state_replaces_existing_status() {
        let boot_state = Arc::new(RwLock::new(BootStatus::default()));
        let next = BootStatus::new("building_search_index", "Building search index");

        update_boot_state(&boot_state, &next).await;

        assert_eq!(*boot_state.read().await, next);
    }

    #[tokio::test]
    async fn warm_start_waits_behind_root_transition_before_reading_services() {
        let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
        let transition = state.vault_transition.write().await;
        let task_state = state.clone();
        let task = tokio::spawn(async move { acquire_warm_start_root_gate(&task_state).await });
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "warm start must wait behind root transition"
        );
        drop(transition);
        drop(task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn peer_root_wal_in_warm_start_gap_fails_before_any_further_data_change() {
        let (state, vault, data) = build_stable_warm_start_state();
        let data_snapshot = Arc::new(std::sync::Mutex::new(None));
        let vault_snapshot = Arc::new(std::sync::Mutex::new(None));
        let data_snapshot_hook = data_snapshot.clone();
        let vault_snapshot_hook = vault_snapshot.clone();
        let data_path = data.path().to_path_buf();
        let vault_path = vault.path().to_path_buf();

        let error = warm_start_services_inner_with_gap(None, &state, None, move || {
            std::fs::write(
                data_path.join("twin/events/root-transition-v1.json"),
                b"peer-prepared",
            )
            .map_err(|error| error.to_string())?;
            *data_snapshot_hook.lock().unwrap() = Some(snapshot_tree(&data_path));
            *vault_snapshot_hook.lock().unwrap() = Some(snapshot_tree(&vault_path));
            Ok(())
        })
        .await
        .unwrap_err();

        assert!(error.contains("root-transition"));
        assert_eq!(
            snapshot_tree(data.path()),
            data_snapshot.lock().unwrap().clone().unwrap()
        );
        assert_eq!(
            snapshot_tree(vault.path()),
            vault_snapshot.lock().unwrap().clone().unwrap()
        );
        assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    }

    #[test]
    fn attached_boot_namespace_initialization_does_not_self_deadlock() {
        let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(initialize_attached_namespace(&coordinator));
        });

        let result = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("attached namespace initialization must not self-deadlock");
        let (namespace, scope) = result.unwrap();
        assert!(namespace.is_dir());
        assert_eq!(
            namespace,
            std::fs::canonicalize(crate::services::vault_namespace::scoped_data_path(
                state.twin_event_store.data_path(),
                &scope,
            ))
            .unwrap()
        );
    }

    #[tokio::test]
    async fn every_readiness_component_failure_keeps_marker_unready_and_commands_unavailable() {
        for component in [
            WarmStartComponent::Migration,
            WarmStartComponent::Overlay,
            WarmStartComponent::Graph,
            WarmStartComponent::Search,
            WarmStartComponent::Chunk,
            WarmStartComponent::LinkDiscovery,
            WarmStartComponent::Optimizer,
            WarmStartComponent::TwinCaches,
        ] {
            let (state, _vault, _data) = build_warm_start_state();
            let coordinator = state.mutation_coordinator.as_ref().unwrap();
            let namespace_path = coordinator.current_namespace_path().unwrap();

            let error = warm_start_services_inner(None, &state, Some(component))
                .await
                .unwrap_err();

            assert!(
                error.contains(&format!("{component:?}")),
                "expected injected {component:?} failure, got: {error}"
            );
            let marker = std::fs::read_to_string(namespace_path.join("ready-v1.json"))
                .expect("failed rebuild should retain the durable unready marker");
            assert!(marker.contains("\"ready\": false"));
            assert!(coordinator.require_namespace_ready().is_err());
            assert!(crate::commands::acquire_derived_root_epoch(&state)
                .await
                .is_err());
        }
    }

    #[test]
    fn canonical_directory_path_is_file_surfaces_as_recoverable_boot_failure() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let events_path = store.events_dir();
        std::fs::create_dir_all(events_path.parent().unwrap()).unwrap();
        std::fs::write(&events_path, b"not a directory").unwrap();

        let error = initialize_twin_event_store_for_boot(&store).unwrap_err();
        let status = BootStatus::failed("failed", "Startup failed", error);
        assert_eq!(status.phase, "failed");
        assert!(!status.ready);
        assert!(status
            .error
            .unwrap()
            .contains("canonical Twin event directory"));
    }

    #[test]
    fn production_twin_store_never_falls_back_to_the_noop_constructor() {
        let desktop = include_str!("lib.rs");
        let settings = include_str!("commands/settings.rs");
        assert!(desktop.contains("TwinStore::with_event_recorder_scoped("));
        let desktop_noop = [
            "let twin_store = TwinStore::",
            "new(settings_service.get().effective_twin_data_path())",
        ]
        .concat();
        assert!(!desktop.contains(&desktop_noop));
        let retarget = settings.find("replace_twin_and_canvas_roots(").unwrap();
        let publish = settings
            .find("settings.publish_runtime_authority(")
            .unwrap();
        assert!(retarget < publish);
        assert!(settings.contains("twin.replace_root_path_scoped("));
        let settings_noop = ["TwinStore::", "new(new_twin_path)"].concat();
        assert!(!settings.contains(&settings_noop));
    }

    #[test]
    fn first_install_without_a_stable_lease_creates_the_configured_vault() {
        let temp = tempfile::tempdir().unwrap();
        let data_path = temp.path().join("data");
        let configured_vault = temp.path().join("first-install-vault");
        std::fs::create_dir(&data_path).unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    crate::models::settings::UserSettings {
                        vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                        ..crate::models::settings::UserSettings::default()
                    },
                ),
            )
            .unwrap();

        let runtime = transition_store
            .prepare_runtime_vault_path(&configured_vault)
            .unwrap();

        assert_eq!(runtime, configured_vault);
        assert!(runtime.is_dir());
    }

    #[test]
    fn stable_attached_vault_removed_after_detachment_probe_is_not_recreated() {
        let temp = tempfile::tempdir().unwrap();
        let data_path = temp.path().join("data");
        let configured_vault = temp.path().join("stable-vault");
        std::fs::create_dir(&data_path).unwrap();
        std::fs::create_dir(&configured_vault).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&configured_vault)
                .unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        let settings = crate::models::settings::UserSettings {
            vault_path: Some(configured_vault.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(settings),
            )
            .unwrap();
        transition_store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope,
                ),
            )
            .unwrap();

        assert!(transition_store.detached_stable_vault().unwrap().is_none());
        std::fs::remove_dir_all(&configured_vault).unwrap();

        let error = transition_store
            .prepare_runtime_vault_path(&configured_vault)
            .unwrap_err();
        assert!(matches!(
            error,
            crate::services::twin_events::MutationError::Io(_)
                | crate::services::twin_events::MutationError::Store(_)
        ));
        assert!(
            !configured_vault.exists(),
            "a stable vault removed after the probe must remain available for reattachment"
        );
    }

    #[test]
    fn production_boot_probes_detachment_before_any_runtime_root_preparation() {
        let desktop = include_str!("lib.rs");
        let probe = desktop.find(".detached_stable_vault()").unwrap();
        let prepare = desktop
            .find(".prepare_runtime_vault_path(&vault_path)")
            .unwrap();

        assert!(probe < prepare);
        let unconditional_vault_create = ["std::fs::create_dir_all", "(&vault_path)"].concat();
        assert!(!desktop.contains(&unconditional_vault_create));
    }

    #[test]
    fn peer_wal_after_a_no_lease_probe_prevents_first_install_creation() {
        let temp = tempfile::tempdir().unwrap();
        let data_path = temp.path().join("data");
        let configured_vault = temp.path().join("first-install-vault");
        std::fs::create_dir(&data_path).unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    crate::models::settings::UserSettings {
                        vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                        ..crate::models::settings::UserSettings::default()
                    },
                ),
            )
            .unwrap();
        assert!(transition_store.detached_stable_vault().unwrap().is_none());
        std::fs::write(
            data_path.join("twin/events/root-transition-v1.json"),
            b"peer-prepared",
        )
        .unwrap();

        let error = transition_store
            .prepare_runtime_vault_path(&configured_vault)
            .unwrap_err();
        assert!(error.to_string().contains("root-transition"));
        assert!(!configured_vault.exists());
    }

    #[test]
    fn peer_stable_lease_after_a_no_lease_probe_prevents_first_install_creation() {
        let temp = tempfile::tempdir().unwrap();
        let data_path = temp.path().join("data");
        let configured_vault = temp.path().join("first-install-vault");
        std::fs::create_dir(&data_path).unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                    crate::models::settings::UserSettings {
                        vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                        ..crate::models::settings::UserSettings::default()
                    },
                ),
            )
            .unwrap();
        assert!(transition_store.detached_stable_vault().unwrap().is_none());
        transition_store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    crate::models::twin_event::ContentDigest::parse("a".repeat(64)).unwrap(),
                ),
            )
            .unwrap();

        let error = transition_store
            .prepare_runtime_vault_path(&configured_vault)
            .unwrap_err();
        assert!(matches!(
            error,
            crate::services::twin_events::MutationError::Io(_)
                | crate::services::twin_events::MutationError::Store(_)
        ));
        assert!(!configured_vault.exists());
    }

    #[test]
    fn coordinator_failure_branch_never_creates_or_adopts_a_vault_descriptor() {
        let desktop = include_str!("lib.rs");
        let coordinator_result = desktop
            .find("let (event_recorder, mutation_coordinator")
            .unwrap();
        let services = desktop.find("// Initialize services").unwrap();
        assert!(
            !desktop[coordinator_result..services].contains("load_or_create_vault_identity"),
            "a failed coordinator must enter isolated recovery without touching vault identity"
        );
    }

    #[test]
    fn warm_start_does_not_reinitialize_events_after_releasing_the_root_guard() {
        let desktop = include_str!("lib.rs");
        let warm_start = desktop.find("async fn warm_start_services_inner(").unwrap();
        let root_gate = desktop
            .find("async fn acquire_warm_start_root_gate(")
            .unwrap();
        assert!(
            !desktop[warm_start..root_gate]
                .contains("initialize_twin_event_store_for_boot(&state.twin_event_store)"),
            "coordinator construction already initializes and activates the event store"
        );
    }

    #[test]
    fn pending_root_wal_failure_uses_only_ephemeral_recovery_service_roots() {
        let temp = tempfile::tempdir().unwrap();
        let data_path = temp.path().join("active-data");
        let vault_path = temp.path().join("active-vault");
        std::fs::create_dir(&data_path).unwrap();
        std::fs::create_dir(&vault_path).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(&data_path)
            .unwrap();
        let settings = crate::models::settings::UserSettings {
            vault_path: Some(vault_path.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        };
        transition_store
            .write_settings_guarded(
                &crate::services::root_transition::NonsecretSettingsV1::from_settings(settings),
            )
            .unwrap();
        transition_store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope,
                ),
            )
            .unwrap();

        // Model the peer gap after the desktop's detachment/lease probes: the
        // descriptor disappears and a peer publishes a prepared root WAL before
        // this process enters coordinator construction.
        std::fs::remove_file(vault_path.join("_grafyn/vault.json")).unwrap();
        std::fs::write(
            data_path.join("twin/events/root-transition-v1.json"),
            b"peer-prepared",
        )
        .unwrap();
        let event_store = Arc::new(TwinEventStore::new(&data_path));
        let coordinator_error = match MutationCoordinator::new_stable(
            &data_path,
            &vault_path,
            event_store,
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("the pending root WAL must reject coordinator construction"),
            Err(error) => error,
        };
        assert!(coordinator_error.to_string().contains("root-transition"));
        let data_before = snapshot_tree(&data_path);
        let vault_before = snapshot_tree(&vault_path);

        let recovery = isolated_recovery_runtime_roots(&data_path, &vault_path).unwrap();
        assert!(!recovery.data_path.starts_with(&data_path));
        assert!(!recovery.vault_path.starts_with(&vault_path));
        assert!(!recovery.derived_data_path.starts_with(&data_path));
        let recovery_path = recovery
            .recovery_runtime
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        let unavailable: Arc<dyn EventRecorder> =
            Arc::new(UnavailableEventRecorder::new(coordinator_error.to_string()));
        {
            let _knowledge = KnowledgeStore::with_event_recorder(
                recovery.vault_path.clone(),
                recovery.derived_data_path.clone(),
                unavailable.clone(),
            );
            let _search = SearchService::new(recovery.derived_data_path.clone()).unwrap();
            let _chunk = ChunkIndex::new(recovery.derived_data_path.clone()).unwrap();
            let _canvas = CanvasStore::with_event_recorder(
                crate::services::canvas_store::scoped_canvas_path(
                    &recovery.data_path,
                    &recovery.active_scope,
                ),
                unavailable.clone(),
            );
            let _twin = TwinStore::with_event_recorder_scoped(
                crate::models::settings::twin_data_path_for_scope(
                    &recovery.data_path,
                    &recovery.active_scope,
                ),
                recovery.data_path.join("twin"),
                recovery.active_scope.clone(),
                unavailable,
            );
            let _link = LinkDiscoveryService::try_new(recovery.derived_data_path.clone()).unwrap();
            let _migration =
                MarkdownMigrationService::try_new(recovery.derived_data_path.clone()).unwrap();
            let _optimizer =
                VaultOptimizerService::try_new(recovery.derived_data_path.clone()).unwrap();
            let _priority = PriorityScoringService::new(recovery.data_path.clone());
            let _retrieval = RetrievalService::new(recovery.data_path.clone());
            let _feedback = FeedbackService::new(recovery.data_path.join("feedback"));
        }

        assert_eq!(snapshot_tree(&data_path), data_before);
        assert_eq!(snapshot_tree(&vault_path), vault_before);
        assert!(!vault_path.join("_grafyn/vault.json").exists());
        let retained_runtime = recovery.recovery_runtime.as_ref().unwrap().clone();
        drop(recovery);
        assert!(recovery_path.exists());
        drop(retained_runtime);
        assert!(!recovery_path.exists());
    }

    #[test]
    fn recovery_runtime_rejects_a_temporary_base_inside_the_active_vault() {
        let active_vault = tempfile::tempdir().unwrap();
        let active_data = tempfile::tempdir().unwrap();
        let before = snapshot_tree(active_vault.path());
        let source = include_str!("lib.rs");
        let precheck = source
            .find("canonical_path_is_within(temporary_base, active_root)")
            .unwrap();
        let create = source.find(".tempdir_in(temporary_base)").unwrap();
        assert!(
            precheck < create,
            "overlap must be rejected before creation"
        );

        let error = match isolated_recovery_runtime_roots_in(
            active_vault.path(),
            active_data.path(),
            active_vault.path(),
        ) {
            Ok(_) => panic!("recovery runtime must not overlap an active vault"),
            Err(error) => error,
        };

        assert!(error.contains("overlaps the active vault root"));
        assert_eq!(snapshot_tree(active_vault.path()), before);
    }
}
