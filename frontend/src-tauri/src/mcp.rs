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
use crate::services::memory::MemoryService;
use crate::services::priority::PriorityScoringService;
use crate::services::retrieval::RetrievalService;
use crate::services::search::SearchService;
use crate::services::settings::SettingsService;
use crate::services::twin_events::{MutationCoordinator, NoopMutationLifecycle, TwinEventStore};
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::sync::Arc;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to stderr (stdout is reserved for MCP protocol)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let args = Args::parse();

    // Resolve paths: CLI args > settings.json > defaults
    let paths = resolve_paths(args.vault, args.data)?;
    let vault_path = paths.vault_path;
    let data_path = paths.data_path;

    log::info!("Vault path: {}", vault_path.display());
    log::info!("Data path: {}", data_path.display());

    // Ensure directories exist
    std::fs::create_dir_all(&vault_path)?;
    std::fs::create_dir_all(&data_path)?;
    // Recover the same canonical mutation journal as desktop before stdio is served.
    let twin_event_store = Arc::new(TwinEventStore::new(&data_path));
    twin_event_store.initialize()?;
    let mutation_coordinator = Arc::new(if paths.custom_data {
        MutationCoordinator::new_custom_mcp(
            &data_path,
            &vault_path,
            twin_event_store,
            Arc::new(NoopMutationLifecycle),
        )?
    } else {
        MutationCoordinator::new(
            &data_path,
            &vault_path,
            twin_event_store,
            Arc::new(NoopMutationLifecycle),
        )?
    });
    mutation_coordinator.recover_pending()?;
    let startup_token = mutation_coordinator.current_authority_token()?;
    let derived_data_path = mutation_coordinator.current_namespace_path()?;
    let derived_token = mutation_coordinator
        .validate_authority_token(&startup_token, true)
        .is_ok()
        .then(|| startup_token.clone());
    let derived_ready = derived_token.is_some();

    // Initialize services
    let knowledge_store = KnowledgeStore::with_event_recorder(
        vault_path,
        derived_data_path.clone(),
        mutation_coordinator.clone(),
    );

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
#[derive(Debug)]
struct ResolvedMcpPaths {
    vault_path: PathBuf,
    data_path: PathBuf,
    custom_data: bool,
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
        });
    }
    // The default authority performs global settings/key/WAL recovery exactly once.
    let settings = SettingsService::load()?;

    let vault_path = settings.vault_path();

    Ok(ResolvedMcpPaths {
        vault_path,
        data_path: settings.data_path(),
        custom_data: false,
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

    #[test]
    fn custom_data_requires_an_explicit_vault_and_custom_start_rejects_transition_wal() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom-data");
        let vault = temp.path().join("custom-vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let missing = resolve_paths(None, Some(data.clone())).unwrap_err();
        assert!(missing.to_string().contains("requires --vault"));

        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        std::fs::write(
            data.join("twin/events/root-transition-v1.json"),
            b"foreign authority",
        )
        .unwrap();
        let paths = resolve_paths(Some(vault), Some(data.clone())).unwrap();
        let events = Arc::new(TwinEventStore::new(&data));
        events.initialize().unwrap();
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
        peer_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        peer.join().unwrap();
    }
}
