#![cfg(feature = "tauri-app")]

mod app_runtime;
mod commands;
pub mod models;
pub mod services;
#[cfg(feature = "e2e-test-runtime")]
pub mod test_runtime;

use models::boot::BootStatus;
use services::twin_events::NoopMutationLifecycle;
#[cfg(desktop)]
use services::vault_optimizer::OptimizerTick;
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
    vault_optimizer::VaultOptimizerService,
};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

pub(crate) const ANDROID_NATIVE_HEALTH_INVALID: &str = "android_native_health_invalid";

/// Application state holding all services
#[derive(Clone)]
pub struct AppState {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub canvas_store: Arc<RwLock<CanvasStore>>,
    pub openrouter: Arc<RwLock<OpenRouterService>>,
    pub ollama: Option<Arc<RwLock<OllamaService>>>,
    pub feedback_service: Arc<RwLock<FeedbackService>>,
    pub settings_service: Arc<RwLock<SettingsService>>,
    pub priority_service: Arc<RwLock<PriorityScoringService>>,
    pub retrieval_service: Arc<RwLock<RetrievalService>>,
    pub chunk_index: Arc<RwLock<ChunkIndex>>,
    pub link_discovery: Option<Arc<RwLock<LinkDiscoveryService>>>,
    pub markdown_migration: Option<Arc<RwLock<MarkdownMigrationService>>>,
    pub vault_optimizer: Option<Arc<RwLock<VaultOptimizerService>>>,
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

impl AppState {
    pub(crate) fn ollama_service(&self) -> Result<Arc<RwLock<OllamaService>>, String> {
        self.ollama
            .clone()
            .ok_or_else(|| "Local Ollama is unavailable on this runtime".to_string())
    }

    pub(crate) fn link_discovery_service(
        &self,
    ) -> Result<Arc<RwLock<LinkDiscoveryService>>, String> {
        self.link_discovery
            .clone()
            .ok_or_else(|| "Link discovery is unavailable on this runtime".to_string())
    }

    pub(crate) fn markdown_migration_service(
        &self,
    ) -> Result<Arc<RwLock<MarkdownMigrationService>>, String> {
        self.markdown_migration
            .clone()
            .ok_or_else(|| "Markdown migration is unavailable on this runtime".to_string())
    }

    pub(crate) fn vault_optimizer_service(
        &self,
    ) -> Result<Arc<RwLock<VaultOptimizerService>>, String> {
        self.vault_optimizer
            .clone()
            .ok_or_else(|| "Vault optimizer is unavailable on this runtime".to_string())
    }
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
    sync_engine: Option<Arc<SyncEngine>>,
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
        sync_engine: Some(sync_engine),
    })
}

fn initialize_local_mutation_runtime(
    data_path: impl AsRef<std::path::Path>,
    vault_path: impl AsRef<std::path::Path>,
    twin_event_store: Arc<TwinEventStore>,
) -> Result<MutationStartupRuntime, String> {
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(
            data_path,
            vault_path,
            twin_event_store,
            Arc::new(NoopMutationLifecycle),
        )
        .map_err(|error| error.to_string())?,
    );
    coordinator
        .recover_pending()
        .map_err(|error| error.to_string())?;
    Ok(MutationStartupRuntime {
        coordinator,
        sync_engine: None,
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
    settings_service: SettingsService,
    committed_warning_app: Option<tauri::AppHandle>,
) -> Result<AppState, String> {
    build_app_state_with_secure_runtime(settings_service, committed_warning_app, true)
}

fn build_app_state_with_secure_runtime(
    mut settings_service: SettingsService,
    committed_warning_app: Option<tauri::AppHandle>,
    secure_runtime_ready: bool,
) -> Result<AppState, String> {
    let desktop_runtime =
        settings_service.runtime_kind() == crate::models::runtime::RuntimeKind::Desktop;
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
        if desktop_runtime || secure_runtime_ready {
            return initialize_stable_mutation_runtime(
                &data_path,
                runtime_vault_path,
                twin_event_store.clone(),
                settings_service.secret_store(),
            );
        }
        initialize_local_mutation_runtime(&data_path, runtime_vault_path, twin_event_store.clone())
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
            runtime.sync_engine,
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
            SearchService::new(derived_data_path.clone()).map_err(|error| {
                format!("Search service initialization failed after rebuild: {error}")
            })?
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
            ChunkIndex::new(derived_data_path.clone()).map_err(|error| {
                format!("Chunk index initialization failed after rebuild: {error}")
            })?
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
    let ollama =
        desktop_runtime.then(|| OllamaService::new(settings_service.get().ollama_base_url.clone()));

    // Initialize priority scoring service
    let priority_service = PriorityScoringService::new(service_data_path.clone());

    // Initialize retrieval service
    let retrieval_service = RetrievalService::new(service_data_path.clone());
    let (link_discovery, markdown_migration, vault_optimizer) = if desktop_runtime {
        (
            Some(
                LinkDiscoveryService::try_new(derived_data_path.clone())
                    .map_err(|error| error.to_string())?,
            ),
            Some(
                MarkdownMigrationService::try_new(derived_data_path.clone())
                    .map_err(|error| error.to_string())?,
            ),
            Some(
                VaultOptimizerService::try_new(derived_data_path)
                    .map_err(|error| error.to_string())?,
            ),
        )
    } else {
        (None, None, None)
    };

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
        ollama: ollama.map(|service| Arc::new(RwLock::new(service))),
        feedback_service: Arc::new(RwLock::new(feedback_service)),
        settings_service: Arc::new(RwLock::new(settings_service)),
        priority_service: Arc::new(RwLock::new(priority_service)),
        retrieval_service: Arc::new(RwLock::new(retrieval_service)),
        chunk_index: Arc::new(RwLock::new(chunk_index)),
        link_discovery: link_discovery.map(|service| Arc::new(RwLock::new(service))),
        markdown_migration: markdown_migration.map(|service| Arc::new(RwLock::new(service))),
        vault_optimizer: vault_optimizer.map(|service| Arc::new(RwLock::new(service))),
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

fn build_recoverable_app_state(
    runtime_paths: &app_runtime::RuntimePaths,
    committed_warning_app: Option<tauri::AppHandle>,
    startup_error: impl Into<String>,
) -> Result<AppState, String> {
    let startup_error = startup_error.into();
    let recovery_base = runtime_paths
        .cache_dir
        .parent()
        .unwrap_or(runtime_paths.cache_dir.as_path());
    std::fs::create_dir_all(recovery_base).map_err(|error| {
        format!(
            "Failed to prepare the app-owned failed-boot cache {}: {error}",
            recovery_base.display()
        )
    })?;
    let recovery_runtime = Arc::new(
        tempfile::Builder::new()
            .prefix("grafyn-failed-boot-runtime-v1-")
            .tempdir_in(recovery_base)
            .map_err(|error| format!("Failed to create isolated failed-boot runtime: {error}"))?,
    );
    let bootstrap = app_runtime::RuntimeBootstrap::new(
        models::runtime::RuntimeKind::Android,
        app_runtime::RuntimePaths::android(
            recovery_runtime.path().join("app-data"),
            recovery_runtime.path().join("app-cache"),
        ),
        Arc::new(app_runtime::UnavailableSecretStore),
        models::runtime::RuntimeFeatureStatusV1::unavailable(
            "failed_boot_recovery",
            "Secure secrets are unavailable while startup recovery is active.",
        ),
        models::runtime::RuntimeFeatureStatusV1::unavailable(
            "failed_boot_recovery",
            "Native image sharing is unavailable while startup recovery is active.",
        ),
    );
    let settings = SettingsService::load_for_runtime(&bootstrap)
        .map_err(|error| format!("Failed to initialize isolated recovery settings: {error}"))?;
    let mut state = build_app_state_with_secure_runtime(settings, committed_warning_app, false)
        .map_err(|error| format!("Failed to initialize isolated recovery services: {error}"))?;
    state.mutation_coordinator = None;
    state.sync_engine = None;
    state.mutation_startup_error = Arc::new(RwLock::new(Some(startup_error.clone())));
    state.boot_state = Arc::new(RwLock::new(BootStatus::failed(
        "failed",
        "Startup failed",
        startup_error,
    )));
    state._recovery_runtime = Some(recovery_runtime);
    Ok(state)
}

#[cfg(desktop)]
fn runtime_bootstrap_for_app(
    _app: &tauri::App<tauri::Wry>,
) -> anyhow::Result<app_runtime::RuntimeBootstrap> {
    Ok(app_runtime::RuntimeBootstrap::desktop())
}

#[cfg(target_os = "android")]
fn runtime_bootstrap_for_app(
    app: &tauri::App<tauri::Wry>,
) -> anyhow::Result<app_runtime::RuntimeBootstrap> {
    let app_data_dir = app.path().app_data_dir()?;
    let app_cache_dir = app.path().app_cache_dir()?;
    let paths = app_runtime::RuntimePaths::android(app_data_dir, app_cache_dir);
    let Some(bridge) = app.try_state::<services::android_bridge::AndroidBridge<tauri::Wry>>()
    else {
        return Ok(app_runtime::RuntimeBootstrap::new(
            models::runtime::RuntimeKind::Android,
            paths,
            Arc::new(app_runtime::UnavailableSecretStore),
            models::runtime::RuntimeFeatureStatusV1::unavailable(
                "android_bridge_unavailable",
                "Android secure secret storage is unavailable.",
            ),
            models::runtime::RuntimeFeatureStatusV1::unavailable(
                "android_bridge_unavailable",
                "Android native image sharing is unavailable.",
            ),
        ));
    };
    let (secure_secrets, mut native_image_share, startup_error) = match bridge.health() {
        Ok(health) => (
            match health.secure_secrets {
                services::android_bridge::NativeHealth::Ready => {
                    models::runtime::RuntimeFeatureStatusV1::ready()
                }
                services::android_bridge::NativeHealth::Unavailable => {
                    models::runtime::RuntimeFeatureStatusV1::unavailable(
                        "android_keystore_unavailable",
                        "Android secure secret storage is unavailable.",
                    )
                }
            },
            match health.native_image_share {
                services::android_bridge::NativeHealth::Ready => {
                    models::runtime::RuntimeFeatureStatusV1::ready()
                }
                services::android_bridge::NativeHealth::Unavailable => {
                    models::runtime::RuntimeFeatureStatusV1::unavailable(
                        "android_image_share_unavailable",
                        "Android native image sharing is unavailable.",
                    )
                }
            },
            None,
        ),
        Err(_) => (
            models::runtime::RuntimeFeatureStatusV1::unavailable(
                ANDROID_NATIVE_HEALTH_INVALID,
                "Android native health is invalid; startup recovery is required.",
            ),
            models::runtime::RuntimeFeatureStatusV1::unavailable(
                ANDROID_NATIVE_HEALTH_INVALID,
                "Android native health is invalid; startup recovery is required.",
            ),
            Some(ANDROID_NATIVE_HEALTH_INVALID),
        ),
    };
    match std::fs::canonicalize(&paths.share_dir) {
        Ok(expected_share) if bridge.share_root() == Some(expected_share.as_path()) => {}
        _ => {
            native_image_share = models::runtime::RuntimeFeatureStatusV1::unavailable(
                "android_image_share_root_unavailable",
                "Android native image sharing storage is unavailable.",
            );
        }
    }
    let bootstrap = app_runtime::RuntimeBootstrap::new(
        models::runtime::RuntimeKind::Android,
        paths,
        bridge.secret_store(),
        secure_secrets,
        native_image_share,
    );
    Ok(match startup_error {
        Some(error) => bootstrap.with_startup_error(error),
        None => bootstrap,
    })
}

#[cfg(target_os = "android")]
fn register_mobile_commands(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        commands::runtime::get_runtime_status,
        commands::boot::get_boot_status,
        commands::notes::list_notes,
        commands::notes::get_note,
        commands::notes::create_note,
        commands::notes::update_note,
        commands::notes::delete_note,
        commands::canvas::list_sessions,
        commands::canvas::get_session,
        commands::canvas::create_session,
        commands::canvas::update_session,
        commands::canvas::delete_session,
        commands::canvas::get_available_models,
        commands::canvas::send_prompt,
        commands::canvas::regenerate_response,
        commands::twin::get_twin_review,
        commands::twin::record_canvas_feedback,
        commands::twin::list_decision_episodes,
        commands::twin::get_decision_mirror_config,
        commands::twin::list_memory_digest,
        commands::twin::review_memory_digest_item,
        commands::twin::list_constitution_items,
        commands::twin::list_action_gaps,
        commands::twin::get_constitution_setup,
        commands::twin_state::list_twin_observations,
        commands::twin_state::list_twin_proposals,
        commands::twin_state::create_companion_capture,
        commands::twin_state::review_twin_proposal,
        commands::twin_state::get_twin_state_projection,
        commands::twin_state::rank_twin_attention,
        commands::twin_state::get_twin_event_timeline,
        commands::image_generation::discover_image_models,
        commands::image_generation::get_image_model_capability,
        commands::image_generation::generate_image,
        commands::image_generation::discard_generated_image_receipt,
        commands::image_generation::save_generated_image,
        commands::image_generation::load_generated_image,
        commands::image_generation::share_generated_image,
        commands::settings::get_settings,
        commands::settings::get_settings_status,
        commands::settings::update_settings,
        commands::settings::get_openrouter_status,
        commands::sync::get_sync_status,
        commands::memory::recall_relevant,
    ])
}

#[cfg(desktop)]
fn register_desktop_commands(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        commands::runtime::get_runtime_status,
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
        // Evidence workspace (desktop-only)
        commands::evidence::evidence_snapshot,
        commands::evidence::create_evidence_pilot,
        commands::evidence::save_evidence_interview,
        commands::evidence::save_evidence_goal,
        commands::evidence::review_evidence_relationship,
        commands::evidence::review_evidence_statement,
        commands::evidence::process_evidence_jobs,
        commands::evidence::predict_evidence_decision,
        commands::evidence::install_evidence_embeddings,
        commands::evidence::record_evidence_choice,
        commands::evidence::list_evidence_predictions,
        // Retrieval commands
        commands::retrieval::retrieve_relevant,
        commands::retrieval::get_retrieval_config,
        commands::retrieval::update_retrieval_config,
    ])
}

fn initialize_runtime_state(
    bootstrap: &mut app_runtime::RuntimeBootstrap,
    committed_warning_app: Option<tauri::AppHandle>,
) -> Result<AppState, String> {
    let recovery_paths = bootstrap.paths.clone();
    let secure_runtime_ready = bootstrap.secure_secrets.is_ready();
    let initialized = match bootstrap.startup_error() {
        Some(error) => Err(error.to_owned()),
        None => SettingsService::load_for_runtime(bootstrap)
            .map_err(|error| error.to_string())
            .and_then(|settings| {
                bootstrap.paths.vault_dir = settings.vault_path();
                build_app_state_with_secure_runtime(
                    settings,
                    committed_warning_app.clone(),
                    secure_runtime_ready,
                )
            }),
    };
    match initialized {
        Ok(state) => Ok(state),
        Err(error) => {
            log::error!("Startup entered recoverable failed-boot mode: {error}");
            build_recoverable_app_state(&recovery_paths, committed_warning_app, error)
        }
    }
}

fn configure_runtime(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.setup(|app| {
        let mut bootstrap = runtime_bootstrap_for_app(app)?;
        let state = initialize_runtime_state(&mut bootstrap, Some(app.handle().clone()))?;
        let canonical_runtime_available = state.mutation_coordinator.is_some()
            && state
                .mutation_startup_error
                .try_read()
                .map(|error| error.is_none())
                .unwrap_or(false);
        let runtime_status =
            bootstrap.status_for_state(canonical_runtime_available, state.sync_engine.is_some());

        app.manage(runtime_status);
        app.manage(state);

        let app_handle = app.handle().clone();
        let state = app.state::<AppState>().inner().clone();
        tauri::async_runtime::spawn(async move {
            match warm_start_services(app_handle.clone(), state.clone()).await {
                Ok(()) => {
                    #[cfg(desktop)]
                    {
                        start_link_discovery_worker(state.clone());
                        start_vault_optimizer_worker(state.clone());
                        commands::evidence::start_worker(state.clone());
                    }
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
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    #[cfg(all(desktop, feature = "desktop-updater"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(all(desktop, feature = "desktop-process"))]
    let builder = builder.plugin(tauri_plugin_process::init());

    #[cfg(target_os = "android")]
    let builder = builder.plugin(services::android_bridge::init());

    let builder = configure_runtime(builder);

    #[cfg(desktop)]
    let builder = register_desktop_commands(builder);

    #[cfg(target_os = "android")]
    let builder = register_mobile_commands(builder);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        log::error!("Error while running Tauri application: {error}");
    }
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
    } else {
        update_boot_state(&state.boot_state, &status).await;
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
    let result =
        warm_start_services_inner_with_gap(app_handle, state, injected_failure, || Ok(())).await;
    if let Err(error) = result.as_ref() {
        record_warm_start_failure(app_handle, state, error).await;
    }
    result
}

async fn record_warm_start_failure(
    app_handle: Option<&tauri::AppHandle>,
    state: &AppState,
    error: &str,
) {
    *state.mutation_startup_error.write().await = Some(error.to_string());
    let status = BootStatus::failed("failed", "Startup failed", error.to_string());
    if let Some(app_handle) = app_handle {
        publish_boot_status(app_handle, state, status).await;
    } else {
        update_boot_state(&state.boot_state, &status).await;
    }
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
        if let Some(service) = state.markdown_migration.as_ref() {
            warm_start_component(WarmStartComponent::Migration, injected_failure)?;
            let migration = service.read().await;
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

#[cfg(desktop)]
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
                let mut discovery = state
                    .link_discovery
                    .as_ref()
                    .expect("desktop runtime provides link discovery")
                    .write()
                    .await;
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
                let mut discovery = state
                    .link_discovery
                    .as_ref()
                    .expect("desktop runtime provides link discovery")
                    .write()
                    .await;
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

#[cfg(desktop)]
fn optimizer_worker_failure_commit(
    error: &anyhow::Error,
) -> Option<crate::services::twin_events::MutationCommit> {
    error
        .downcast_ref::<crate::services::twin_events::MutationError>()
        .and_then(|error| error.authority_advanced_commit())
}

#[cfg(desktop)]
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
                let mut optimizer = state
                    .vault_optimizer
                    .as_ref()
                    .expect("desktop runtime provides vault optimizer")
                    .write()
                    .await;
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
                            let mut optimizer = state
                                .vault_optimizer
                                .as_ref()
                                .expect("desktop runtime provides vault optimizer")
                                .write()
                                .await;
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
                            let mut optimizer = state
                                .vault_optimizer
                                .as_ref()
                                .expect("desktop runtime provides vault optimizer")
                                .write()
                                .await;
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
mod lib_tests;

#[cfg(test)]
mod app_runtime_tests;
