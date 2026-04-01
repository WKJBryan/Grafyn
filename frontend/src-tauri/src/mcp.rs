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

mod models;
mod services;
mod mcp_tools;

use crate::mcp_tools::GrafynMcpServer;
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::memory::MemoryService;
use crate::services::priority::PriorityScoringService;
use crate::services::retrieval::RetrievalService;
use crate::services::search::SearchService;
use crate::services::settings::SettingsService;
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
    let (vault_path, data_path) = resolve_paths(args.vault, args.data);

    log::info!("Vault path: {}", vault_path.display());
    log::info!("Data path: {}", data_path.display());

    // Ensure directories exist
    std::fs::create_dir_all(&vault_path)?;
    std::fs::create_dir_all(&data_path)?;

    // Initialize services
    let knowledge_store = KnowledgeStore::new(vault_path);

    // Try full SearchService first; fall back to read-only if writer lock is held
    let search_service = match SearchService::new(data_path.clone()) {
        Ok(s) => {
            log::info!("Search service initialized with write access");
            s
        }
        Err(e) => {
            log::warn!(
                "Could not acquire search writer (Grafyn app may be running): {}. \
                 Falling back to read-only search.",
                e
            );
            match SearchService::new_readonly(data_path.clone()) {
                Ok(s) => {
                    log::info!("Search service initialized in read-only mode");
                    s
                }
                Err(e2) => {
                    log::error!("Failed to open search index: {}", e2);
                    log::info!("Starting without search — create/update will skip indexing");
                    // Create a minimal writable service as last resort
                    SearchService::new(data_path.clone())?
                }
            }
        }
    };

    // Build graph index from notes
    let mut graph_index = GraphIndex::new();
    let notes_for_graph: Vec<_> = knowledge_store
        .list_notes()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| knowledge_store.get_note(&m.id).ok())
        .collect();
    graph_index.build_from_notes(&notes_for_graph);
    log::info!(
        "Graph index built: {} notes, {} links",
        graph_index.stats().total_notes,
        graph_index.stats().total_links
    );

    let memory_service = MemoryService::new();
    let priority_service = PriorityScoringService::new(data_path.clone());
    let retrieval_service = RetrievalService::new(data_path.clone());

    // Try to open chunk index (read-only — Tauri app may hold the writer lock)
    let chunk_index = match ChunkIndex::new_readonly(data_path.clone()) {
        Ok(ci) => {
            log::info!("Chunk index opened in read-only mode");
            Some(Arc::new(RwLock::new(ci)))
        }
        Err(e) => {
            log::warn!("Chunk index not available: {}. search_chunks and chunk recall will be disabled.", e);
            None
        }
    };

    // Create MCP server
    let server = GrafynMcpServer::new(
        Arc::new(RwLock::new(knowledge_store)),
        Arc::new(RwLock::new(search_service)),
        Arc::new(RwLock::new(graph_index)),
        Arc::new(RwLock::new(memory_service)),
        chunk_index,
        Arc::new(RwLock::new(retrieval_service)),
        Arc::new(RwLock::new(priority_service)),
    );

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
fn resolve_paths(cli_vault: Option<PathBuf>, cli_data: Option<PathBuf>) -> (PathBuf, PathBuf) {
    // Try loading from settings if CLI args aren't provided
    let settings = if cli_vault.is_none() || cli_data.is_none() {
        SettingsService::load().ok()
    } else {
        None
    };

    let vault_path = cli_vault
        .or_else(|| settings.as_ref().map(|s| s.vault_path()))
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Grafyn")
                .join("vault")
        });

    let data_path = cli_data
        .or_else(|| settings.as_ref().map(|s| s.data_path()))
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| {
                    dirs::document_dir().unwrap_or_else(|| PathBuf::from("."))
                })
                .join("Grafyn")
                .join("data")
        });

    (vault_path, data_path)
}
