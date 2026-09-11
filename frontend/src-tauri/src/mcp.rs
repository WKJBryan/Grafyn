//! Grafyn MCP Server Binary
//!
//! Standalone MCP server for Claude Desktop integration.
//! Communicates over stdio using the Model Context Protocol.
//!
//! Usage:
//!   grafyn-mcp [--vault <path>] [--data <path>]
//!
//! If paths are not specified, reads from Grafyn's settings.json,
//! falling back to default ~/Documents/Grafyn/ paths.

// The MCP binary shares modules with the Tauri app but only uses a subset.
// Suppress dead_code warnings for the unused services/models.
#![allow(dead_code)]

mod mcp_tools;
mod models;
mod services;

use crate::mcp_tools::GrafynMcpServer;
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::link_discovery::LinkDiscoveryService;
use crate::services::memory::MemoryService;
use crate::services::priority::PriorityScoringService;
use crate::services::retrieval::RetrievalService;
use crate::services::search::SearchService;
use crate::services::settings::SettingsService;
use crate::services::sync::engine::SyncEngine;
use crate::services::sync::identity::VaultIdentity;
use crate::services::sync::secrets::{SecretAccount, SecretBytes, SecretStore, SecretStoreError};
use crate::services::twin::TwinStore;
#[cfg(test)]
use crate::services::twin_events::NoopMutationLifecycle;
use crate::services::twin_events::{
    MutationCoordinator, MutationError, MutationIntentV1, MutationLifecycle, TwinEventStore,
};
use crate::services::vault_optimizer::VaultOptimizerService;
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

/// Grafyn MCP Server — exposes your knowledge base to Claude Desktop
#[derive(Parser, Debug)]
#[command(name = "grafyn-mcp", version, about)]
struct Args {
    /// Path to the vault directory (markdown notes)
    #[arg(long)]
    vault: Option<PathBuf>,

    /// Path to the data directory (search index, settings)
    #[arg(long)]
    data: Option<PathBuf>,
}

fn prepare_mcp_root_directories(
    vault_path: &std::path::Path,
    data_path: &std::path::Path,
) -> Result<Option<VaultIdentity>, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_path)?;
    crate::services::twin_events::validate_real_directory(data_path, "MCP data root")?;
    crate::services::twin_events::AnchoredRoot::open(data_path)?
        .open_directory("twin/events", true)?;
    let process_lock =
        crate::services::twin_events::acquire_shared_coordinator_process_lock(data_path)?;
    let result =
        (|| -> Result<Option<VaultIdentity>, crate::services::twin_events::MutationError> {
            crate::services::root_transition::reject_transition_wal_locked(
                data_path,
                &process_lock,
            )?;
            let prepared_scope =
                crate::services::twin_events::prepared_stable_migration_scope_locked(
                    data_path,
                    &process_lock,
                )?;
            let durable_scope = crate::services::root_transition::stable_root_scope_locked(
                data_path,
                &process_lock,
            )?;
            if durable_scope
                .as_ref()
                .zip(prepared_scope.as_ref())
                .is_some_and(|(durable, prepared)| durable != prepared)
            {
                return Err(
                    crate::services::twin_events::MutationError::RecoveryConflict(
                        "prepared stable migration conflicts with the durable stable lease".into(),
                    ),
                );
            }
            let identity = if let Some(stable_scope) = durable_scope.or(prepared_scope) {
                crate::services::twin_events::validate_real_directory(
                    vault_path,
                    "stable MCP vault root",
                )?;
                let identity = crate::services::sync::identity::load_vault_identity(vault_path)?;
                if identity.root_scope != stable_scope {
                    return Err(
                        crate::services::twin_events::MutationError::RecoveryConflict(
                            "configured MCP vault does not match the durable stable lease".into(),
                        ),
                    );
                }
                Some(identity)
            } else {
                std::fs::create_dir_all(vault_path)?;
                crate::services::twin_events::validate_real_directory(
                    vault_path,
                    "MCP vault root",
                )?;
                None
            };
            Ok(identity)
        })();
    process_lock.unlock()?;
    Ok(result?)
}

#[derive(Debug)]
struct UnavailableSyncSecretStore;

impl SecretStore for UnavailableSyncSecretStore {
    fn put(&self, _account: &SecretAccount, _secret: &SecretBytes) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }

    fn get(&self, _account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }

    fn delete(&self, _account: &SecretAccount) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }
}

struct McpMutationRuntime {
    coordinator: Arc<MutationCoordinator>,
    sync_engine: Arc<SyncEngine>,
}

struct McpStartupAuthority {
    startup_token: crate::services::vault_namespace::VaultAuthorityTokenV1,
    derived_token: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
}

// A fresh custom root has no identity until the coordinator completes its
// legacy-ownership audit. Recovery must fail closed during that narrow gap.
#[derive(Debug, Default)]
struct FreshCustomSyncLifecycle {
    engine: Mutex<Option<Arc<SyncEngine>>>,
}

impl FreshCustomSyncLifecycle {
    fn bind(&self, engine: Arc<SyncEngine>) -> Result<(), MutationError> {
        let mut current = self
            .engine
            .lock()
            .map_err(|_| MutationError::Invalid("custom sync lifecycle lock poisoned".into()))?;
        if current.is_some() {
            return Err(MutationError::RecoveryConflict(
                "custom sync lifecycle is already bound".into(),
            ));
        }
        *current = Some(engine);
        Ok(())
    }

    fn with_engine(
        &self,
        action: impl FnOnce(&SyncEngine) -> Result<(), MutationError>,
    ) -> Result<(), MutationError> {
        let current = self
            .engine
            .lock()
            .map_err(|_| MutationError::Invalid("custom sync lifecycle lock poisoned".into()))?;
        let engine = current.as_ref().ok_or_else(|| {
            MutationError::RecoveryConflict(
                "custom sync lifecycle is not bound during startup recovery".into(),
            )
        })?;
        action(engine)
    }
}

impl MutationLifecycle for FreshCustomSyncLifecycle {
    fn stage_before_local(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        self.with_engine(|engine| engine.stage_before_local(intent))
    }

    fn committed(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        self.with_engine(|engine| engine.committed(intent))
    }

    fn known_failure(&self, mutation_id: Option<&str>, reason: &str) -> Result<(), MutationError> {
        self.with_engine(|engine| engine.known_failure(mutation_id, reason))
    }
}

fn initialize_mcp_mutation_runtime(
    data_path: &std::path::Path,
    vault_path: &std::path::Path,
    custom_data: bool,
    identity: Option<VaultIdentity>,
    secret_store: Option<Arc<dyn SecretStore>>,
    twin_event_store: Arc<TwinEventStore>,
) -> Result<McpMutationRuntime, MutationError> {
    let attach_device = secret_store.is_some();
    let secret_store: Arc<dyn SecretStore> =
        secret_store.unwrap_or_else(|| Arc::new(UnavailableSyncSecretStore));
    let (coordinator, sync_engine) = if let Some(identity) = identity {
        let root_key = if attach_device {
            crate::services::sync::vault_keys::load_vault_root_key(
                secret_store.as_ref(),
                &identity.descriptor.vault_id().to_string(),
            )
            .map_err(|error| MutationError::Invalid(error.to_string()))?
        } else {
            None
        };
        let sync_engine = Arc::new(SyncEngine::open_core(
            data_path,
            vault_path,
            identity,
            root_key,
            secret_store.clone(),
            twin_event_store.clone(),
        )?);
        let coordinator = Arc::new(if custom_data {
            MutationCoordinator::new_custom_mcp(
                data_path,
                vault_path,
                twin_event_store,
                sync_engine.clone(),
            )?
        } else {
            MutationCoordinator::new_stable(
                data_path,
                vault_path,
                twin_event_store,
                sync_engine.clone(),
            )?
        });
        (coordinator, sync_engine)
    } else {
        if !custom_data {
            return Err(MutationError::Invalid(
                "default MCP startup requires a validated vault identity".into(),
            ));
        }
        let lifecycle = Arc::new(FreshCustomSyncLifecycle::default());
        let coordinator = Arc::new(MutationCoordinator::new_custom_mcp(
            data_path,
            vault_path,
            twin_event_store.clone(),
            lifecycle.clone(),
        )?);
        let identity = crate::services::sync::identity::load_vault_identity(vault_path)?;
        let root_key = if attach_device {
            crate::services::sync::vault_keys::load_vault_root_key(
                secret_store.as_ref(),
                &identity.descriptor.vault_id().to_string(),
            )
            .map_err(|error| MutationError::Invalid(error.to_string()))?
        } else {
            None
        };
        let sync_engine = Arc::new(SyncEngine::open_core(
            data_path,
            vault_path,
            identity,
            root_key,
            secret_store.clone(),
            twin_event_store,
        )?);
        lifecycle.bind(sync_engine.clone())?;
        (coordinator, sync_engine)
    };
    if attach_device {
        let device = coordinator.load_or_create_device_signing_identity(secret_store)?;
        sync_engine.attach_device_identity(device)?;
    }
    coordinator.recover_pending()?;
    Ok(McpMutationRuntime {
        coordinator,
        sync_engine,
    })
}

fn prepare_mcp_startup_authority(
    data_path: &std::path::Path,
    runtime: &McpMutationRuntime,
    knowledge_store: &mut KnowledgeStore,
) -> Result<McpStartupAuthority, Box<dyn std::error::Error>> {
    runtime
        .sync_engine
        .recover_pending_inbox(&runtime.coordinator)?;
    runtime
        .sync_engine
        .bootstrap_existing_vault(&runtime.coordinator, knowledge_store)?;

    let startup_token = runtime.coordinator.current_authority_token()?;
    runtime
        .coordinator
        .validate_authority_token(&startup_token, false)?;
    if runtime
        .coordinator
        .validate_authority_token(&startup_token, true)
        .is_err()
    {
        rebuild_mcp_remote_authority(
            data_path,
            &runtime.coordinator,
            knowledge_store,
            &startup_token,
        )?;
    }
    runtime
        .coordinator
        .validate_authority_token(&startup_token, true)?;
    Ok(McpStartupAuthority {
        startup_token: startup_token.clone(),
        derived_token: Some(startup_token),
    })
}

fn rebuild_mcp_remote_authority(
    data_path: &std::path::Path,
    coordinator: &Arc<MutationCoordinator>,
    knowledge_store: &mut KnowledgeStore,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<(), Box<dyn std::error::Error>> {
    knowledge_store.reload_authoritative_state();
    let notes = knowledge_store.list_full_notes()?;
    let twin_path =
        crate::models::settings::twin_data_path_for_scope(data_path, &expected.root_scope);
    let mut twin_store = TwinStore::with_event_recorder_scoped(
        twin_path,
        data_path.join("twin"),
        expected.root_scope.clone(),
        coordinator.clone(),
    );
    let derived_path =
        crate::services::vault_namespace::scoped_data_path(data_path, &expected.root_scope);
    let mut search = SearchService::new(derived_path.clone())?;
    let mut chunks = ChunkIndex::new(derived_path.clone())?;
    let mut discovery = LinkDiscoveryService::try_new(derived_path.clone())?;
    let mut optimizer = VaultOptimizerService::try_new(derived_path)?;
    let _optimizer_state_lock = optimizer.acquire_state_lock()?;

    let guard = coordinator.begin_root_transition()?;
    let lease = guard.current_lease()?;
    let authority = guard.capture_authority_token(&lease)?;
    if &authority != expected {
        return Err("vault authority changed before MCP remote repair".into());
    }
    guard.invalidate_namespace(&lease)?;

    twin_store.rebuild_mutation_caches()?;
    search.reindex_all(&notes)?;
    chunks.reindex_all(&notes)?;
    let mut graph = GraphIndex::new();
    graph.build_from_notes(&notes);
    discovery.reload_from_disk_checked()?;
    discovery.bootstrap_checked(&notes)?;
    optimizer.reload_from_disk_checked()?;
    optimizer.recover_pending_publications_locked(knowledge_store, &guard)?;

    guard.validate_authority_token(&authority, false)?;
    guard.publish_namespace_ready(&authority)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to stderr (stdout is reserved for MCP protocol)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let args = Args::parse();

    // Resolve paths: CLI args > settings.json > defaults
    let ResolvedMcpPaths {
        vault_path,
        data_path,
        custom_data,
        secret_store,
    } = resolve_paths(args.vault, args.data)?;

    log::info!("Vault path: {}", vault_path.display());
    log::info!("Data path: {}", data_path.display());

    // A stable lease owns its vault identity. Never manufacture a replacement
    // root or descriptor when the configured vault was moved or is unavailable.
    let prepared_identity = prepare_mcp_root_directories(&vault_path, &data_path)?;
    let vault_identity = if custom_data {
        prepared_identity
    } else {
        Some(match prepared_identity {
            Some(identity) => identity,
            None => crate::services::sync::identity::load_or_create_vault_identity(&vault_path)?,
        })
    };
    // Recover the same canonical mutation journal as desktop before stdio is served.
    let twin_event_store = Arc::new(TwinEventStore::new(&data_path));
    let mutation_runtime = initialize_mcp_mutation_runtime(
        &data_path,
        &vault_path,
        custom_data,
        vault_identity,
        secret_store,
        twin_event_store,
    )?;
    let mutation_coordinator = mutation_runtime.coordinator.clone();
    log::info!(
        "Sync foundation initialized (provisioned: {})",
        mutation_runtime.sync_engine.status()?.provisioned
    );
    let derived_data_path = mutation_coordinator.current_namespace_path()?;

    // Initialize services
    let mut knowledge_store = KnowledgeStore::with_event_recorder(
        vault_path,
        derived_data_path.clone(),
        mutation_coordinator.clone(),
    );
    let startup_authority =
        prepare_mcp_startup_authority(&data_path, &mutation_runtime, &mut knowledge_store)?;
    let startup_token = startup_authority.startup_token;
    let derived_token = startup_authority.derived_token;
    let derived_ready = derived_token.is_some();

    // A namespace without the matching durable readiness marker may still serve
    // authoritative note CRUD, but must never expose global or stale indexes.
    let search_service = if derived_ready {
        match SearchService::new_readonly(derived_data_path.clone()) {
            Ok(service) => {
                service.reload_reader()?;
                log::info!("Search service initialized in read-only mode");
                Some(service)
            }
            Err(error) => {
                log::error!("Failed to open scoped search index read-only: {}", error);
                None
            }
        }
    } else {
        log::warn!("Vault-derived namespace is not ready; derived MCP tools are unavailable");
        None
    };

    // Graph data is derived and is admitted only with the same ready namespace.
    let mut graph_index = GraphIndex::new();
    if derived_ready {
        let notes_for_graph: Vec<_> = knowledge_store
            .list_notes()
            .unwrap_or_default()
            .iter()
            .filter_map(|metadata| knowledge_store.get_note(&metadata.id).ok())
            .collect();
        graph_index.build_from_notes(&notes_for_graph);
    }
    log::info!(
        "Graph index built: {} notes, {} links",
        graph_index.stats().total_notes,
        graph_index.stats().total_links
    );

    let memory_service = MemoryService::new();
    let priority_service = PriorityScoringService::new(data_path.clone());
    let retrieval_service = RetrievalService::new(data_path.clone());

    // Try to open chunk index (read-only — Tauri app may hold the writer lock)
    let chunk_index = if derived_ready {
        match ChunkIndex::new_readonly(derived_data_path.clone()) {
            Ok(ci) => {
                ci.reload_reader()?;
                log::info!("Chunk index opened in read-only mode");
                Some(Arc::new(RwLock::new(ci)))
            }
            Err(e) => {
                log::warn!(
                "Chunk index not available: {}. search_chunks and chunk recall will be disabled.",
                e
            );
                None
            }
        }
    } else {
        None
    };

    // Create MCP server
    mutation_coordinator.validate_authority_token(&startup_token, false)?;
    if let Some(token) = &derived_token {
        mutation_coordinator.validate_authority_token(token, true)?;
    }
    let server = GrafynMcpServer::new_with_governed_derived_state(
        Arc::new(RwLock::new(knowledge_store)),
        search_service.map(|service| Arc::new(RwLock::new(service))),
        Arc::new(RwLock::new(graph_index)),
        Arc::new(RwLock::new(memory_service)),
        chunk_index,
        Arc::new(RwLock::new(retrieval_service)),
        Arc::new(RwLock::new(priority_service)),
        mutation_coordinator,
        startup_token,
        derived_token,
    )?;

    log::info!("Starting Grafyn MCP server on stdio...");

    // Serve over stdio (Claude Desktop communicates via stdin/stdout)
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| log::error!("MCP server error: {}", e))?;

    // Block until the client disconnects
    service.waiting().await?;

    log::info!("MCP server shutting down.");
    Ok(())
}

/// Resolve vault and data paths from CLI args, settings file, or defaults.
struct ResolvedMcpPaths {
    vault_path: PathBuf,
    data_path: PathBuf,
    custom_data: bool,
    secret_store: Option<Arc<dyn crate::services::sync::secrets::SecretStore>>,
}

fn resolve_paths(
    cli_vault: Option<PathBuf>,
    cli_data: Option<PathBuf>,
) -> Result<ResolvedMcpPaths, Box<dyn std::error::Error>> {
    if let Some((vault_path, data_path)) =
        validate_cli_override_pair(cli_vault.as_deref(), cli_data.as_deref())?
    {
        let vault_path = vault_path.to_path_buf();
        let data_path = data_path.to_path_buf();
        return Ok(ResolvedMcpPaths {
            vault_path,
            data_path,
            custom_data: true,
            secret_store: None,
        });
    }
    // The default authority performs global settings/key/WAL recovery exactly once.
    let settings = SettingsService::load()?;

    let vault_path = settings.vault_path();
    let data_path = settings.data_path();
    let secret_store = settings.secret_store();

    Ok(ResolvedMcpPaths {
        vault_path,
        data_path,
        custom_data: false,
        secret_store: Some(secret_store),
    })
}

fn validate_cli_override_pair<'a>(
    vault: Option<&'a std::path::Path>,
    data: Option<&'a std::path::Path>,
) -> Result<Option<(&'a std::path::Path, &'a std::path::Path)>, &'static str> {
    match (vault, data) {
        (None, None) => Ok(None),
        (Some(vault), Some(data)) => Ok(Some((vault, data))),
        (Some(_), None) => Err("--vault requires --data for isolated MCP mode"),
        (None, Some(_)) => Err("--data requires --vault for isolated MCP mode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twin_events::{CustomMcpRootBindingV1, CUSTOM_MCP_ROOT_BINDING_KEY};

    fn tree_snapshot(
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .map(Result::unwrap)
            .map(|entry| {
                let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
                let contents = entry
                    .file_type()
                    .is_file()
                    .then(|| std::fs::read(entry.path()).unwrap());
                (relative, contents)
            })
            .collect()
    }

    #[test]
    fn custom_data_requires_an_explicit_vault_and_custom_start_rejects_transition_wal() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom-data");
        let vault = temp.path().join("custom-vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let missing = resolve_paths(None, Some(data.clone()))
            .err()
            .expect("missing vault override must fail");
        assert!(missing.to_string().contains("requires --vault"));

        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        std::fs::write(
            data.join("twin/events/root-transition-v1.json"),
            b"foreign authority",
        )
        .unwrap();
        std::fs::write(data.join("twin/events/mutation-v1.lock"), b"").unwrap();
        let data_before = walkdir::WalkDir::new(&data)
            .into_iter()
            .map(Result::unwrap)
            .map(|entry| {
                let relative = entry.path().strip_prefix(&data).unwrap().to_path_buf();
                let contents = entry
                    .file_type()
                    .is_file()
                    .then(|| std::fs::read(entry.path()).unwrap());
                (relative, contents)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let paths = resolve_paths(Some(vault), Some(data.clone())).unwrap();
        let prepare_error = prepare_mcp_root_directories(&paths.vault_path, &data)
            .expect_err("MCP root preparation must reject a transition WAL before identity writes");
        assert!(prepare_error.to_string().contains("root-transition"));
        assert!(!paths
            .vault_path
            .join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)
            .exists());
        assert!(!data.join(CUSTOM_MCP_ROOT_BINDING_KEY).exists());
        let data_after = walkdir::WalkDir::new(&data)
            .into_iter()
            .map(Result::unwrap)
            .map(|entry| {
                let relative = entry.path().strip_prefix(&data).unwrap().to_path_buf();
                let contents = entry
                    .file_type()
                    .is_file()
                    .then(|| std::fs::read(entry.path()).unwrap());
                (relative, contents)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(data_after, data_before);
        let events = Arc::new(TwinEventStore::new(&data));
        let error = match MutationCoordinator::new_custom_mcp(
            &data,
            &paths.vault_path,
            events,
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("custom start must reject a transition WAL"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("root-transition"));
    }

    #[test]
    fn cli_overrides_are_an_inseparable_vault_and_data_pair() {
        let vault = PathBuf::from("isolated-vault");
        let data = PathBuf::from("isolated-data");

        assert!(validate_cli_override_pair(Some(&vault), None).is_err());
        assert!(validate_cli_override_pair(None, Some(&data)).is_err());
        assert!(validate_cli_override_pair(None, None).unwrap().is_none());
        assert_eq!(
            validate_cli_override_pair(Some(&vault), Some(&data)).unwrap(),
            Some((vault.as_path(), data.as_path()))
        );
    }

    #[test]
    fn custom_data_mode_has_no_access_to_the_global_secret_store() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();

        let paths = resolve_paths(Some(vault), Some(data)).unwrap();

        assert!(paths.secret_store.is_none());
    }

    #[test]
    fn fresh_custom_sync_lifecycle_fails_closed_before_binding() {
        let lifecycle = FreshCustomSyncLifecycle::default();

        let error = lifecycle
            .known_failure(None, "startup recovery probe")
            .unwrap_err();

        assert!(matches!(error, MutationError::RecoveryConflict(_)));
        assert!(error.to_string().contains("not bound"));
    }

    #[test]
    fn custom_mcp_runtime_uses_an_explicit_unprovisioned_sync_engine() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("data");
        let identity = prepare_mcp_root_directories(&vault, &data).unwrap();
        let runtime = initialize_mcp_mutation_runtime(
            &data,
            &vault,
            true,
            identity,
            None,
            Arc::new(TwinEventStore::new(&data)),
        )
        .unwrap();

        assert!(!runtime.sync_engine.status().unwrap().provisioned);
        let _ = runtime
            .coordinator
            .commit_local(
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("mcp").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "custom-unprovisioned.md",
                    "local only until a vault key is provisioned",
                )],
                Vec::new(),
            )
            .unwrap();
        let status = runtime.sync_engine.status().unwrap();
        assert!(!status.provisioned);
        assert_eq!(status.outbox_operations, 0);
        let mut knowledge = KnowledgeStore::with_event_recorder(
            vault,
            runtime.coordinator.current_namespace_path().unwrap(),
            runtime.coordinator.clone(),
        );
        let startup = prepare_mcp_startup_authority(&data, &runtime, &mut knowledge).unwrap();
        assert_eq!(runtime.sync_engine.status().unwrap().outbox_operations, 0);
        assert_eq!(
            startup.startup_token,
            runtime.coordinator.current_authority_token().unwrap()
        );
    }

    #[test]
    fn provisioned_mcp_startup_recovers_a_durable_remote_inbox_before_bootstrap() {
        use grafyn_sync_protocol::{
            seal_operation, DeviceId, DeviceSigningKey, NoteRevisionV1, OperationPayloadV1,
            OperationV1, VaultRootKey,
        };

        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
        let secrets = Arc::new(crate::services::sync::secrets::MemorySecretStore::default());
        let root_key = VaultRootKey::from_bytes([0xd4; 32]);
        crate::services::sync::vault_keys::provision_vault_root_key(
            secrets.as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &root_key,
        )
        .unwrap();
        std::fs::write(vault.join("existing.md"), b"existing MCP bytes  \r\n").unwrap();
        let runtime = initialize_mcp_mutation_runtime(
            &data,
            &vault,
            false,
            Some(identity.clone()),
            Some(secrets),
            Arc::new(TwinEventStore::new(&data)),
        )
        .unwrap();
        let remote_id = DeviceId::parse_str("123e4567-e89b-42d3-a456-4266141740d4").unwrap();
        let remote_key = DeviceSigningKey::from_seed([0xd4; 32]);
        runtime
            .sync_engine
            .trust_device(remote_id, remote_key.public_key())
            .unwrap();
        let remote_note_id = "remote-md";
        let remote_markdown = "---\nnote_id: remote-md\n---\n\nremote startup recovery term\r\n";
        let operation = OperationV1::new(
            1_800_000_000_000,
            Vec::new(),
            OperationPayloadV1::NoteRevision(
                NoteRevisionV1::put(remote_note_id, remote_markdown.into()).unwrap(),
            ),
        )
        .unwrap();
        let envelope = seal_operation(
            &root_key,
            identity.descriptor.vault_id(),
            &remote_id,
            &remote_key,
            &operation,
        )
        .unwrap();
        crate::services::sync::operation_store::OperationStore::open(
            &data,
            identity.root_scope,
            *identity.descriptor.vault_id(),
        )
        .unwrap()
        .receive(&envelope)
        .unwrap();
        let projected_key = format!(
            "synced/{}.md",
            crate::services::twin_events::digest_bytes(
                format!("grafyn.sync.remote-path.v1:{remote_note_id}").as_bytes()
            )
            .as_str()
        );
        let derived_data_path = runtime.coordinator.current_namespace_path().unwrap();
        let mut knowledge = KnowledgeStore::with_event_recorder(
            vault.clone(),
            derived_data_path.clone(),
            runtime.coordinator.clone(),
        );

        let startup = prepare_mcp_startup_authority(&data, &runtime, &mut knowledge).unwrap();

        assert_eq!(runtime.sync_engine.status().unwrap().outbox_operations, 1);
        assert_eq!(runtime.sync_engine.status().unwrap().pending_operations, 0);
        assert_eq!(
            std::fs::read(vault.join("existing.md")).unwrap(),
            b"existing MCP bytes  \r\n"
        );
        assert_eq!(
            std::fs::read(vault.join(projected_key)).unwrap(),
            remote_markdown.as_bytes()
        );
        let derived = startup
            .derived_token
            .expect("remote recovery must publish its exact derived authority");
        assert_eq!(derived, startup.startup_token);
        runtime
            .coordinator
            .validate_authority_token(&derived, true)
            .unwrap();
        let readonly = SearchService::new_readonly(derived_data_path).unwrap();
        readonly.reload_reader().unwrap();
        let results = readonly.search("recovery term", 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note.id, remote_note_id);
    }

    #[test]
    fn mcp_restart_repairs_missing_ready_after_exact_remote_replay_without_outbox_echo() {
        use grafyn_sync_protocol::{
            seal_operation, DeviceId, DeviceSigningKey, NoteRevisionV1, OperationPayloadV1,
            OperationV1, VaultRootKey,
        };

        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
        let secrets = Arc::new(crate::services::sync::secrets::MemorySecretStore::default());
        let root_key = VaultRootKey::from_bytes([0xe6; 32]);
        crate::services::sync::vault_keys::provision_vault_root_key(
            secrets.as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &root_key,
        )
        .unwrap();
        let runtime = initialize_mcp_mutation_runtime(
            &data,
            &vault,
            false,
            Some(identity.clone()),
            Some(secrets.clone()),
            Arc::new(TwinEventStore::new(&data)),
        )
        .unwrap();
        let remote_id = DeviceId::parse_str("123e4567-e89b-42d3-a456-4266141740e6").unwrap();
        let remote_key = DeviceSigningKey::from_seed([0xe6; 32]);
        runtime
            .sync_engine
            .trust_device(remote_id, remote_key.public_key())
            .unwrap();
        let note_id = "mcp-crash-exact";
        let markdown = "---\nnote_id: mcp-crash-exact\n---\n\nMCP exact replay recovery term\n";
        let operation = OperationV1::new(
            1_800_000_000_001,
            Vec::new(),
            OperationPayloadV1::NoteRevision(
                NoteRevisionV1::put(note_id, markdown.into()).unwrap(),
            ),
        )
        .unwrap();
        let envelope = seal_operation(
            &root_key,
            identity.descriptor.vault_id(),
            &remote_id,
            &remote_key,
            &operation,
        )
        .unwrap();
        let derived_data_path = runtime.coordinator.current_namespace_path().unwrap();
        let mut knowledge = KnowledgeStore::with_event_recorder(
            vault.clone(),
            derived_data_path.clone(),
            runtime.coordinator.clone(),
        );
        let initially_ready = runtime.coordinator.current_authority_token().unwrap();
        rebuild_mcp_remote_authority(
            &data,
            &runtime.coordinator,
            &mut knowledge,
            &initially_ready,
        )
        .unwrap();
        runtime
            .coordinator
            .validate_authority_token(&initially_ready, true)
            .unwrap();
        let projected_key = format!(
            "synced/{}.md",
            crate::services::twin_events::digest_bytes(
                format!("grafyn.sync.remote-path.v1:{note_id}").as_bytes()
            )
            .as_str()
        );
        runtime
            .coordinator
            .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance);
        assert!(runtime
            .sync_engine
            .receive_envelopes(
                &runtime.coordinator,
                &[envelope.to_json().unwrap().into_bytes()],
            )
            .is_err());
        let committed_authority = runtime.coordinator.current_authority_token().unwrap();
        assert!(runtime
            .coordinator
            .validate_authority_token(&committed_authority, true)
            .is_err());
        assert!(!vault.join(&projected_key).exists());
        assert_eq!(runtime.sync_engine.status().unwrap().pending_operations, 1);
        assert_eq!(runtime.sync_engine.status().unwrap().outbox_operations, 0);
        drop(knowledge);
        drop(runtime);

        let restarted = initialize_mcp_mutation_runtime(
            &data,
            &vault,
            false,
            Some(identity),
            Some(secrets),
            Arc::new(TwinEventStore::new(&data)),
        )
        .unwrap();
        assert_eq!(
            restarted.sync_engine.status().unwrap().pending_operations,
            1
        );
        assert_eq!(restarted.sync_engine.status().unwrap().outbox_operations, 0);
        // Test coordinators publish readiness during construction. Clear that
        // test-only marker to reproduce the production crash state on restart.
        let crash_authority = restarted.coordinator.current_authority_token().unwrap();
        assert_eq!(crash_authority, committed_authority);
        restarted
            .coordinator
            .invalidate_namespace_before_recovery(&crash_authority)
            .unwrap();
        assert!(restarted
            .coordinator
            .validate_authority_token(&crash_authority, true)
            .is_err());
        let mut restarted_knowledge = KnowledgeStore::with_event_recorder(
            vault.clone(),
            restarted.coordinator.current_namespace_path().unwrap(),
            restarted.coordinator.clone(),
        );

        let startup =
            prepare_mcp_startup_authority(&data, &restarted, &mut restarted_knowledge).unwrap();

        assert_eq!(
            restarted.sync_engine.status().unwrap().pending_operations,
            0
        );
        assert_eq!(restarted.sync_engine.status().unwrap().outbox_operations, 0);
        assert_eq!(startup.startup_token, crash_authority);
        let derived = startup
            .derived_token
            .expect("exact remote replay must still rebuild and publish current readiness");
        assert_eq!(derived, startup.startup_token);
        restarted
            .coordinator
            .validate_authority_token(&derived, true)
            .unwrap();
        assert_eq!(
            std::fs::read(vault.join(projected_key)).unwrap(),
            markdown.as_bytes()
        );
        let readonly = SearchService::new_readonly(derived_data_path).unwrap();
        readonly.reload_reader().unwrap();
        let results = readonly.search("exact replay recovery", 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note.id, note_id);
    }

    #[test]
    fn custom_root_preflight_creates_a_fresh_pair_only_without_a_stable_lease() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");

        prepare_mcp_root_directories(&vault, &data).unwrap();

        let coordinator = MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let identity = crate::services::sync::identity::load_vault_identity(&vault).unwrap();
        let binding: CustomMcpRootBindingV1 =
            serde_json::from_slice(&std::fs::read(data.join(CUSTOM_MCP_ROOT_BINDING_KEY)).unwrap())
                .unwrap();

        assert!(data.is_dir());
        assert!(vault.is_dir());
        assert_eq!(binding.schema_version, 1);
        assert_eq!(binding.root_scope, identity.root_scope);
        assert_eq!(
            binding.canonical_vault_path,
            std::fs::canonicalize(&vault).unwrap().to_str().unwrap()
        );
        assert_eq!(
            coordinator.current_root_epoch().unwrap().root_scope,
            identity.root_scope
        );
        let descriptor_before =
            std::fs::read(vault.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY))
                .unwrap();
        let binding_before = std::fs::read(data.join(CUSTOM_MCP_ROOT_BINDING_KEY)).unwrap();
        let lease_before =
            std::fs::read(data.join("twin/events/active-markdown-root-v1.json")).unwrap();
        drop(coordinator);

        let reopened = MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(vault.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY))
                .unwrap(),
            descriptor_before
        );
        assert_eq!(
            std::fs::read(data.join(CUSTOM_MCP_ROOT_BINDING_KEY)).unwrap(),
            binding_before
        );
        assert_eq!(
            std::fs::read(data.join("twin/events/active-markdown-root-v1.json")).unwrap(),
            lease_before
        );
        assert_eq!(
            reopened.current_root_epoch().unwrap().root_scope,
            identity.root_scope
        );
    }

    #[test]
    fn desktop_and_custom_mcp_reopen_one_scope_writer_device_and_event_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        prepare_mcp_root_directories(&vault, &data).unwrap();

        let custom_events = Arc::new(TwinEventStore::new(&data));
        let custom = MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            custom_events.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let scope = custom.current_root_epoch().unwrap().root_scope;
        let writer = custom.writer_device_id();
        let expected_events = data
            .join("twin/events/vaults/v1")
            .join(scope.as_str())
            .join("records/v1");
        assert_eq!(
            std::fs::canonicalize(custom_events.events_dir()).unwrap(),
            std::fs::canonicalize(&expected_events).unwrap()
        );
        assert!(resolve_paths(Some(vault.clone()), Some(data.clone()))
            .unwrap()
            .secret_store
            .is_none());
        drop(custom);

        let desktop_events = Arc::new(TwinEventStore::new(&data));
        let desktop = MutationCoordinator::new_stable(
            &data,
            &vault,
            desktop_events.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        assert_eq!(desktop.current_root_epoch().unwrap().root_scope, scope);
        assert_eq!(desktop.writer_device_id(), writer);
        assert_eq!(
            std::fs::canonicalize(desktop_events.events_dir()).unwrap(),
            std::fs::canonicalize(&expected_events).unwrap()
        );
        let signing = desktop
            .load_or_create_device_signing_identity(Arc::new(
                crate::services::sync::secrets::MemorySecretStore::default(),
            ))
            .unwrap();
        assert_eq!(signing.device_id().to_string(), writer.as_str());
        drop(desktop);

        let reopened_events = Arc::new(TwinEventStore::new(&data));
        let reopened = MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            reopened_events.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        assert_eq!(reopened.current_root_epoch().unwrap().root_scope, scope);
        assert_eq!(reopened.writer_device_id(), writer);
        assert_eq!(
            std::fs::canonicalize(reopened_events.events_dir()).unwrap(),
            std::fs::canonicalize(expected_events).unwrap()
        );
        assert!(resolve_paths(Some(vault), Some(data))
            .unwrap()
            .secret_store
            .is_none());
    }

    #[test]
    fn custom_start_rejects_b_before_binding_no_lease_legacy_data_owned_by_a() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault_a = temp.path().join("vault-a");
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault_a).unwrap();
        std::fs::create_dir(&vault_b).unwrap();
        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        let legacy_scope_a =
            crate::services::sync::identity::legacy_path_scope_for_migration(&vault_a).unwrap();
        let legacy_twin = data.join("twin").join(legacy_scope_a.as_str());
        std::fs::create_dir(&legacy_twin).unwrap();
        std::fs::write(legacy_twin.join("owned-by-a.json"), b"vault-a").unwrap();
        std::fs::create_dir(data.join("search_index")).unwrap();
        std::fs::write(data.join("search_index/sentinel"), b"vault-a").unwrap();
        crate::services::twin_events::acquire_shared_coordinator_process_lock(&data)
            .unwrap()
            .unlock()
            .unwrap();
        let data_before = tree_snapshot(&data);
        let vault_a_before = tree_snapshot(&vault_a);
        let vault_b_before = tree_snapshot(&vault_b);

        prepare_mcp_root_directories(&vault_b, &data).unwrap();
        let error = match MutationCoordinator::new_custom_mcp(
            &data,
            &vault_b,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("vault B must not adopt vault A's no-lease legacy data"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("legacy"));
        assert_eq!(tree_snapshot(&data), data_before);
        assert_eq!(tree_snapshot(&vault_a), vault_a_before);
        assert_eq!(tree_snapshot(&vault_b), vault_b_before);
        assert!(!vault_b
            .join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)
            .exists());
        assert!(!data.join(CUSTOM_MCP_ROOT_BINDING_KEY).exists());
        assert!(!data
            .join("twin/events/active-markdown-root-v1.json")
            .exists());

        prepare_mcp_root_directories(&vault_a, &data).unwrap();
        let coordinator_a = MutationCoordinator::new_custom_mcp(
            &data,
            &vault_a,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let identity_a = crate::services::sync::identity::load_vault_identity(&vault_a).unwrap();
        let binding: CustomMcpRootBindingV1 =
            serde_json::from_slice(&std::fs::read(data.join(CUSTOM_MCP_ROOT_BINDING_KEY)).unwrap())
                .unwrap();
        assert_eq!(binding.root_scope, identity_a.root_scope);
        assert_eq!(
            coordinator_a.current_root_epoch().unwrap().root_scope,
            identity_a.root_scope
        );
    }

    #[test]
    fn blocked_legacy_custom_start_publishes_no_binding_or_other_bytes() {
        for blocker in ["retained-receipt", "pending-optimizer-publication"] {
            let temp = tempfile::tempdir().unwrap();
            let data = temp.path().join("data");
            let vault = temp.path().join("vault");
            std::fs::create_dir(&data).unwrap();
            std::fs::create_dir(&vault).unwrap();
            std::fs::create_dir_all(data.join("twin/events")).unwrap();
            let legacy = MutationCoordinator::new(
                &data,
                &vault,
                Arc::new(TwinEventStore::new(&data)),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap();
            let guard = legacy.begin_root_transition().unwrap();
            let lease = guard.current_lease().unwrap();
            let twin_path = guard.prepare_twin_data_path(&vault, &lease).unwrap();
            std::fs::create_dir_all(&twin_path).unwrap();
            std::fs::write(twin_path.join("legacy-source.json"), b"legacy-source").unwrap();
            guard.initialize_namespace(&lease).unwrap();
            drop(guard);
            drop(legacy);
            let identity =
                crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();

            let blocker_path = match blocker {
                "retained-receipt" => data
                    .join("twin/mutations/receipts/v1")
                    .join("retained.json"),
                "pending-optimizer-publication" => {
                    crate::services::vault_namespace::scoped_data_path(&data, &lease.root_scope)
                        .join("vault_migration/optimizer/pending-publications-v1/pending.json")
                }
                _ => unreachable!(),
            };
            std::fs::create_dir_all(blocker_path.parent().unwrap()).unwrap();
            std::fs::write(&blocker_path, blocker.as_bytes()).unwrap();
            prepare_mcp_root_directories(&vault, &data).unwrap();
            let descriptor_path = vault.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY);
            let descriptor_before = std::fs::read(&descriptor_path).unwrap();
            let lease_path = data.join("twin/events/active-markdown-root-v1.json");
            let lease_before = std::fs::read(&lease_path).unwrap();
            let blocker_before = std::fs::read(&blocker_path).unwrap();
            let source_before = std::fs::read(twin_path.join("legacy-source.json")).unwrap();
            let data_before = tree_snapshot(&data);
            let vault_before = tree_snapshot(&vault);

            let error = match MutationCoordinator::new_custom_mcp(
                &data,
                &vault,
                Arc::new(TwinEventStore::new(&data)),
                Arc::new(NoopMutationLifecycle),
            ) {
                Ok(_) => panic!("{blocker} must block stable custom migration"),
                Err(error) => error,
            };

            assert!(
                error.to_string().contains("stable migration"),
                "unexpected {blocker} error: {error}"
            );
            assert_eq!(std::fs::read(&descriptor_path).unwrap(), descriptor_before);
            assert_eq!(std::fs::read(&lease_path).unwrap(), lease_before);
            assert_eq!(std::fs::read(&blocker_path).unwrap(), blocker_before);
            assert_eq!(
                std::fs::read(twin_path.join("legacy-source.json")).unwrap(),
                source_before
            );
            assert!(!data.join(CUSTOM_MCP_ROOT_BINDING_KEY).exists());
            assert_eq!(tree_snapshot(&data), data_before);
            assert_eq!(tree_snapshot(&vault), vault_before);
            assert_eq!(
                identity.root_scope,
                crate::services::sync::identity::load_vault_identity(&vault)
                    .unwrap()
                    .root_scope
            );
        }
    }

    #[test]
    fn stable_root_preflight_never_manufactures_a_missing_vault_or_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let original = temp.path().join("original-vault");
        let empty_replacement = temp.path().join("empty-replacement");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&empty_replacement).unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&original).unwrap();
        let store = crate::services::root_transition::RootTransitionStore::new(
            &data,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope,
                ),
            )
            .unwrap();
        std::fs::remove_dir_all(&original).unwrap();

        assert!(prepare_mcp_root_directories(&original, &data).is_err());
        assert!(!original.exists());
        assert!(prepare_mcp_root_directories(&empty_replacement, &data).is_err());
        assert!(!empty_replacement
            .join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)
            .exists());
    }

    #[test]
    fn stable_custom_data_without_a_binding_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        let legacy = MutationCoordinator::new(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        drop(legacy);
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
        let store = crate::services::root_transition::RootTransitionStore::new(
            &data,
            temp.path().join("settings.json"),
            Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
        )
        .unwrap();
        store
            .write_lease(
                &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                    identity.root_scope,
                ),
            )
            .unwrap();
        prepare_mcp_root_directories(&vault, &data).unwrap();
        let data_before = tree_snapshot(&data);
        let vault_before = tree_snapshot(&vault);

        let error = match MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("stable custom data without its path binding must fail closed"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("no durable vault-path binding"));
        assert_eq!(tree_snapshot(&data), data_before);
        assert_eq!(tree_snapshot(&vault), vault_before);
        assert!(!data.join(CUSTOM_MCP_ROOT_BINDING_KEY).exists());
    }

    #[test]
    fn custom_root_binding_rejects_a_same_uuid_copy_at_another_path() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let first = temp.path().join("first-vault");
        let copied = temp.path().join("copied-vault");
        prepare_mcp_root_directories(&first, &data).unwrap();
        let first_coordinator = MutationCoordinator::new_custom_mcp(
            &data,
            &first,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let identity = crate::services::sync::identity::load_vault_identity(&first).unwrap();
        std::fs::create_dir(&copied).unwrap();
        std::fs::create_dir(copied.join("_grafyn")).unwrap();
        std::fs::copy(
            first.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY),
            copied.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY),
        )
        .unwrap();
        assert_eq!(
            first_coordinator.current_root_epoch().unwrap().root_scope,
            identity.root_scope
        );
        drop(first_coordinator);
        let binding_path = data.join(CUSTOM_MCP_ROOT_BINDING_KEY);
        let binding_before = std::fs::read(&binding_path).unwrap();
        let data_before = tree_snapshot(&data);

        prepare_mcp_root_directories(&first, &data).unwrap();
        prepare_mcp_root_directories(&copied, &data).unwrap();
        let error = match MutationCoordinator::new_custom_mcp(
            &data,
            &copied,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("a same-UUID copy at another path must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("binding"));
        assert_eq!(std::fs::read(binding_path).unwrap(), binding_before);
        assert_eq!(tree_snapshot(&data), data_before);
    }

    #[test]
    fn established_custom_binding_never_recreates_a_missing_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        prepare_mcp_root_directories(&vault, &data).unwrap();
        let coordinator = MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        drop(coordinator);
        let descriptor = vault.join(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY);
        std::fs::remove_file(&descriptor).unwrap();
        let binding_path = data.join(CUSTOM_MCP_ROOT_BINDING_KEY);
        let binding_before = std::fs::read(&binding_path).unwrap();

        let error = match MutationCoordinator::new_custom_mcp(
            &data,
            &vault,
            Arc::new(TwinEventStore::new(&data)),
            Arc::new(NoopMutationLifecycle),
        ) {
            Ok(_) => panic!("an established binding requires its existing descriptor"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("vault-descriptor-missing"));
        assert!(!descriptor.exists());
        assert_eq!(std::fs::read(binding_path).unwrap(), binding_before);
    }

    #[test]
    fn custom_start_retains_process_lock_from_wal_check_through_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = Arc::new(TwinEventStore::new(&data));
        events.initialize().unwrap();
        let (checked_tx, checked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let data_for_start = data.clone();
        let vault_for_start = vault.clone();
        let start = std::thread::spawn(move || {
            MutationCoordinator::new_custom_mcp_with_hook(
                &data_for_start,
                &vault_for_start,
                events,
                Arc::new(NoopMutationLifecycle),
                move || {
                    checked_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
            .unwrap();
        });
        checked_rx.recv().unwrap();
        let (peer_tx, peer_rx) = std::sync::mpsc::channel();
        let data_for_peer = data.clone();
        let peer = std::thread::spawn(move || {
            let lock = crate::services::twin_events::acquire_shared_coordinator_process_lock(
                &data_for_peer,
            )
            .unwrap();
            peer_tx.send(()).unwrap();
            lock.unlock().unwrap();
        });

        assert!(peer_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        release_tx.send(()).unwrap();
        start.join().unwrap();
        peer_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        peer.join().unwrap();
    }
}
