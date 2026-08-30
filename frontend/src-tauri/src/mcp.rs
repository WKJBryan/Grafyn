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
    if paths.custom_data {
        crate::services::root_transition::reject_custom_transition_wal(&data_path)?;
    }

    // Recover the same canonical mutation journal as desktop before stdio is served.
    let twin_event_store = Arc::new(TwinEventStore::new(&data_path));
    twin_event_store.initialize()?;
    let mutation_coordinator = Arc::new(MutationCoordinator::new(
        &data_path,
        &vault_path,
        twin_event_store,
        Arc::new(NoopMutationLifecycle),
    )?);
    mutation_coordinator.recover_pending()?;
    let derived_data_path = mutation_coordinator.current_namespace_path()?;
    let derived_ready = mutation_coordinator.require_namespace_ready().is_ok();

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
    let server = GrafynMcpServer::new_with_governed_derived_state(
        Arc::new(RwLock::new(knowledge_store)),
        search_service.map(|service| Arc::new(RwLock::new(service))),
        Arc::new(RwLock::new(graph_index)),
        Arc::new(RwLock::new(memory_service)),
        chunk_index,
        Arc::new(RwLock::new(retrieval_service)),
        Arc::new(RwLock::new(priority_service)),
        mutation_coordinator,
        derived_ready,
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
    if let Some(data_path) = cli_data {
        let vault_path = cli_vault.ok_or(
            "--data defines an isolated MCP authority and therefore requires --vault",
        )?;
        crate::services::root_transition::reject_custom_transition_wal(&data_path)?;
        return Ok(ResolvedMcpPaths {
            vault_path,
            data_path,
            custom_data: true,
        });
    }
    // The default authority performs global settings/key/WAL recovery exactly once.
    let settings = SettingsService::load()?;

    let vault_path = cli_vault
        .or_else(|| Some(settings.vault_path()))
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Grafyn")
                .join("vault")
        });

    Ok(ResolvedMcpPaths {
        vault_path,
        data_path: settings.data_path(),
        custom_data: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_data_requires_an_explicit_vault_and_rejects_transition_wal() {
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
        let error = resolve_paths(Some(vault), Some(data)).unwrap_err();
        assert!(error.to_string().contains("root-transition"));
    }
}
