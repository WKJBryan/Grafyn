#![cfg(feature = "tauri-app")]

mod commands;
pub mod models;
pub mod services;

use models::boot::BootStatus;
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
    twin::TwinStore,
    twin_events::{
        EventRecorder, MutationCoordinator, NoopMutationLifecycle, TwinEventStore,
        UnavailableEventRecorder,
    },
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
    pub mutation_startup_error: Arc<RwLock<Option<String>>>,
    pub(crate) loaded_authority:
        Arc<RwLock<Option<crate::services::vault_namespace::VaultAuthorityTokenV1>>>,
    pub vault_transition: Arc<RwLock<()>>,
    /// MemoryService is stateless — no lock needed, just Arc for shared ownership
    pub memory_service: Arc<MemoryService>,
    pub boot_state: Arc<RwLock<BootStatus>>,
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
            let mut settings_service = SettingsService::load()?;

            let vault_path = settings_service.vault_path();
            let data_path = settings_service.data_path();

            log::info!("Vault path: {:?}", vault_path);
            log::info!("Data path: {:?}", data_path);

            // Create directories if they don't exist
            if let Err(e) = std::fs::create_dir_all(&vault_path) {
                log::error!(
                    "Failed to create vault directory {}: {}",
                    vault_path.display(),
                    e
                );
            }
            if let Err(e) = std::fs::create_dir_all(&data_path) {
                log::error!(
                    "Failed to create data directory {}: {}",
                    data_path.display(),
                    e
                );
            }

            // Initialize canonical mutation capture before any command can mutate user bytes.
            let twin_event_store = Arc::new(TwinEventStore::new(data_path.clone()));
            let coordinator = (|| -> Result<Arc<MutationCoordinator>, String> {
                twin_event_store
                    .initialize()
                    .map_err(|error| error.to_string())?;
                let coordinator = Arc::new(
                    MutationCoordinator::new(
                        &data_path,
                        &vault_path,
                        twin_event_store.clone(),
                        Arc::new(NoopMutationLifecycle),
                    )
                    .map_err(|error| error.to_string())?,
                );
                coordinator
                    .recover_pending()
                    .map_err(|error| error.to_string())?;
                Ok(coordinator)
            })();
            let (event_recorder, mutation_coordinator, mutation_startup_error): (
                Arc<dyn EventRecorder>,
                Option<Arc<MutationCoordinator>>,
                Option<String>,
            ) = match coordinator {
                    Ok(coordinator) => (coordinator.clone(), Some(coordinator), None),
                    Err(error) => (
                        Arc::new(UnavailableEventRecorder::new(error.clone())),
                        None,
                        Some(error),
                    ),
                };
            let derived_data_path = if let Some(coordinator) = mutation_coordinator.as_ref() {
                let guard = coordinator
                    .begin_root_transition()
                    .map_err(|error| error.to_string())?;
                let lease = guard.current_lease().map_err(|error| error.to_string())?;
                guard
                    .initialize_namespace(&lease)
                    .map_err(|error| error.to_string())?;
                guard
                    .invalidate_namespace(&lease)
                    .map_err(|error| error.to_string())?;
                coordinator
                    .current_namespace_path()
                    .map_err(|error| error.to_string())?
            } else {
                let scope = crate::services::twin_events::root_identity_for_path(&vault_path)
                    .map_err(|error| error.to_string())?;
                crate::services::vault_namespace::scoped_data_path(&data_path, &scope)
            };

            // Initialize services
            let knowledge_store = KnowledgeStore::with_event_recorder(
                vault_path.clone(),
                derived_data_path.clone(),
                event_recorder.clone(),
            );
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
                Err(e) => return Err(e.into()),
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

            let canvas_store =
                CanvasStore::with_event_recorder(data_path.join("canvas"), event_recorder.clone());
            let twin_data_path = if let Some(coordinator) = mutation_coordinator.as_ref() {
                let guard = coordinator
                    .begin_root_transition()
                    .map_err(|error| error.to_string())?;
                let lease = guard.current_lease().map_err(|error| error.to_string())?;
                guard
                    .prepare_twin_data_path(&vault_path, &lease)
                    .map_err(|error| error.to_string())?
            } else {
                crate::models::settings::twin_data_path_for_vault(&data_path, &vault_path)
                    .map_err(|error| error.to_string())?
            };
            let twin_store = TwinStore::with_event_recorder(
                twin_data_path,
                data_path.join("twin"),
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
            let priority_service = PriorityScoringService::new(data_path.clone());

            // Initialize retrieval service
            let retrieval_service = RetrievalService::new(data_path.clone());
            let link_discovery = LinkDiscoveryService::try_new(derived_data_path.clone())
                .map_err(|error| error.to_string())?;
            let markdown_migration = MarkdownMigrationService::try_new(derived_data_path.clone())
                .map_err(|error| error.to_string())?;
            let vault_optimizer = VaultOptimizerService::try_new(derived_data_path)
                .map_err(|error| error.to_string())?;

            // Initialize feedback service using runtime environment only.
            // Release builds must not embed repository credentials.
            let feedback_service = FeedbackService::new(data_path.join("feedback"));
            let boot_state = Arc::new(RwLock::new(BootStatus::default()));

            // Create app state (MemoryService is stateless — no RwLock needed)
            let state = AppState {
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
                mutation_startup_error: Arc::new(RwLock::new(mutation_startup_error)),
                loaded_authority: Arc::new(RwLock::new(None)),
                vault_transition: Arc::new(RwLock::new(())),
                memory_service: Arc::new(MemoryService::new()),
                boot_state,
            };

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
    let _root_epoch = acquire_warm_start_root_gate(state).await?;
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .ok_or_else(|| "mutation coordinator is unavailable".to_string())?;
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
    let boot_started = Instant::now();

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("opening_twin_events", "Opening governed Twin history"),
    )
    .await;
    initialize_twin_event_store_for_boot(&state.twin_event_store)?;
    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("opening_store", "Loading notes from your vault"),
    )
    .await;

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
    let namespace_token = {
        let guard = coordinator
            .begin_root_transition()
            .map_err(|error| error.to_string())?;
        let current_lease = guard.current_lease().map_err(|error| error.to_string())?;
        if current_lease != namespace_lease {
            return Err("vault authority changed before derived rebuild".into());
        }
        guard
            .capture_authority_token(&current_lease)
            .map_err(|error| error.to_string())?
    };

    // Reload authoritative inputs only after the exact generation is captured;
    // notes returned by normalization and previously populated Twin caches may
    // belong to an older peer generation.
    let full_notes = {
        let mut knowledge = state.knowledge_store.write().await;
        knowledge.reload_authoritative_state();
        knowledge
            .list_full_notes()
            .map_err(|error| error.to_string())?
    };
    warm_start_component(WarmStartComponent::TwinCaches, injected_failure)?;
    state
        .twin_store
        .write()
        .await
        .rebuild_mutation_caches()
        .map_err(|error| error.to_string())?;
    coordinator
        .validate_authority_token(&namespace_token, false)
        .map_err(|error| error.to_string())?;

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_graph", "Building graph from your notes"),
    )
    .await;

    warm_start_component(WarmStartComponent::Graph, injected_failure)?;
    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&full_notes);
    }

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_search_index", "Building search index"),
    )
    .await;

    warm_start_component(WarmStartComponent::Search, injected_failure)?;
    {
        let mut search = state.search_service.write().await;
        search.reindex_all(&full_notes).map_err(|e| e.to_string())?;
    }

    maybe_publish_boot_phase(
        app_handle,
        state,
        &boot_started,
        BootStatus::new("building_chunk_index", "Building chunk index"),
    )
    .await;

    warm_start_component(WarmStartComponent::Chunk, injected_failure)?;
    {
        let mut chunks = state.chunk_index.write().await;
        chunks
            .reindex_all(&full_notes)
            .map_err(|error| error.to_string())?;
    }

    warm_start_component(WarmStartComponent::LinkDiscovery, injected_failure)?;
    {
        let mut discovery = state.link_discovery.write().await;
        discovery
            .bootstrap_checked(&full_notes)
            .map_err(|error| error.to_string())?;
    }

    warm_start_component(WarmStartComponent::Optimizer, injected_failure)?;
    {
        let mut optimizer = state.vault_optimizer.write().await;
        optimizer
            .bootstrap_checked(&full_notes)
            .map_err(|error| error.to_string())?;
    }

    let publish_guard = coordinator
        .begin_root_transition()
        .map_err(|error| error.to_string())?;
    let current_lease = publish_guard
        .current_lease()
        .map_err(|error| error.to_string())?;
    if current_lease != namespace_lease
        || crate::services::vault_namespace::scoped_data_path(
            coordinator.data_path(),
            &current_lease.root_scope,
        ) != namespace_path
    {
        return Err("vault authority changed while derived state was rebuilding".into());
    }
    publish_guard
        .publish_namespace_ready(&namespace_token)
        .map_err(|error| error.to_string())?;
    *state.loaded_authority.write().await = Some(namespace_token);

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
                            crate::services::twin_events::MutationError::Invalid(
                                error.to_string(),
                            )
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

            let completion_ticket = match crate::commands::acquire_expected_derived_root_epoch(
                &state,
                &root_epoch,
            )
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
                            crate::services::twin_events::MutationError::Invalid(
                                error.to_string(),
                            )
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
            let (tick, prepared_optimizer_revision, prepared_commit) = {
                let store = state.knowledge_store.read().await;
                store.clear_last_mutation_commit();
                let mut optimizer = state.vault_optimizer.write().await;
                let tick = optimizer.with_locked_fresh_state(|optimizer| {
                    optimizer.prepare_next_expecting_authority(
                        &store,
                        &settings,
                        root_guard.authority().clone(),
                    )
                });
                let revision = optimizer.state_revision();
                let commit = store.take_last_mutation_commit();
                (tick, revision, commit)
            };

            // Whichever branch below reindexes a note, it does so only AFTER
            // the block that produced `applied_note_id` has ended and its
            // `knowledge_store`/`vault_optimizer` guards have dropped —
            // `commit_note_index_refresh` reacquires `knowledge_store` itself
            // (see its doc comment in `commands/mod.rs` for why this can't
            // just be a call to `commit_note_write`).
            let (applied_note_id, mutation_commit) = match tick {
                Ok(OptimizerTick::Idle) => (None, None),
                Ok(OptimizerTick::Applied(note_id)) => (Some(note_id), prepared_commit),
                Ok(OptimizerTick::Pending(pending)) => {
                    // Only non-`sidecar_first` edit modes reach here, and only
                    // for the narrow `update_note` write itself — acquire the
                    // write lock just for this, in the same canonical order.
                    let result = {
                        let mut store = state.knowledge_store.write().await;
                        store.clear_last_mutation_commit();
                        let mut optimizer = state.vault_optimizer.write().await;
                        let result = optimizer.with_locked_fresh_state(|optimizer| {
                            if optimizer.state_revision() != prepared_optimizer_revision {
                                anyhow::bail!(
                                    "vault optimizer state changed before pending apply"
                                );
                            }
                            optimizer.apply_pending(&mut store, *pending)
                        });
                        let commit = store.take_last_mutation_commit();
                        (result, commit)
                    };
                    match result {
                        (Ok(applied_note_id), commit) => (applied_note_id, commit),
                        (Err(error), _) => {
                            log::warn!("Background vault optimizer failed to apply: {}", error);
                            (None, None)
                        }
                    }
                }
                Err(error) => {
                    log::warn!("Background vault optimizer failed to prepare: {}", error);
                    (None, None)
                }
            };

            if let Some(commit) = mutation_commit {
                crate::commands::repair_after_authority_mutation(
                    &state,
                    &commit,
                    "vault optimizer",
                )
                .await;
            } else if let Some(note_id) = applied_note_id {
                log::error!(
                    "Vault optimizer changed note '{}' without exposing its mutation commit",
                    note_id
                );
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

    fn build_warm_start_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let (mut state, vault, data) =
            crate::commands::commit_note_write_tests::build_test_state();
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
            crate::services::markdown_migration::MarkdownMigrationService::new(
                discarded.clone(),
            ),
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

    #[tokio::test]
    async fn update_boot_state_replaces_existing_status() {
        let boot_state = Arc::new(RwLock::new(BootStatus::default()));
        let next = BootStatus::new("building_search_index", "Building search index");

        update_boot_state(&boot_state, &next).await;

        assert_eq!(*boot_state.read().await, next);
    }

    #[tokio::test]
    async fn warm_start_waits_behind_root_transition_before_reading_services() {
        let (state, _vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let transition = state.vault_transition.write().await;
        let task_state = state.clone();
        let task = tokio::spawn(async move { acquire_warm_start_root_gate(&task_state).await });
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "warm start must wait behind root transition");
        drop(transition);
        drop(task.await.unwrap().unwrap());
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
        assert!(desktop.contains("TwinStore::with_event_recorder("));
        let desktop_noop = [
            "let twin_store = TwinStore::",
            "new(settings_service.get().effective_twin_data_path())",
        ]
        .concat();
        assert!(!desktop.contains(&desktop_noop));
        let retarget = settings
            .find("twin.replace_root_path(candidate_twin)")
            .unwrap();
        let publish = settings
            .find("settings.publish_runtime_authority(")
            .unwrap();
        assert!(retarget < publish);
        let settings_noop = ["TwinStore::", "new(new_twin_path)"].concat();
        assert!(!settings.contains(&settings_noop));
    }
}
