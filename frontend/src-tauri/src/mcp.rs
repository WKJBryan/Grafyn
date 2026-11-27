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

mod headless;
mod mcp_tools;
mod models;
mod services;

use crate::headless::Headless;
use crate::mcp_tools::GrafynMcpServer;
use crate::services::settings::SettingsService;
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;

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
    let core = Headless::open(vault_path, data_path)?;
    let server = GrafynMcpServer::new(core);

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
                .unwrap_or_else(|| dirs::document_dir().unwrap_or_else(|| PathBuf::from(".")))
                .join("Grafyn")
                .join("data")
        });

    (vault_path, data_path)
}
