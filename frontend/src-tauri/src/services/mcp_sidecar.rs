//! MCP Sidecar Service
//!
//! Manages the Python backend sidecar process that provides MCP (Model Context Protocol)
//! support for Claude Desktop and ChatGPT integration.
//!
//! The sidecar runs as a separate process on localhost and provides:
//! - MCP endpoint at /sse for AI assistant integration
//! - Semantic search with vector embeddings (sentence-transformers)
//! - LanceDB vector database support
//! - Full API compatibility with web backend

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tauri::api::process::{Command, CommandChild, CommandEvent};
use tauri::Manager;
use tokio::sync::RwLock;

/// Default port for the Python backend sidecar
pub const DEFAULT_MCP_PORT: u16 = 8765;

/// Maximum consecutive auto-restart attempts before giving up
const MAX_RESTARTS: u32 = 3;

/// Health check interval in seconds
const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Status of the MCP sidecar process
#[derive(Debug, Clone, serde::Serialize)]
pub enum SidecarStatus {
    /// Sidecar is not running
    Stopped,
    /// Sidecar is starting up
    Starting,
    /// Sidecar is running and ready
    Running { port: u16, url: String },
    /// Sidecar failed to start
    Failed { error: String },
}

/// MCP Sidecar Service manages the Python backend process
pub struct McpSidecarService {
    /// Current status of the sidecar
    status: RwLock<SidecarStatus>,
    /// Port the sidecar is running on
    port: u16,
    /// Path to the vault directory
    vault_path: PathBuf,
    /// Path to the data directory
    data_path: PathBuf,
    /// Whether the sidecar is enabled
    enabled: AtomicBool,
    /// Child process handle (kept alive to prevent process termination)
    #[allow(dead_code)]
    child: RwLock<Option<CommandChild>>,
    /// Consecutive restart attempts (reset on successful health check)
    restart_count: AtomicU32,
    /// Whether the health check loop is already running
    monitoring: AtomicBool,
}

impl McpSidecarService {
    /// Create a new MCP sidecar service
    pub fn new(vault_path: PathBuf, data_path: PathBuf, port: Option<u16>) -> Self {
        Self {
            status: RwLock::new(SidecarStatus::Stopped),
            port: port.unwrap_or(DEFAULT_MCP_PORT),
            vault_path,
            data_path,
            enabled: AtomicBool::new(false),
            child: RwLock::new(None),
            restart_count: AtomicU32::new(0),
            monitoring: AtomicBool::new(false),
        }
    }

    /// Get the current status of the sidecar
    pub async fn status(&self) -> SidecarStatus {
        self.status.read().await.clone()
    }

    /// Get the MCP endpoint URL
    pub fn mcp_url(&self) -> String {
        format!("http://localhost:{}/sse", self.port)
    }

    /// Get the API base URL
    pub fn api_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Check if sidecar is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Enable or disable the sidecar
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Start the sidecar and begin monitoring its health.
    ///
    /// This is the preferred entry point — it calls `start()` and then
    /// spawns a background health-check loop that auto-restarts the
    /// sidecar if it crashes (up to MAX_RESTARTS times).
    pub async fn start_monitored(self: &Arc<Self>, app_handle: &tauri::AppHandle) -> Result<(), String> {
        self.restart_count.store(0, Ordering::SeqCst);
        self.start(app_handle).await?;

        // Only spawn one monitoring loop at a time
        if !self.monitoring.swap(true, Ordering::SeqCst) {
            let sidecar = Arc::clone(self);
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                sidecar.health_check_loop(&handle).await;
            });
        }

        Ok(())
    }

    /// Start the Python backend sidecar
    pub async fn start(&self, app_handle: &tauri::AppHandle) -> Result<(), String> {
        // Check if already running
        {
            let status = self.status.read().await;
            if matches!(*status, SidecarStatus::Running { .. } | SidecarStatus::Starting) {
                return Ok(());
            }
        }

        // Update status to starting
        {
            let mut status = self.status.write().await;
            *status = SidecarStatus::Starting;
        }

        log::info!("Starting MCP sidecar on port {}", self.port);

        // Build command arguments
        let args = vec![
            "--port".to_string(),
            self.port.to_string(),
            "--vault-path".to_string(),
            self.vault_path.to_string_lossy().to_string(),
            "--data-path".to_string(),
            self.data_path.to_string_lossy().to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(), // Bind to localhost only for security
        ];

        // Spawn the sidecar process
        let result = Command::new_sidecar("grafyn-backend")
            .map_err(|e| format!("Failed to create sidecar command: {}", e))?
            .args(args)
            .spawn();

        match result {
            Ok((mut rx, child)) => {
                // Store the child handle to keep it alive
                {
                    let mut child_guard = self.child.write().await;
                    *child_guard = Some(child);
                }

                // Clone values for the async task
                let port = self.port;
                let status_clone = Arc::new(RwLock::new(SidecarStatus::Starting));

                // Spawn a task to handle sidecar output
                let handle = app_handle.clone();
                let status_for_task = status_clone.clone();

                tauri::async_runtime::spawn(async move {
                    let mut startup_detected = false;

                    while let Some(event) = rx.recv().await {
                        match event {
                            CommandEvent::Stdout(line) => {
                                log::info!("[mcp-sidecar] {}", line);

                                // Detect when uvicorn is ready
                                if line.contains("Uvicorn running") || line.contains("Application startup complete") {
                                    startup_detected = true;
                                    let mut status = status_for_task.write().await;
                                    *status = SidecarStatus::Running {
                                        port,
                                        url: format!("http://localhost:{}/sse", port),
                                    };
                                    log::info!("MCP sidecar ready on port {}", port);

                                    // Emit event to frontend
                                    let _ = handle.emit_all("mcp-sidecar-ready", &format!("http://localhost:{}/sse", port));
                                }
                            }
                            CommandEvent::Stderr(line) => {
                                log::warn!("[mcp-sidecar] {}", line);
                            }
                            CommandEvent::Error(error) => {
                                log::error!("[mcp-sidecar] Error: {}", error);
                                let mut status = status_for_task.write().await;
                                *status = SidecarStatus::Failed {
                                    error: error.clone(),
                                };
                                let _ = handle.emit_all("mcp-sidecar-error", error);
                            }
                            CommandEvent::Terminated(payload) => {
                                log::info!("[mcp-sidecar] Terminated with code: {:?}", payload.code);
                                let mut status = status_for_task.write().await;
                                if !startup_detected {
                                    *status = SidecarStatus::Failed {
                                        error: format!("Process terminated with code: {:?}", payload.code),
                                    };
                                } else {
                                    *status = SidecarStatus::Stopped;
                                }
                                let _ = handle.emit_all("mcp-sidecar-stopped", ());
                                break;
                            }
                            _ => {}
                        }
                    }
                });

                // Wait a moment for startup
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                // Copy status from the monitoring task
                {
                    let task_status = status_clone.read().await;
                    let mut status = self.status.write().await;
                    *status = task_status.clone();
                }

                Ok(())
            }
            Err(e) => {
                let error_msg = format!("Failed to spawn sidecar: {}", e);
                log::error!("{}", error_msg);

                let mut status = self.status.write().await;
                *status = SidecarStatus::Failed {
                    error: error_msg.clone(),
                };

                Err(error_msg)
            }
        }
    }

    /// Stop the sidecar process
    pub async fn stop(&self) -> Result<(), String> {
        log::info!("Stopping MCP sidecar");

        // Take the child handle to drop it (which kills the process)
        let mut child_guard = self.child.write().await;
        if let Some(child) = child_guard.take() {
            // Kill the process
            if let Err(e) = child.kill() {
                log::warn!("Error killing sidecar process: {}", e);
            }
        }

        // Update status
        let mut status = self.status.write().await;
        *status = SidecarStatus::Stopped;

        Ok(())
    }

    /// Restart the sidecar process
    pub async fn restart(&self, app_handle: &tauri::AppHandle) -> Result<(), String> {
        self.stop().await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        self.start(app_handle).await
    }

    /// Check if the sidecar is healthy by making a request to the health endpoint
    pub async fn health_check(&self) -> bool {
        let client = reqwest::Client::new();
        let url = format!("http://localhost:{}/health", self.port);

        match client.get(&url).timeout(std::time::Duration::from_secs(2)).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Periodic health check loop that auto-restarts the sidecar on failure.
    ///
    /// Runs every HEALTH_CHECK_INTERVAL_SECS. If the sidecar is Running but
    /// fails a health check, or if it's Stopped/Failed but still enabled,
    /// an auto-restart is attempted with exponential backoff.
    async fn health_check_loop(self: &Arc<Self>, app_handle: &tauri::AppHandle) {
        log::info!("MCP health monitoring started (interval: {}s)", HEALTH_CHECK_INTERVAL_SECS);

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;

            // Stop monitoring if sidecar was explicitly disabled
            if !self.is_enabled() {
                log::info!("MCP sidecar disabled, stopping health monitor");
                self.monitoring.store(false, Ordering::SeqCst);
                break;
            }

            let current_status = self.status().await;
            match current_status {
                SidecarStatus::Running { .. } => {
                    if self.health_check().await {
                        // Healthy — reset restart counter
                        self.restart_count.store(0, Ordering::SeqCst);
                    } else {
                        log::warn!("MCP sidecar health check failed, attempting restart");
                        self.attempt_auto_restart(app_handle).await;
                    }
                }
                SidecarStatus::Stopped | SidecarStatus::Failed { .. } => {
                    // Sidecar is enabled but not running — try to bring it back
                    log::warn!("MCP sidecar not running but enabled, attempting restart");
                    self.attempt_auto_restart(app_handle).await;
                }
                SidecarStatus::Starting => {
                    // Still starting, give it time
                }
            }
        }
    }

    /// Attempt an auto-restart with exponential backoff.
    ///
    /// Backoff delays: 2s, 4s, 8s (then gives up after MAX_RESTARTS).
    async fn attempt_auto_restart(&self, app_handle: &tauri::AppHandle) {
        let count = self.restart_count.fetch_add(1, Ordering::SeqCst);

        if count >= MAX_RESTARTS {
            log::error!(
                "MCP sidecar exceeded max restart attempts ({}), giving up",
                MAX_RESTARTS
            );
            let mut status = self.status.write().await;
            *status = SidecarStatus::Failed {
                error: format!("Exceeded {} restart attempts", MAX_RESTARTS),
            };
            let _ = app_handle.emit_all(
                "mcp-sidecar-error",
                format!("MCP sidecar failed after {} restart attempts. Re-enable in Settings to try again.", MAX_RESTARTS),
            );
            return;
        }

        // Exponential backoff: 2^(count+1) seconds → 2s, 4s, 8s
        let delay_secs = 2u64.pow(count + 1);
        log::info!(
            "Auto-restarting MCP sidecar (attempt {}/{}, backoff {}s)",
            count + 1,
            MAX_RESTARTS,
            delay_secs
        );
        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;

        if let Err(e) = self.restart(app_handle).await {
            log::error!("Auto-restart failed: {}", e);
        }
    }
}

impl Drop for McpSidecarService {
    fn drop(&mut self) {
        // Note: The child process will be killed when the CommandChild is dropped
        log::info!("McpSidecarService dropped, sidecar process will be terminated");
    }
}
