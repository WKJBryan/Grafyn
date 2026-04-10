//! User settings model for desktop app configuration

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanvasModelPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
}

fn default_canvas_model_presets() -> Vec<CanvasModelPreset> {
    Vec::new()
}

/// User-configurable settings for the desktop app
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    /// Path to the vault (markdown notes folder)
    /// If None, uses default ~/Documents/Grafyn/vault
    pub vault_path: Option<String>,

    /// OpenRouter API key for LLM features (Canvas)
    /// Users need their own key as it has usage costs
    #[serde(default, skip_serializing)]
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

    /// Whether smart web search auto-detection is enabled in canvas
    #[serde(default = "default_smart_web_search")]
    pub smart_web_search: bool,

    /// Whether the background link discovery scheduler is enabled
    #[serde(default = "default_background_link_discovery_enabled")]
    pub background_link_discovery_enabled: bool,

    /// Whether background discovery may use LLM reranking for high-priority notes
    #[serde(default)]
    pub background_link_discovery_llm_enabled: bool,

    /// Whether the continuous vault optimizer is enabled
    #[serde(default = "default_background_vault_optimizer_enabled")]
    pub background_vault_optimizer_enabled: bool,

    /// Whether the vault optimizer may use LLM refinement when budget allows
    #[serde(default)]
    pub background_vault_optimizer_llm_enabled: bool,

    /// Soft monthly LLM budget for optimizer work, in USD whole dollars
    #[serde(default = "default_background_vault_optimizer_budget_monthly")]
    pub background_vault_optimizer_budget_monthly: u32,

    /// Maximum number of optimizer-authored note writes per day
    #[serde(default = "default_background_vault_optimizer_max_daily_writes")]
    pub background_vault_optimizer_max_daily_writes: u32,

    /// Write mode for optimizer-authored note edits
    #[serde(default = "default_background_vault_optimizer_edit_mode")]
    pub background_vault_optimizer_edit_mode: String,

    /// Whether the vault-local program.md policy file is active
    #[serde(default = "default_background_vault_optimizer_program_enabled")]
    pub background_vault_optimizer_program_enabled: bool,

    /// Path to the vault-local optimizer program file relative to the vault root
    #[serde(default = "default_vault_optimizer_program_path")]
    pub vault_optimizer_program_path: String,

    /// Saved model presets for canvas prompts
    #[serde(default = "default_canvas_model_presets")]
    pub canvas_model_presets: Vec<CanvasModelPreset>,
}

fn default_theme() -> String {
    "system".to_string()
}

pub fn default_llm_model() -> String {
    "anthropic/claude-3.5-haiku".to_string()
}

fn default_smart_web_search() -> bool {
    true
}

fn default_background_link_discovery_enabled() -> bool {
    true
}

fn default_background_vault_optimizer_enabled() -> bool {
    true
}

fn default_background_vault_optimizer_budget_monthly() -> u32 {
    0
}

fn default_background_vault_optimizer_max_daily_writes() -> u32 {
    25
}

fn default_background_vault_optimizer_edit_mode() -> String {
    "sidecar_first".to_string()
}

fn default_background_vault_optimizer_program_enabled() -> bool {
    true
}

fn default_vault_optimizer_program_path() -> String {
    "_grafyn/program.md".to_string()
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
            smart_web_search: true,
            background_link_discovery_enabled: default_background_link_discovery_enabled(),
            background_link_discovery_llm_enabled: false,
            background_vault_optimizer_enabled: default_background_vault_optimizer_enabled(),
            background_vault_optimizer_llm_enabled: false,
            background_vault_optimizer_budget_monthly:
                default_background_vault_optimizer_budget_monthly(),
            background_vault_optimizer_max_daily_writes:
                default_background_vault_optimizer_max_daily_writes(),
            background_vault_optimizer_edit_mode: default_background_vault_optimizer_edit_mode(),
            background_vault_optimizer_program_enabled:
                default_background_vault_optimizer_program_enabled(),
            vault_optimizer_program_path: default_vault_optimizer_program_path(),
            canvas_model_presets: default_canvas_model_presets(),
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
            .unwrap_or_else(|| {
                dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
            })
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
    pub smart_web_search: Option<bool>,
    pub background_link_discovery_enabled: Option<bool>,
    pub background_link_discovery_llm_enabled: Option<bool>,
    pub background_vault_optimizer_enabled: Option<bool>,
    pub background_vault_optimizer_llm_enabled: Option<bool>,
    pub background_vault_optimizer_budget_monthly: Option<u32>,
    pub background_vault_optimizer_max_daily_writes: Option<u32>,
    pub background_vault_optimizer_edit_mode: Option<String>,
    pub background_vault_optimizer_program_enabled: Option<bool>,
    pub vault_optimizer_program_path: Option<String>,
    pub canvas_model_presets: Option<Vec<CanvasModelPreset>>,
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
    pub smart_web_search: bool,
    pub background_link_discovery_enabled: bool,
    pub background_link_discovery_llm_enabled: bool,
    pub background_vault_optimizer_enabled: bool,
    pub background_vault_optimizer_llm_enabled: bool,
    pub background_vault_optimizer_budget_monthly: u32,
    pub background_vault_optimizer_max_daily_writes: u32,
    pub background_vault_optimizer_edit_mode: String,
    pub background_vault_optimizer_program_enabled: bool,
    pub vault_optimizer_program_path: String,
    pub canvas_model_presets: Vec<CanvasModelPreset>,
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
            smart_web_search: settings.smart_web_search,
            background_link_discovery_enabled: settings.background_link_discovery_enabled,
            background_link_discovery_llm_enabled: settings.background_link_discovery_llm_enabled,
            background_vault_optimizer_enabled: settings.background_vault_optimizer_enabled,
            background_vault_optimizer_llm_enabled: settings.background_vault_optimizer_llm_enabled,
            background_vault_optimizer_budget_monthly: settings
                .background_vault_optimizer_budget_monthly,
            background_vault_optimizer_max_daily_writes: settings
                .background_vault_optimizer_max_daily_writes,
            background_vault_optimizer_edit_mode: settings
                .background_vault_optimizer_edit_mode
                .clone(),
            background_vault_optimizer_program_enabled: settings
                .background_vault_optimizer_program_enabled,
            vault_optimizer_program_path: settings.vault_optimizer_program_path.clone(),
            canvas_model_presets: settings.canvas_model_presets.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_start_with_empty_canvas_presets() {
        let settings = UserSettings::default();
        assert!(settings.canvas_model_presets.is_empty());
        assert!(settings.background_link_discovery_enabled);
        assert!(!settings.background_link_discovery_llm_enabled);
        assert!(settings.background_vault_optimizer_enabled);
        assert!(!settings.background_vault_optimizer_llm_enabled);
        assert_eq!(settings.background_vault_optimizer_budget_monthly, 0);
        assert_eq!(settings.background_vault_optimizer_edit_mode, "sidecar_first");
        assert_eq!(settings.vault_optimizer_program_path, "_grafyn/program.md");
    }

    #[test]
    fn older_settings_payloads_deserialize_without_canvas_presets() {
        let json = r#"{
            "vault_path": "C:\\Vault",
            "setup_completed": true,
            "theme": "dark",
            "mcp_enabled": false,
            "llm_model": "openai/gpt-4o",
            "smart_web_search": true
        }"#;

        let settings: UserSettings =
            serde_json::from_str(json).expect("settings should deserialize");
        assert!(settings.canvas_model_presets.is_empty());
        assert_eq!(settings.llm_model, "openai/gpt-4o");
        assert!(settings.background_link_discovery_enabled);
        assert!(!settings.background_link_discovery_llm_enabled);
        assert!(settings.background_vault_optimizer_enabled);
        assert!(!settings.background_vault_optimizer_llm_enabled);
        assert_eq!(settings.background_vault_optimizer_budget_monthly, 0);
    }
}
