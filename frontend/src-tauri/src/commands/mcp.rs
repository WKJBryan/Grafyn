//! MCP Sidecar Commands
//!
//! Tauri IPC commands for managing the Python backend sidecar that provides
//! MCP (Model Context Protocol) support for Claude Desktop and ChatGPT.

use crate::services::mcp_sidecar::SidecarStatus;
use crate::AppState;
use tauri::State;

/// Response containing MCP server information
#[derive(serde::Serialize)]
pub struct McpInfo {
    /// Whether MCP sidecar is enabled
    pub enabled: bool,
    /// Current status of the sidecar
    pub status: SidecarStatus,
    /// MCP endpoint URL (for Claude Desktop config)
    pub mcp_url: String,
    /// API base URL
    pub api_url: String,
    /// Port the server is running on
    pub port: u16,
}

/// Get MCP sidecar status and connection information
#[tauri::command]
pub async fn get_mcp_status(state: State<'_, AppState>) -> Result<McpInfo, String> {
    let sidecar = &state.mcp_sidecar;

    Ok(McpInfo {
        enabled: sidecar.is_enabled(),
        status: sidecar.status().await,
        mcp_url: sidecar.mcp_url(),
        api_url: sidecar.api_url(),
        port: crate::services::mcp_sidecar::DEFAULT_MCP_PORT,
    })
}

/// Start the MCP sidecar
#[tauri::command]
pub async fn start_mcp_sidecar(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<McpInfo, String> {
    let sidecar = &state.mcp_sidecar;

    // Enable the sidecar
    sidecar.set_enabled(true);

    // Start it
    sidecar.start(&app_handle).await?;

    // Return updated status
    Ok(McpInfo {
        enabled: sidecar.is_enabled(),
        status: sidecar.status().await,
        mcp_url: sidecar.mcp_url(),
        api_url: sidecar.api_url(),
        port: crate::services::mcp_sidecar::DEFAULT_MCP_PORT,
    })
}

/// Stop the MCP sidecar
#[tauri::command]
pub async fn stop_mcp_sidecar(state: State<'_, AppState>) -> Result<McpInfo, String> {
    let sidecar = &state.mcp_sidecar;

    // Stop it
    sidecar.stop().await?;

    // Disable the sidecar
    sidecar.set_enabled(false);

    // Return updated status
    Ok(McpInfo {
        enabled: sidecar.is_enabled(),
        status: sidecar.status().await,
        mcp_url: sidecar.mcp_url(),
        api_url: sidecar.api_url(),
        port: crate::services::mcp_sidecar::DEFAULT_MCP_PORT,
    })
}

/// Restart the MCP sidecar
#[tauri::command]
pub async fn restart_mcp_sidecar(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<McpInfo, String> {
    let sidecar = &state.mcp_sidecar;

    // Restart it
    sidecar.restart(&app_handle).await?;

    // Return updated status
    Ok(McpInfo {
        enabled: sidecar.is_enabled(),
        status: sidecar.status().await,
        mcp_url: sidecar.mcp_url(),
        api_url: sidecar.api_url(),
        port: crate::services::mcp_sidecar::DEFAULT_MCP_PORT,
    })
}

/// Check if the MCP sidecar is healthy
#[tauri::command]
pub async fn check_mcp_health(state: State<'_, AppState>) -> Result<bool, String> {
    let sidecar = &state.mcp_sidecar;
    Ok(sidecar.health_check().await)
}

/// Get the MCP URL for Claude Desktop configuration
///
/// Returns a JSON snippet that users can add to their claude_desktop_config.json
#[tauri::command]
pub async fn get_mcp_config_snippet(state: State<'_, AppState>) -> Result<String, String> {
    let sidecar = &state.mcp_sidecar;
    let mcp_url = sidecar.mcp_url();

    let config = serde_json::json!({
        "mcpServers": {
            "grafyn-local": {
                "url": mcp_url
            }
        }
    });

    Ok(serde_json::to_string_pretty(&config).unwrap_or_default())
}
