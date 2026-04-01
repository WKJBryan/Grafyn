//! Settings service for managing user preferences

use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use anyhow::{Context, Result};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "com.grafyn.app";
const OPENROUTER_KEY_ACCOUNT: &str = "openrouter_api_key";

/// Service for managing user settings
#[derive(Debug, Clone)]
pub struct SettingsService {
    config_path: PathBuf,
    settings: UserSettings,
}

impl SettingsService {
    /// Create a SettingsService with default settings (used as fallback)
    pub fn load_defaults() -> Self {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::data_local_dir())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grafyn");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            log::error!("Failed to create config directory {}: {}", config_dir.display(), e);
        }

        Self {
            config_path: config_dir.join("settings.json"),
            settings: UserSettings::default(),
        }
    }

    /// Load settings from disk or create defaults
    pub fn load() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::data_local_dir())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grafyn");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            log::error!("Failed to create config directory {}: {}", config_dir.display(), e);
        }
        let config_path = config_dir.join("settings.json");

        let mut settings: UserSettings = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read settings file")?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            UserSettings::default()
        };

        let mut migrated_legacy_plaintext_key = false;

        // Prefer OS keychain storage for API keys.
        if let Some(stored_key) = load_openrouter_api_key() {
            settings.openrouter_api_key = Some(stored_key);
        } else if let Some(legacy_key) = settings.openrouter_api_key.clone() {
            if !legacy_key.is_empty() {
                if let Err(error) = store_openrouter_api_key(&legacy_key) {
                    log::warn!("Failed to migrate OpenRouter key to OS keychain: {}", error);
                }
                migrated_legacy_plaintext_key = true;
            }
        }

        let service = Self {
            config_path,
            settings,
        };

        // Re-save after migration so settings.json no longer contains plaintext API keys.
        if migrated_legacy_plaintext_key {
            if let Err(error) = service.save() {
                log::warn!("Failed to persist settings cleanup after key migration: {}", error);
            }
        }

        Ok(service)
    }

    /// Get current settings
    pub fn get(&self) -> &UserSettings {
        &self.settings
    }

    /// Get settings status for frontend
    pub fn status(&self) -> SettingsStatus {
        SettingsStatus::from(&self.settings)
    }

    /// Update settings and persist to disk
    pub fn update(&mut self, update: SettingsUpdate) -> Result<UserSettings> {
        // Apply updates
        if let Some(vault_path) = update.vault_path {
            // Validate the path exists (or can be created)
            let path = PathBuf::from(&vault_path);
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .context("Failed to create vault directory")?;
            }
            self.settings.vault_path = Some(vault_path);
        }

        if let Some(api_key) = update.openrouter_api_key {
            self.settings.openrouter_api_key = if api_key.is_empty() {
                if let Err(error) = clear_openrouter_api_key() {
                    log::warn!("Failed to clear OpenRouter API key from OS keychain: {}", error);
                }
                None
            } else {
                if let Err(error) = store_openrouter_api_key(&api_key) {
                    log::warn!("Failed to store OpenRouter API key in OS keychain: {}", error);
                }
                Some(api_key)
            };
        }

        if let Some(setup_completed) = update.setup_completed {
            self.settings.setup_completed = setup_completed;
        }

        if let Some(theme) = update.theme {
            self.settings.theme = theme;
        }

        if let Some(mcp_enabled) = update.mcp_enabled {
            self.settings.mcp_enabled = mcp_enabled;
        }

        if let Some(llm_model) = update.llm_model {
            self.settings.llm_model = if llm_model.is_empty() {
                crate::models::settings::default_llm_model()
            } else {
                llm_model
            };
        }

        if let Some(smart_web_search) = update.smart_web_search {
            self.settings.smart_web_search = smart_web_search;
        }

        if let Some(canvas_model_presets) = update.canvas_model_presets {
            self.settings.canvas_model_presets = canvas_model_presets;
        }

        // Persist to disk
        self.save()?;

        Ok(self.settings.clone())
    }

    /// Save settings to disk
    fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.settings)
            .context("Failed to serialize settings")?;
        std::fs::write(&self.config_path, json)
            .context("Failed to write settings file")?;
        log::info!("Settings saved to {:?}", self.config_path);
        Ok(())
    }

    /// Get the effective vault path
    pub fn vault_path(&self) -> PathBuf {
        self.settings.effective_vault_path()
    }

    /// Get the effective data path
    pub fn data_path(&self) -> PathBuf {
        self.settings.effective_data_path()
    }

    /// Get OpenRouter API key (if configured)
    pub fn openrouter_api_key(&self) -> Option<&str> {
        self.settings.openrouter_api_key.as_deref()
    }

    /// Check if initial setup is needed
    pub fn needs_setup(&self) -> bool {
        self.settings.needs_setup()
    }

    /// Mark setup as completed
    pub fn complete_setup(&mut self) -> Result<()> {
        self.settings.setup_completed = true;
        self.save()
    }

    /// Check if MCP sidecar is enabled in settings
    pub fn mcp_enabled(&self) -> bool {
        self.settings.mcp_enabled
    }

    /// Clear the OpenRouter API key
    pub fn clear_openrouter_key(&mut self) -> Result<()> {
        if let Err(error) = clear_openrouter_api_key() {
            log::warn!("Failed to clear OpenRouter API key from OS keychain: {}", error);
        }
        self.settings.openrouter_api_key = None;
        self.save()
    }
}

fn keyring_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, OPENROUTER_KEY_ACCOUNT)
        .context("Failed to initialize OS keychain entry")
}

fn load_openrouter_api_key() -> Option<String> {
    let entry = match keyring_entry() {
        Ok(entry) => entry,
        Err(error) => {
            log::debug!("OpenRouter keychain unavailable: {}", error);
            return None;
        }
    };
    match entry.get_password() {
        Ok(password) if !password.is_empty() => Some(password),
        Ok(_) => None,
        Err(error) => {
            log::debug!("OpenRouter key not available in OS keychain: {}", error);
            None
        }
    }
}

fn store_openrouter_api_key(api_key: &str) -> Result<()> {
    let entry = keyring_entry()?;
    entry
        .set_password(api_key)
        .context("Failed to store OpenRouter API key in OS keychain")
}

fn clear_openrouter_api_key() -> Result<()> {
    let entry = keyring_entry()?;
    entry
        .delete_password()
        .context("Failed to delete OpenRouter API key from OS keychain")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::CanvasModelPreset;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_default_settings() {
        let settings = UserSettings::default();
        assert!(settings.needs_setup());
        assert!(!settings.has_openrouter_key());
        assert!(settings.canvas_model_presets.is_empty());
    }

    #[test]
    fn test_settings_update() {
        let mut settings = UserSettings::default();
        settings.vault_path = Some("/test/vault".to_string());
        settings.openrouter_api_key = Some("sk-test".to_string());
        settings.setup_completed = true;

        assert!(!settings.needs_setup());
        assert!(settings.has_openrouter_key());
    }

    #[test]
    fn test_update_persists_canvas_model_presets() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("grafyn-settings-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let config_path = temp_dir.join("settings.json");

        let mut service = SettingsService {
            config_path: config_path.clone(),
            settings: UserSettings::default(),
        };

        let presets = vec![CanvasModelPreset {
            id: "preset-1".to_string(),
            name: "Fast trio".to_string(),
            model_ids: vec!["openai/gpt-4o".to_string(), "anthropic/claude-3.5-sonnet".to_string()],
        }];

        let updated = service
            .update(SettingsUpdate {
                vault_path: None,
                openrouter_api_key: None,
                setup_completed: None,
                theme: None,
                mcp_enabled: None,
                llm_model: None,
                smart_web_search: None,
                canvas_model_presets: Some(presets.clone()),
            })
            .expect("settings update should succeed");

        assert_eq!(updated.canvas_model_presets, presets);

        let persisted = std::fs::read_to_string(config_path).expect("settings file should exist");
        assert!(persisted.contains("\"canvas_model_presets\""));
        assert!(persisted.contains("\"Fast trio\""));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
