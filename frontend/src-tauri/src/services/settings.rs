//! Settings service for managing user preferences

use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Service for managing user settings
#[derive(Debug, Clone)]
pub struct SettingsService {
    config_path: PathBuf,
    settings: UserSettings,
}

impl SettingsService {
    /// Load settings from disk or create defaults
    pub fn load() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::data_local_dir())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Seedream");

        std::fs::create_dir_all(&config_dir).ok();
        let config_path = config_dir.join("settings.json");

        let settings = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read settings file")?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            UserSettings::default()
        };

        Ok(Self {
            config_path,
            settings,
        })
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
                None
            } else {
                Some(api_key)
            };
        }

        if let Some(setup_completed) = update.setup_completed {
            self.settings.setup_completed = setup_completed;
        }

        if let Some(theme) = update.theme {
            self.settings.theme = theme;
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

    /// Clear the OpenRouter API key
    pub fn clear_openrouter_key(&mut self) -> Result<()> {
        self.settings.openrouter_api_key = None;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_settings() {
        let settings = UserSettings::default();
        assert!(settings.needs_setup());
        assert!(!settings.has_openrouter_key());
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
}
