//! MCP Configuration Commands
//!
//! Tauri IPC commands for the native Rust MCP server (grafyn-mcp binary).
//! Unlike the old Python sidecar, the MCP server is a standalone binary that
//! Claude Desktop launches directly via stdio — no process management needed.

use crate::AppState;
use tauri::State;

/// MCP status and configuration information
#[derive(serde::Serialize)]
pub struct McpInfo {
    /// Whether the grafyn-mcp binary was found
    pub available: bool,
    /// Path to the grafyn-mcp binary (if found)
    pub binary_path: Option<String>,
    /// Claude Desktop config snippet
    pub config_snippet: String,
}

/// Get MCP status and configuration
///
/// Returns whether the MCP binary is available and the config snippet
/// for Claude Desktop integration.
#[tauri::command]
pub async fn get_mcp_status(state: State<'_, AppState>) -> Result<McpInfo, String> {
    let binary_path = find_mcp_binary();
    let settings = state.settings_service.read().await;
    let vault_path = settings.vault_path();
    let data_path = settings.data_path();

    let config_snippet = build_config_snippet(
        binary_path.as_deref(),
        &vault_path.to_string_lossy(),
        &data_path.to_string_lossy(),
    );

    Ok(McpInfo {
        available: binary_path.is_some(),
        binary_path,
        config_snippet,
    })
}

/// Get the Claude Desktop config snippet for MCP integration
///
/// Returns a JSON string that users can paste into claude_desktop_config.json
#[tauri::command]
pub async fn get_mcp_config_snippet(state: State<'_, AppState>) -> Result<String, String> {
    let binary_path = find_mcp_binary();
    let settings = state.settings_service.read().await;
    let vault_path = settings.vault_path();
    let data_path = settings.data_path();

    Ok(build_config_snippet(
        binary_path.as_deref(),
        &vault_path.to_string_lossy(),
        &data_path.to_string_lossy(),
    ))
}

/// Find the grafyn-mcp binary adjacent to the current executable.
///
/// Tauri bundles external binaries in the same directory as the main app.
fn find_mcp_binary() -> Option<String> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()?
        .to_path_buf();

    // Check for platform-specific binary name
    let binary_name = if cfg!(target_os = "windows") {
        "grafyn-mcp.exe"
    } else {
        "grafyn-mcp"
    };

    let binary_path = exe_dir.join(binary_name);
    if binary_path.exists() {
        Some(binary_path.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Build the Claude Desktop config snippet JSON.
fn build_config_snippet(binary_path: Option<&str>, vault_path: &str, data_path: &str) -> String {
    let cmd = binary_path.unwrap_or(if cfg!(target_os = "windows") {
        "grafyn-mcp.exe"
    } else {
        "grafyn-mcp"
    });

    let config = serde_json::json!({
        "mcpServers": {
            "grafyn": {
                "command": cmd,
                "args": ["--vault", vault_path, "--data", data_path]
            }
        }
    });

    serde_json::to_string_pretty(&config).unwrap_or_default()
}
