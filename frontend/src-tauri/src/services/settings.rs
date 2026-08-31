//! Settings service for managing user preferences

use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

const KEYRING_SERVICE: &str = "com.grafyn.app";
const OPENROUTER_KEY_ACCOUNT: &str = "openrouter_api_key";

const LEGACY_TWIN_ASSIGNMENT_KEY: &str = "twin/legacy-assignment-v1.json";
const LEGACY_TWIN_ASSIGNMENT_STAGING_KEY: &str = "twin/mutations/staging/v1";
const LEGACY_TWIN_ASSIGNMENT_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyTwinAssignmentState {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTwinAssignmentV1 {
    schema_version: u16,
    root_scope: crate::models::twin_event::ContentDigest,
    lease_epoch_uuid: String,
    legacy_name: String,
    current_name: String,
    state: LegacyTwinAssignmentState,
}

pub(crate) fn prepare_twin_data_path_locked(
    data_path: &Path,
    vault_path: &Path,
    lease: &crate::services::twin_events::ActiveMarkdownRootLeaseV1,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
) -> Result<PathBuf> {
    prepare_twin_data_path_locked_inner(
        data_path,
        vault_path,
        lease,
        process_lock,
        &mut || {},
        &mut || {},
    )
}

#[cfg(test)]
fn prepare_twin_data_path_locked_with_rename_hook(
    data_path: &Path,
    vault_path: &Path,
    lease: &crate::services::twin_events::ActiveMarkdownRootLeaseV1,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
    mut rename_hook: impl FnMut(),
) -> Result<PathBuf> {
    prepare_twin_data_path_locked_inner(
        data_path,
        vault_path,
        lease,
        process_lock,
        &mut || {},
        &mut rename_hook,
    )
}

#[cfg(test)]
fn prepare_twin_data_path_locked_with_marker_install_hook(
    data_path: &Path,
    vault_path: &Path,
    lease: &crate::services::twin_events::ActiveMarkdownRootLeaseV1,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
    mut marker_install_hook: impl FnMut(),
) -> Result<PathBuf> {
    prepare_twin_data_path_locked_inner(
        data_path,
        vault_path,
        lease,
        process_lock,
        &mut marker_install_hook,
        &mut || {},
    )
}

fn prepare_twin_data_path_locked_inner(
    data_path: &Path,
    vault_path: &Path,
    lease: &crate::services::twin_events::ActiveMarkdownRootLeaseV1,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
    marker_install_hook: &mut impl FnMut(),
    rename_hook: &mut impl FnMut(),
) -> Result<PathBuf> {
    crate::services::twin_events::validate_real_directory(data_path, "Grafyn data root")
        .map_err(anyhow::Error::new)?;
    crate::services::twin_events::validate_real_directory(vault_path, "Markdown vault root")
        .map_err(anyhow::Error::new)?;
    let current = crate::models::settings::twin_data_path_for_scope(data_path, &lease.root_scope);
    let legacy = crate::models::settings::legacy_twin_data_path_for_vault(data_path, vault_path);
    let current_name = current
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("Twin namespace is not UTF-8"))?;
    let legacy_name = legacy
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("legacy Twin namespace is not UTF-8"))?;
    let root =
        crate::services::twin_events::AnchoredRoot::open(data_path).map_err(anyhow::Error::new)?;
    if !process_lock
        .covers_data_path(data_path)
        .map_err(anyhow::Error::new)?
    {
        anyhow::bail!("legacy Twin assignment lock belongs to another data root");
    }
    let expected_scope = if lease.is_stable() {
        crate::services::sync::identity::load_vault_identity(vault_path)
            .map_err(anyhow::Error::new)?
            .root_scope
    } else {
        crate::services::twin_events::root_identity_for_path(vault_path)
            .map_err(anyhow::Error::new)?
    };
    let durable_lease: crate::services::twin_events::ActiveMarkdownRootLeaseV1 =
        serde_json::from_slice(
            &root
                .read_bounded("twin/events/active-markdown-root-v1.json", 4096)
                .map_err(anyhow::Error::new)?
                .ok_or_else(|| anyhow::anyhow!("active Markdown root lease is missing"))?,
        )
        .context("invalid active Markdown root lease")?;
    if &durable_lease != lease || lease.root_scope != expected_scope {
        anyhow::bail!("legacy Twin assignment lease changed");
    }
    if lease.is_stable() {
        std::fs::create_dir_all(&current)?;
        crate::services::twin_events::validate_real_directory(&current, "Twin namespace")
            .map_err(anyhow::Error::new)?;
        return Ok(current);
    }
    root.open_directory("twin", true)
        .map_err(anyhow::Error::new)?;
    let current_key = format!("twin/{current_name}");
    let legacy_key = format!("twin/{legacy_name}");
    let has_current = root
        .directory_exists(&current_key)
        .map_err(anyhow::Error::new)?;
    let has_legacy = root
        .directory_exists(&legacy_key)
        .map_err(anyhow::Error::new)?;
    let assignment = match root
        .read_bounded(LEGACY_TWIN_ASSIGNMENT_KEY, LEGACY_TWIN_ASSIGNMENT_LIMIT)
        .map_err(anyhow::Error::new)?
    {
        Some(bytes) => Some(
            serde_json::from_slice::<LegacyTwinAssignmentV1>(&bytes)
                .context("invalid legacy Twin assignment")?,
        ),
        None => None,
    };
    if let Some(assignment) = &assignment {
        let matches_authority = assignment.schema_version == 1
            && assignment.root_scope == lease.root_scope
            && assignment.lease_epoch_uuid == lease.epoch_uuid
            && assignment.legacy_name == legacy_name
            && assignment.current_name == current_name;
        if !matches_authority && assignment.state == LegacyTwinAssignmentState::Prepared {
            anyhow::bail!("prepared legacy Twin assignment belongs to another root authority");
        }
        if !matches_authority
            && (assignment.schema_version != 1
                || assignment.root_scope != lease.root_scope
                || assignment.lease_epoch_uuid != lease.epoch_uuid
                || assignment.legacy_name != legacy_name
                || assignment.current_name != current_name)
        {
            if has_legacy {
                anyhow::bail!("legacy Twin namespace belongs to another root authority");
            }
            return Ok(current);
        }
        if assignment.state == LegacyTwinAssignmentState::Prepared {
            match (has_legacy, has_current) {
                (true, false) => {}
                (false, true) => {
                    write_legacy_twin_assignment(
                        &root,
                        &LegacyTwinAssignmentV1 {
                            state: LegacyTwinAssignmentState::Committed,
                            ..assignment.clone()
                        },
                    )?;
                    return Ok(current);
                }
                (false, false) => {
                    anyhow::bail!(
                        "prepared legacy Twin assignment has neither source nor destination"
                    )
                }
                (true, true) => anyhow::bail!(
                    "legacy and current Twin namespaces both exist; refusing to merge"
                ),
            }
        }
    }
    if has_current && has_legacy {
        anyhow::bail!("legacy and current Twin namespaces both exist; refusing to merge");
    }
    if has_legacy {
        #[cfg(not(windows))]
        anyhow::bail!(
            "legacy Twin namespace uses a case-folded root hash and is ambiguous on this platform"
        );
        #[cfg(windows)]
        {
            let prepared = LegacyTwinAssignmentV1 {
                schema_version: 1,
                root_scope: lease.root_scope.clone(),
                lease_epoch_uuid: lease.epoch_uuid.clone(),
                legacy_name: legacy_name.to_string(),
                current_name: current_name.to_string(),
                state: LegacyTwinAssignmentState::Prepared,
            };
            if assignment.is_none() {
                install_initial_legacy_twin_assignment(&root, &prepared, marker_install_hook)?;
            }
        }
        rename_twin_namespace_no_replace(&root, &legacy_key, &current_key, rename_hook)?;
    }
    if assignment
        .as_ref()
        .is_some_and(|assignment| assignment.state == LegacyTwinAssignmentState::Prepared)
        || has_legacy
    {
        write_legacy_twin_assignment(
            &root,
            &LegacyTwinAssignmentV1 {
                schema_version: 1,
                root_scope: lease.root_scope.clone(),
                lease_epoch_uuid: lease.epoch_uuid.clone(),
                legacy_name: legacy_name.to_string(),
                current_name: current_name.to_string(),
                state: LegacyTwinAssignmentState::Committed,
            },
        )?;
    }
    Ok(current)
}

fn rename_twin_namespace_no_replace(
    root: &crate::services::twin_events::AnchoredRoot,
    legacy_key: &str,
    current_key: &str,
    rename_hook: &mut impl FnMut(),
) -> Result<()> {
    #[cfg(test)]
    {
        root.rename_no_replace_with_hook(legacy_key, current_key, false, || rename_hook())
            .map_err(anyhow::Error::new)
    }
    #[cfg(not(test))]
    {
        let _ = rename_hook;
        root.rename_no_replace(legacy_key, current_key, false)
            .map_err(anyhow::Error::new)
    }
}

fn write_legacy_twin_assignment(
    root: &crate::services::twin_events::AnchoredRoot,
    assignment: &LegacyTwinAssignmentV1,
) -> Result<()> {
    root.put_atomic(
        LEGACY_TWIN_ASSIGNMENT_KEY,
        &encoded_legacy_twin_assignment(assignment)?,
    )
    .map_err(anyhow::Error::new)
}

fn install_initial_legacy_twin_assignment(
    root: &crate::services::twin_events::AnchoredRoot,
    assignment: &LegacyTwinAssignmentV1,
    marker_install_hook: &mut impl FnMut(),
) -> Result<()> {
    let bytes = encoded_legacy_twin_assignment(assignment)?;
    marker_install_hook();
    root.install_no_clobber(
        LEGACY_TWIN_ASSIGNMENT_KEY,
        LEGACY_TWIN_ASSIGNMENT_STAGING_KEY,
        &bytes,
    )
    .map_err(anyhow::Error::new)?;
    let durable = root
        .read_bounded(LEGACY_TWIN_ASSIGNMENT_KEY, LEGACY_TWIN_ASSIGNMENT_LIMIT)
        .map_err(anyhow::Error::new)?
        .ok_or_else(|| anyhow::anyhow!("legacy Twin assignment disappeared after install"))?;
    let durable: LegacyTwinAssignmentV1 =
        serde_json::from_slice(&durable).context("invalid legacy Twin assignment")?;
    if &durable != assignment {
        anyhow::bail!("legacy Twin assignment was concurrently installed by another authority");
    }
    Ok(())
}

fn encoded_legacy_twin_assignment(assignment: &LegacyTwinAssignmentV1) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(assignment)?;
    bytes.push(b'\n');
    if bytes.len() > LEGACY_TWIN_ASSIGNMENT_LIMIT {
        anyhow::bail!("legacy Twin assignment exceeds its size limit");
    }
    Ok(bytes)
}

/// Service for managing user settings
#[derive(Clone)]
pub struct SettingsService {
    config_path: PathBuf,
    data_path: PathBuf,
    settings: UserSettings,
    key_source: crate::services::root_transition::OpenRouterKeySource,
    active_key_version: Option<String>,
    environment_runtime_secret: bool,
    secret_store: Arc<dyn crate::services::sync::secrets::SecretStore>,
}

impl SettingsService {
    #[cfg(feature = "mcp")]
    pub(crate) fn recover_root_transition_at(data_path: &Path) -> Result<()> {
        let config_dir = dirs::config_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grafyn");
        std::fs::create_dir_all(&config_dir).context("Failed to create Grafyn config directory")?;
        std::fs::create_dir_all(data_path).context("Failed to create Grafyn data directory")?;
        let store = crate::services::root_transition::RootTransitionStore::new(
            data_path,
            config_dir.join("settings.json"),
            Arc::new(crate::services::sync::secrets::KeyringSecretStore),
        )
        .map_err(anyhow::Error::new)?;
        store.recover().map_err(anyhow::Error::new)?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_test(config_path: PathBuf, settings: UserSettings) -> Self {
        let data_path = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("data");
        std::fs::create_dir_all(&data_path).expect("test data directory");
        Self {
            config_path,
            data_path,
            settings,
            key_source: crate::services::root_transition::OpenRouterKeySource::Unset,
            active_key_version: None,
            environment_runtime_secret: false,
            secret_store: Arc::new(
                crate::services::root_transition::MemoryVersionedSecretStore::default(),
            ),
        }
    }

    /// Create a SettingsService with default settings (used as fallback)
    pub fn load_defaults() -> Self {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::data_local_dir())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grafyn");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            log::error!(
                "Failed to create config directory {}: {}",
                config_dir.display(),
                e
            );
        }

        Self {
            config_path: config_dir.join("settings.json"),
            data_path: UserSettings::default().effective_data_path(),
            settings: UserSettings::default(),
            key_source: crate::services::root_transition::OpenRouterKeySource::Unset,
            active_key_version: None,
            environment_runtime_secret: false,
            secret_store: Arc::new(crate::services::sync::secrets::KeyringSecretStore),
        }
    }

    /// Load settings from disk or create defaults
    pub fn load() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::data_local_dir())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Grafyn");

        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            log::error!(
                "Failed to create config directory {}: {}",
                config_dir.display(),
                e
            );
        }
        let config_path = config_dir.join("settings.json");
        let data_path = UserSettings::default().effective_data_path();
        std::fs::create_dir_all(&data_path).context("Failed to create Grafyn data directory")?;
        let secret_store: Arc<dyn crate::services::sync::secrets::SecretStore> =
            Arc::new(crate::services::sync::secrets::KeyringSecretStore);
        let transition_store = crate::services::root_transition::RootTransitionStore::new(
            &data_path,
            &config_path,
            secret_store.clone(),
        )
        .map_err(anyhow::Error::new)?;
        let startup = transition_store
            .load_startup_settings(
                || {
                    load_openrouter_api_key().map_err(|error| {
                        crate::services::twin_events::MutationError::Io(error.to_string())
                    })
                },
                || {
                    clear_openrouter_api_key().map_err(|error| {
                        crate::services::twin_events::MutationError::Io(error.to_string())
                    })
                },
            )
            .map_err(anyhow::Error::new)?;
        let settings = startup.settings;
        let key_source = startup.key_source;
        let active_key_version = startup.active_key_version;

        Ok(Self {
            config_path,
            data_path,
            settings,
            key_source,
            active_key_version,
            environment_runtime_secret: false,
            secret_store,
        })
    }

    /// Get current settings
    pub fn get(&self) -> &UserSettings {
        &self.settings
    }

    pub(crate) fn secret_store(&self) -> Arc<dyn crate::services::sync::secrets::SecretStore> {
        self.secret_store.clone()
    }

    /// Get settings status for frontend
    pub fn status(&self) -> SettingsStatus {
        SettingsStatus::from(&self.settings)
    }

    /// Update settings and persist to disk
    #[cfg(test)]
    pub fn update(&mut self, update: SettingsUpdate) -> Result<UserSettings> {
        if update.openrouter_api_key.is_some() {
            anyhow::bail!("OpenRouter key updates require the coordinated settings boundary");
        }
        if update.vault_path.is_some() {
            anyhow::bail!("vault changes require the coordinated settings boundary");
        }
        let environment_runtime_secret = self.environment_runtime_secret();
        let snapshot = self
            .root_transition_store()?
            .patch_settings_guarded(|fresh| {
                apply_update_fields(fresh, &update).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
            })
            .map_err(anyhow::Error::new)?;
        self.settings = snapshot.settings;
        self.key_source = snapshot.key_source;
        if snapshot.key_source == crate::services::root_transition::OpenRouterKeySource::Unset {
            self.settings.openrouter_api_key = environment_runtime_secret;
        }
        self.active_key_version = snapshot.active_key_version;
        Ok(self.settings.clone())
    }

    pub(crate) fn root_transition_store(
        &self,
    ) -> Result<crate::services::root_transition::RootTransitionStore> {
        crate::services::root_transition::RootTransitionStore::new(
            &self.data_path,
            &self.config_path,
            self.secret_store.clone(),
        )
        .map_err(anyhow::Error::new)
    }

    pub(crate) fn publish_runtime_authority(
        &mut self,
        mut settings: UserSettings,
        key_source: crate::services::root_transition::OpenRouterKeySource,
        active_key_version: Option<String>,
        resolved_secret: Option<String>,
    ) {
        settings.openrouter_api_key = resolved_secret;
        self.settings = settings;
        self.key_source = key_source;
        self.active_key_version = active_key_version;
        self.environment_runtime_secret = false;
    }

    pub(crate) fn adopt_environment_runtime_secret(&mut self, secret: String) {
        if self.key_source == crate::services::root_transition::OpenRouterKeySource::Unset
            && self.active_key_version.is_none()
            && !secret.is_empty()
        {
            self.settings.openrouter_api_key = Some(secret);
            self.environment_runtime_secret = true;
        }
    }

    pub(crate) fn environment_runtime_secret(&self) -> Option<String> {
        self.environment_runtime_secret
            .then(|| self.settings.openrouter_api_key.clone())
            .flatten()
    }

    pub(crate) fn allows_environment_fallback(&self) -> bool {
        self.key_source == crate::services::root_transition::OpenRouterKeySource::Unset
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

    /// Check if MCP sidecar is enabled in settings
    pub fn mcp_enabled(&self) -> bool {
        self.settings.mcp_enabled
    }
}

#[cfg(test)]
fn choose_legacy_authority(keychain: Option<String>, plaintext: Option<String>) -> Option<String> {
    keychain.or(plaintext)
}

pub(crate) fn apply_update_fields(
    settings: &mut UserSettings,
    update: &SettingsUpdate,
) -> Result<()> {
    if let Some(vault_path) = update.vault_path.as_deref() {
        let path = PathBuf::from(vault_path);
        crate::services::twin_events::validate_real_directory(&path, "vault directory")
            .map_err(anyhow::Error::new)?;
        settings.vault_path = Some(
            std::fs::canonicalize(path)
                .context("Failed to canonicalize vault directory")?
                .to_string_lossy()
                .into_owned(),
        );
    }
    if let Some(api_key) = update.openrouter_api_key.as_deref() {
        settings.openrouter_api_key = (!api_key.is_empty()).then(|| api_key.to_string());
    }
    if let Some(value) = update.setup_completed {
        settings.setup_completed = value;
    }
    if let Some(value) = &update.theme {
        settings.theme.clone_from(value);
    }
    if let Some(value) = update.mcp_enabled {
        settings.mcp_enabled = value;
    }
    if let Some(value) = &update.llm_model {
        settings.llm_model = if value.is_empty() {
            crate::models::settings::default_llm_model()
        } else {
            value.clone()
        };
    }
    if let Some(value) = &update.twin_llm_provider {
        settings.twin_llm_provider = match value.trim().to_ascii_lowercase().as_str() {
            "ollama" => "ollama".to_string(),
            _ => "openrouter".to_string(),
        };
    }
    if let Some(value) = &update.ollama_base_url {
        let trimmed = value.trim().trim_end_matches('/').to_string();
        settings.ollama_base_url = if trimmed.is_empty() {
            "http://localhost:11434".to_string()
        } else {
            trimmed
        };
    }
    if let Some(value) = &update.ollama_model {
        settings.ollama_model = value.trim().to_string();
    }
    if let Some(value) = update.smart_web_search {
        settings.smart_web_search = value;
    }
    if let Some(value) = update.background_link_discovery_enabled {
        settings.background_link_discovery_enabled = value;
    }
    if let Some(value) = update.background_link_discovery_llm_enabled {
        settings.background_link_discovery_llm_enabled = value;
    }
    if let Some(value) = update.background_vault_optimizer_enabled {
        settings.background_vault_optimizer_enabled = value;
    }
    if let Some(value) = update.background_vault_optimizer_llm_enabled {
        settings.background_vault_optimizer_llm_enabled = value;
    }
    if let Some(value) = update.background_vault_optimizer_budget_monthly {
        settings.background_vault_optimizer_budget_monthly = value;
    }
    if let Some(value) = update.background_vault_optimizer_max_daily_writes {
        settings.background_vault_optimizer_max_daily_writes = value.max(1);
    }
    if let Some(value) = &update.background_vault_optimizer_edit_mode {
        settings.background_vault_optimizer_edit_mode = if value.trim().is_empty() {
            "sidecar_first".to_string()
        } else {
            value.clone()
        };
    }
    if let Some(value) = update.background_vault_optimizer_program_enabled {
        settings.background_vault_optimizer_program_enabled = value;
    }
    if let Some(value) = &update.vault_optimizer_program_path {
        settings.vault_optimizer_program_path = if value.trim().is_empty() {
            "_grafyn/program.md".to_string()
        } else {
            value.replace('\\', "/")
        };
    }
    if let Some(value) = &update.canvas_model_presets {
        settings.canvas_model_presets.clone_from(value);
    }
    Ok(())
}

/// Load settings from `config_path`. If the file doesn't exist, returns defaults. If it
/// exists but fails to parse (truncated write, hand edit gone wrong, etc.), the corrupt
/// file is quarantined — renamed to `settings.json.corrupt-{unix-timestamp}` beside it —
/// so the original bytes are never lost, an error is logged, and defaults are returned.
///
/// This is deliberately different from a bare `unwrap_or_default()`: without
/// quarantining, a corrupt file would (a) silently reset every setting including the
/// vault path, re-triggering first-run setup, and (b) get permanently overwritten by the
/// very next `save()`, destroying any chance of recovering the original content.
///
/// Only parse failures are quarantined. An I/O read error (e.g. permissions) is
/// propagated as before, since the file itself may be perfectly fine.
#[cfg(test)]
fn load_settings_from_file(config_path: &Path) -> Result<UserSettings> {
    if !config_path.exists() {
        return Ok(UserSettings::default());
    }

    let content = std::fs::read_to_string(config_path).context("Failed to read settings file")?;

    match serde_json::from_str(&content) {
        Ok(settings) => Ok(settings),
        Err(parse_error) => {
            log::error!(
                "Settings file {} is corrupt ({}); quarantining and falling back to defaults",
                config_path.display(),
                parse_error
            );
            quarantine_corrupt_file(config_path);
            Ok(UserSettings::default())
        }
    }
}

/// Rename a corrupt file to `{name}.corrupt-{unix-timestamp}` in the same directory.
/// Best-effort: if the rename itself fails, log and leave the file in place.
#[cfg(test)]
fn quarantine_corrupt_file(path: &Path) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    let quarantine_path = path.with_file_name(format!("{file_name}.corrupt-{timestamp}"));

    match std::fs::rename(path, &quarantine_path) {
        Ok(()) => {
            log::error!(
                "Quarantined corrupt file {} to {}",
                path.display(),
                quarantine_path.display()
            );
        }
        Err(e) => {
            log::error!(
                "Failed to quarantine corrupt file {} to {}: {}",
                path.display(),
                quarantine_path.display(),
                e
            );
        }
    }
}

fn keyring_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, OPENROUTER_KEY_ACCOUNT)
        .context("Failed to initialize OS keychain entry")
}

fn load_openrouter_api_key() -> Result<Option<String>> {
    let entry = keyring_entry()?;
    match entry.get_password() {
        Ok(password) if !password.is_empty() => Ok(Some(password)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => {
            Err(anyhow::Error::new(error).context("Failed to read legacy OpenRouter key"))
        }
    }
}

fn clear_openrouter_api_key() -> Result<()> {
    let entry = keyring_entry()?;
    match entry.delete_password() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => {
            Err(anyhow::Error::new(error).context("Failed to delete legacy OpenRouter API key"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::CanvasModelPreset;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn legacy_twin_path(data: &Path, vault: &Path) -> PathBuf {
        let normalized = vault
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let normalized = normalized.strip_prefix("//?/").unwrap_or(&normalized);
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in normalized.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        data.join("twin").join(format!("{hash:016x}"))
    }

    #[test]
    fn legacy_twin_namespace_moves_once_without_merge() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(data.join("twin")).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let legacy = legacy_twin_path(&data, &vault);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("record.json"), "legacy").unwrap();

        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();

        #[cfg(not(windows))]
        {
            assert!(guard.prepare_twin_data_path(&vault, &lease).is_err());
            return;
        }
        #[cfg(windows)]
        let current = guard.prepare_twin_data_path(&vault, &lease).unwrap();
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(current.join("record.json")).unwrap(),
            "legacy"
        );

        std::fs::create_dir(&legacy).unwrap();
        assert!(guard.prepare_twin_data_path(&vault, &lease).is_err());
        assert!(legacy.exists());
        assert!(current.exists());
    }

    #[cfg(windows)]
    #[test]
    fn legacy_twin_assignment_preserves_a_raced_destination() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(data.join("twin")).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let legacy = legacy_twin_path(&data, &vault);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("record.json"), "legacy").unwrap();

        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        drop(guard);
        drop(coordinator);
        let current = crate::models::settings::twin_data_path_for_scope(&data, &lease.root_scope);
        let lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&data).unwrap();

        let error =
            prepare_twin_data_path_locked_with_rename_hook(&data, &vault, &lease, &lock, || {
                std::fs::create_dir(&current).unwrap();
                std::fs::write(current.join("record.json"), "foreign").unwrap();
            })
            .unwrap_err();

        assert!(error.to_string().contains("no-replace rename failed"));
        assert_eq!(
            std::fs::read_to_string(legacy.join("record.json")).unwrap(),
            "legacy"
        );
        assert_eq!(
            std::fs::read_to_string(current.join("record.json")).unwrap(),
            "foreign"
        );
        lock.unlock().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn legacy_twin_assignment_preserves_a_raced_marker() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(data.join("twin")).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let legacy = legacy_twin_path(&data, &vault);
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("record.json"), "legacy").unwrap();

        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        drop(guard);
        drop(coordinator);
        let marker = data.join(LEGACY_TWIN_ASSIGNMENT_KEY.replace('/', "\\"));
        let lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&data).unwrap();

        let error = prepare_twin_data_path_locked_with_marker_install_hook(
            &data,
            &vault,
            &lease,
            &lock,
            || std::fs::write(&marker, b"foreign-marker").unwrap(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid legacy Twin assignment"));
        assert_eq!(std::fs::read(&marker).unwrap(), b"foreign-marker");
        assert_eq!(
            std::fs::read_to_string(legacy.join("record.json")).unwrap(),
            "legacy"
        );
        lock.unlock().unwrap();
    }

    #[test]
    fn prepared_legacy_twin_assignment_rejects_neither_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(data.join("twin")).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        let current = crate::models::settings::twin_data_path_for_scope(&data, &lease.root_scope);
        let legacy = legacy_twin_path(&data, &vault);
        let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
        write_legacy_twin_assignment(
            &root,
            &LegacyTwinAssignmentV1 {
                schema_version: 1,
                root_scope: lease.root_scope.clone(),
                lease_epoch_uuid: lease.epoch_uuid.clone(),
                legacy_name: legacy.file_name().unwrap().to_str().unwrap().into(),
                current_name: current.file_name().unwrap().to_str().unwrap().into(),
                state: LegacyTwinAssignmentState::Prepared,
            },
        )
        .unwrap();

        let error = guard.prepare_twin_data_path(&vault, &lease).unwrap_err();

        assert!(error.to_string().contains("neither"));
        let marker: LegacyTwinAssignmentV1 = serde_json::from_slice(
            &root
                .read_bounded(LEGACY_TWIN_ASSIGNMENT_KEY, LEGACY_TWIN_ASSIGNMENT_LIMIT)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(marker.state, LegacyTwinAssignmentState::Prepared);
    }

    #[test]
    fn prepared_legacy_twin_assignment_rejects_foreign_authority() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        let foreign_vault = temp.path().join("foreign-vault");
        std::fs::create_dir_all(data.join("twin")).unwrap();
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&foreign_vault).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        let current = crate::models::settings::twin_data_path_for_scope(&data, &lease.root_scope);
        let legacy = legacy_twin_path(&data, &vault);
        let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
        write_legacy_twin_assignment(
            &root,
            &LegacyTwinAssignmentV1 {
                schema_version: 1,
                root_scope: crate::services::twin_events::root_identity_for_path(&foreign_vault)
                    .unwrap(),
                lease_epoch_uuid: lease.epoch_uuid.clone(),
                legacy_name: legacy.file_name().unwrap().to_str().unwrap().into(),
                current_name: current.file_name().unwrap().to_str().unwrap().into(),
                state: LegacyTwinAssignmentState::Prepared,
            },
        )
        .unwrap();

        let error = guard.prepare_twin_data_path(&vault, &lease).unwrap_err();
        assert!(error.to_string().contains("another root authority"));
    }

    fn vault_update(path: impl Into<String>) -> SettingsUpdate {
        SettingsUpdate {
            vault_path: Some(path.into()),
            openrouter_api_key: None,
            setup_completed: None,
            theme: None,
            mcp_enabled: None,
            llm_model: None,
            twin_llm_provider: None,
            ollama_base_url: None,
            ollama_model: None,
            smart_web_search: None,
            background_link_discovery_enabled: None,
            background_link_discovery_llm_enabled: None,
            background_vault_optimizer_enabled: None,
            background_vault_optimizer_llm_enabled: None,
            background_vault_optimizer_budget_monthly: None,
            background_vault_optimizer_max_daily_writes: None,
            background_vault_optimizer_edit_mode: None,
            background_vault_optimizer_program_enabled: None,
            vault_optimizer_program_path: None,
            canvas_model_presets: None,
        }
    }

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

        let mut service = SettingsService::for_test(config_path.clone(), UserSettings::default());

        let presets = vec![CanvasModelPreset {
            id: "preset-1".to_string(),
            name: "Fast trio".to_string(),
            model_ids: vec![
                "openai/gpt-4o".to_string(),
                "anthropic/claude-3.5-sonnet".to_string(),
            ],
        }];

        let updated = service
            .update(SettingsUpdate {
                vault_path: None,
                openrouter_api_key: None,
                setup_completed: None,
                theme: None,
                mcp_enabled: None,
                llm_model: None,
                twin_llm_provider: None,
                ollama_base_url: None,
                ollama_model: None,
                smart_web_search: None,
                background_link_discovery_enabled: None,
                background_link_discovery_llm_enabled: None,
                background_vault_optimizer_enabled: None,
                background_vault_optimizer_llm_enabled: None,
                background_vault_optimizer_budget_monthly: None,
                background_vault_optimizer_max_daily_writes: None,
                background_vault_optimizer_edit_mode: None,
                background_vault_optimizer_program_enabled: None,
                vault_optimizer_program_path: None,
                canvas_model_presets: Some(presets.clone()),
            })
            .expect("settings update should succeed");

        assert_eq!(updated.canvas_model_presets, presets);

        let persisted = std::fs::read_to_string(config_path).expect("settings file should exist");
        assert!(persisted.contains("\"canvas_model_presets\""));
        assert!(persisted.contains("\"Fast trio\""));

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn settings_writes_are_atomic_with_no_tmp_litter() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("settings.json");

        let mut service = SettingsService::for_test(config_path.clone(), UserSettings::default());

        service
            .update(SettingsUpdate {
                vault_path: None,
                openrouter_api_key: None,
                setup_completed: None,
                theme: Some("dark".to_string()),
                mcp_enabled: None,
                llm_model: None,
                twin_llm_provider: None,
                ollama_base_url: None,
                ollama_model: None,
                smart_web_search: None,
                background_link_discovery_enabled: None,
                background_link_discovery_llm_enabled: None,
                background_vault_optimizer_enabled: None,
                background_vault_optimizer_llm_enabled: None,
                background_vault_optimizer_budget_monthly: None,
                background_vault_optimizer_max_daily_writes: None,
                background_vault_optimizer_edit_mode: None,
                background_vault_optimizer_program_enabled: None,
                vault_optimizer_program_path: None,
                canvas_model_presets: None,
            })
            .expect("settings update should succeed");

        let persisted = std::fs::read_to_string(&config_path).expect("settings file should exist");
        assert!(persisted.contains("\"theme\": \"dark\""));
        crate::services::atomic_io::assert_no_tmp_siblings(temp_dir.path());
    }

    #[test]
    fn vault_update_requires_an_existing_real_directory() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("settings.json");
        let mut service = SettingsService::for_test(config_path, UserSettings::default());
        let missing = temp.path().join("missing");
        assert!(service
            .update(vault_update(missing.to_string_lossy()))
            .is_err());
        assert!(!missing.exists());

        let regular_file = temp.path().join("not-a-vault");
        std::fs::write(&regular_file, b"not a directory").unwrap();
        assert!(service
            .update(vault_update(regular_file.to_string_lossy()))
            .is_err());

        let real = temp.path().join("real-vault");
        let link = temp.path().join("linked-vault");
        std::fs::create_dir(&real).unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&real, &link);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&real, &link);
        if linked.is_ok() {
            assert!(service
                .update(vault_update(link.to_string_lossy()))
                .is_err());
        }
    }

    #[test]
    fn ordinary_settings_service_update_rejects_even_a_valid_vault_change() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("settings.json");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let before = UserSettings::default();
        let mut service = SettingsService::for_test(config_path, before.clone());

        let error = service
            .update(vault_update(vault.to_string_lossy()))
            .expect_err("vault changes must use the coordinated command boundary");
        assert!(error.to_string().contains("coordinated settings boundary"));
        assert_eq!(service.get().vault_path, before.vault_path);
        assert_eq!(service.get().theme, before.theme);
    }

    #[test]
    fn legacy_secret_authority_prefers_keychain_over_stale_plaintext() {
        assert_eq!(
            choose_legacy_authority(Some("new-keychain".into()), Some("stale-file".into())),
            Some("new-keychain".into())
        );
        assert_eq!(
            choose_legacy_authority(None, Some("file-fallback".into())),
            Some("file-fallback".into())
        );
    }

    #[test]
    fn failed_settings_persistence_does_not_publish_runtime_values() {
        let temp = tempfile::tempdir().unwrap();
        let before = UserSettings::default();
        let mut service = SettingsService::for_test(temp.path().to_path_buf(), before.clone());
        let mut update = vault_update(temp.path().to_string_lossy());
        update.vault_path = None;
        update.theme = Some("dark".into());
        assert!(service.update(update).is_err());
        assert_eq!(service.get().theme, before.theme);
    }

    #[test]
    fn corrupt_settings_file_is_quarantined_and_defaults_returned() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("settings.json");
        let corrupt_bytes = b"{ this is not valid json at all";
        std::fs::write(&config_path, corrupt_bytes).expect("seed corrupt settings file");

        let settings =
            load_settings_from_file(&config_path).expect("should fall back to defaults, not error");

        // Defaults returned, not an error and not a crash.
        assert_eq!(settings.vault_path, UserSettings::default().vault_path);
        assert!(settings.needs_setup());

        // The original file is gone from its normal location...
        assert!(
            !config_path.exists(),
            "corrupt file should be moved out of the way, not left in place"
        );

        // ...but quarantined as a sibling with the original bytes intact.
        let quarantined: Vec<_> = std::fs::read_dir(temp_dir.path())
            .expect("read temp dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("settings.json.corrupt-"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            quarantined.len(),
            1,
            "expected exactly one quarantined sibling file"
        );
        let quarantined_content =
            std::fs::read(&quarantined[0]).expect("quarantined file should be readable");
        assert_eq!(quarantined_content, corrupt_bytes);
    }

    #[test]
    fn valid_settings_file_loads_normally() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("settings.json");

        let original = UserSettings {
            theme: "dark".to_string(),
            vault_path: Some("/test/vault".to_string()),
            ..UserSettings::default()
        };
        std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&original).expect("serialize settings"),
        )
        .expect("seed valid settings file");

        let loaded = load_settings_from_file(&config_path).expect("valid settings should load");
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.vault_path, Some("/test/vault".to_string()));

        // Nothing should be quarantined for a healthy file.
        let quarantined = std::fs::read_dir(temp_dir.path())
            .expect("read temp dir")
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains(".corrupt-"))
                    .unwrap_or(false)
            });
        assert!(!quarantined, "valid file should not be quarantined");
    }

    #[test]
    fn missing_settings_file_returns_defaults_without_quarantine() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("settings.json");

        let settings = load_settings_from_file(&config_path).expect("missing file is not an error");
        assert!(settings.needs_setup());
    }
}
