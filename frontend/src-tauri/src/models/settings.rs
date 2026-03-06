//! User settings model for desktop app configuration

use serde::{Deserialize, Serialize};

/// User-configurable settings for the desktop app
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    /// Path to the vault (markdown notes folder)
    /// If None, uses default ~/Documents/Grafyn/vault
    pub vault_path: Option<String>,

    /// OpenRouter API key for LLM features (Canvas)
    /// Users need their own key as it has usage costs
    #[serde(default)]
    pub openrouter_api_key: Option<String>,

    /// Whether the user has completed initial setup
    #[serde(default)]
    pub setup_completed: bool,

    /// Theme preference (light/dark/system)
    #[serde(default = "default_theme")]
    pub theme: String,

    /// Whether MCP sidecar is enabled
    #[serde(default)]
    pub mcp_enabled: bool,

    /// LLM model for distillation & link discovery (OpenRouter model ID)
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
}

fn default_theme() -> String {
    "system".to_string()
}

pub fn default_llm_model() -> String {
    "anthropic/claude-3.5-haiku".to_string()
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            vault_path: None,
            openrouter_api_key: None,
            setup_completed: false,
            theme: default_theme(),
            mcp_enabled: false,
            llm_model: default_llm_model(),
        }
    }
}

impl UserSettings {
    /// Check if the app needs initial setup
    pub fn needs_setup(&self) -> bool {
        !self.setup_completed || self.vault_path.is_none()
    }

    /// Check if OpenRouter is configured
    pub fn has_openrouter_key(&self) -> bool {
        self.openrouter_api_key
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false)
    }

    /// Get the effective vault path (with default fallback)
    pub fn effective_vault_path(&self) -> std::path::PathBuf {
        if let Some(ref path) = self.vault_path {
            std::path::PathBuf::from(path)
        } else {
            // Default to ~/Documents/Grafyn/vault
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("Grafyn")
                .join("vault")
        }
    }

    /// Get the effective data path (always in app data directory)
    pub fn effective_data_path(&self) -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
            .join("Grafyn")
            .join("data")
    }
}

/// Settings update request from frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsUpdate {
    pub vault_path: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub setup_completed: Option<bool>,
    pub theme: Option<String>,
    pub mcp_enabled: Option<bool>,
    pub llm_model: Option<String>,
}

/// Response for settings status check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsStatus {
    pub needs_setup: bool,
    pub has_vault_path: bool,
    pub has_openrouter_key: bool,
    pub vault_path: Option<String>,
    pub theme: String,
    pub mcp_enabled: bool,
    pub llm_model: String,
}

impl From<&UserSettings> for SettingsStatus {
    fn from(settings: &UserSettings) -> Self {
        Self {
            needs_setup: settings.needs_setup(),
            has_vault_path: settings.vault_path.is_some(),
            has_openrouter_key: settings.has_openrouter_key(),
            vault_path: settings.vault_path.clone(),
            theme: settings.theme.clone(),
            mcp_enabled: settings.mcp_enabled,
            llm_model: settings.llm_model.clone(),
        }
    }
}
