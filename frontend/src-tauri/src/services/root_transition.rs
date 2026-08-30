use crate::models::settings::{CanvasModelPreset, UserSettings};
use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{
    root_identity_for_path, ActiveMarkdownRootLeaseV1, AnchoredRoot, MutationError,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub(crate) const ROOT_TRANSITION_LIMIT: usize = 256 * 1024;
const SETTINGS_LIMIT: usize = 1024 * 1024;
const KEY_REF_LIMIT: usize = 4096;
const ROOT_TRANSITION_KEY: &str = "twin/events/root-transition-v1.json";
const ACTIVE_ROOT_LEASE_KEY: &str = "twin/events/active-markdown-root-v1.json";
const OPENROUTER_KEY_REF: &str = "twin/events/openrouter-key-ref-v1.json";
const ROOT_TRANSITION_SCHEMA_VERSION: u16 = 1;
const KEY_REF_SCHEMA_VERSION: u16 = 1;
const LEASE_SCHEMA_VERSION: u16 = 1;
const KEYRING_SERVICE: &str = "com.grafyn.app";
const VERSIONED_KEY_PREFIX: &str = "openrouter_api_key/";
const LEGACY_MIGRATION_KEY_VERSION: &str = "00000000-0000-4000-8000-000000000001";

#[cfg(feature = "mcp")]
pub(crate) fn reject_custom_transition_wal_locked(
    data_path: &Path,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if !process_lock.covers_data_path(data_path)? {
        return Err(MutationError::Invalid(
            "custom MCP transition check lock belongs to another data root".into(),
        ));
    }
    let root = AnchoredRoot::open(data_path)?;
    if root
        .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
        .is_some()
    {
        return Err(MutationError::RecoveryConflict(
            "custom-mcp-root-transition-requires-default-authority-recovery".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NonsecretSettingsV1 {
    pub(crate) vault_path: Option<String>,
    pub(crate) setup_completed: bool,
    pub(crate) theme: String,
    pub(crate) mcp_enabled: bool,
    pub(crate) llm_model: String,
    pub(crate) twin_llm_provider: String,
    pub(crate) ollama_base_url: String,
    pub(crate) ollama_model: String,
    pub(crate) smart_web_search: bool,
    pub(crate) background_link_discovery_enabled: bool,
    pub(crate) background_link_discovery_llm_enabled: bool,
    pub(crate) background_vault_optimizer_enabled: bool,
    pub(crate) background_vault_optimizer_llm_enabled: bool,
    pub(crate) background_vault_optimizer_budget_monthly: u32,
    pub(crate) background_vault_optimizer_max_daily_writes: u32,
    pub(crate) background_vault_optimizer_edit_mode: String,
    pub(crate) background_vault_optimizer_program_enabled: bool,
    pub(crate) vault_optimizer_program_path: String,
    pub(crate) canvas_model_presets: Vec<CanvasModelPreset>,
}

impl NonsecretSettingsV1 {
    pub(crate) fn from_settings(mut settings: UserSettings) -> Self {
        settings.openrouter_api_key = None;
        Self {
            vault_path: settings.vault_path,
            setup_completed: settings.setup_completed,
            theme: settings.theme,
            mcp_enabled: settings.mcp_enabled,
            llm_model: settings.llm_model,
            twin_llm_provider: settings.twin_llm_provider,
            ollama_base_url: settings.ollama_base_url,
            ollama_model: settings.ollama_model,
            smart_web_search: settings.smart_web_search,
            background_link_discovery_enabled: settings.background_link_discovery_enabled,
            background_link_discovery_llm_enabled: settings.background_link_discovery_llm_enabled,
            background_vault_optimizer_enabled: settings.background_vault_optimizer_enabled,
            background_vault_optimizer_llm_enabled: settings.background_vault_optimizer_llm_enabled,
            background_vault_optimizer_budget_monthly: settings
                .background_vault_optimizer_budget_monthly,
            background_vault_optimizer_max_daily_writes: settings
                .background_vault_optimizer_max_daily_writes,
            background_vault_optimizer_edit_mode: settings.background_vault_optimizer_edit_mode,
            background_vault_optimizer_program_enabled: settings
                .background_vault_optimizer_program_enabled,
            vault_optimizer_program_path: settings.vault_optimizer_program_path,
            canvas_model_presets: settings.canvas_model_presets,
        }
    }

    pub(crate) fn into_settings(self) -> UserSettings {
        UserSettings {
            vault_path: self.vault_path,
            openrouter_api_key: None,
            setup_completed: self.setup_completed,
            theme: self.theme,
            mcp_enabled: self.mcp_enabled,
            llm_model: self.llm_model,
            twin_llm_provider: self.twin_llm_provider,
            ollama_base_url: self.ollama_base_url,
            ollama_model: self.ollama_model,
            smart_web_search: self.smart_web_search,
            background_link_discovery_enabled: self.background_link_discovery_enabled,
            background_link_discovery_llm_enabled: self.background_link_discovery_llm_enabled,
            background_vault_optimizer_enabled: self.background_vault_optimizer_enabled,
            background_vault_optimizer_llm_enabled: self.background_vault_optimizer_llm_enabled,
            background_vault_optimizer_budget_monthly: self
                .background_vault_optimizer_budget_monthly,
            background_vault_optimizer_max_daily_writes: self
                .background_vault_optimizer_max_daily_writes,
            background_vault_optimizer_edit_mode: self.background_vault_optimizer_edit_mode,
            background_vault_optimizer_program_enabled: self
                .background_vault_optimizer_program_enabled,
            vault_optimizer_program_path: self.vault_optimizer_program_path,
            canvas_model_presets: self.canvas_model_presets,
        }
    }

    fn validate(&self) -> Result<(), MutationError> {
        let bounded = [
            self.vault_path.as_deref().unwrap_or_default(),
            &self.theme,
            &self.llm_model,
            &self.twin_llm_provider,
            &self.ollama_base_url,
            &self.ollama_model,
            &self.background_vault_optimizer_edit_mode,
            &self.vault_optimizer_program_path,
        ];
        if bounded.iter().any(|value| value.len() > 32 * 1024)
            || self.canvas_model_presets.len() > 64
            || self.canvas_model_presets.iter().any(|preset| {
                preset.id.len() > 512
                    || preset.name.len() > 512
                    || preset.model_ids.len() > 64
                    || preset.model_ids.iter().any(|model| model.len() > 512)
            })
        {
            return Err(MutationError::Invalid(
                "root transition settings exceed their bounds".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootAuthorityV1 {
    pub(crate) canonical_vault_path: String,
    pub(crate) root_scope: ContentDigest,
    pub(crate) lease: ActiveMarkdownRootLeaseV1,
    pub(crate) nonsecret_settings: NonsecretSettingsV1,
    pub(crate) openrouter_key_source: OpenRouterKeySource,
    pub(crate) openrouter_key_version: Option<String>,
}

impl RootAuthorityV1 {
    #[cfg(test)]
    pub(crate) fn new(
        vault_path: &Path,
        settings: UserSettings,
        lease: ActiveMarkdownRootLeaseV1,
        openrouter_key_version: Option<String>,
    ) -> Result<Self, MutationError> {
        let key_source = if openrouter_key_version.is_some() {
            OpenRouterKeySource::Versioned
        } else {
            OpenRouterKeySource::Unset
        };
        Self::new_with_key_source(
            vault_path,
            settings,
            lease,
            key_source,
            openrouter_key_version,
        )
    }

    pub(crate) fn new_with_key_source(
        vault_path: &Path,
        settings: UserSettings,
        lease: ActiveMarkdownRootLeaseV1,
        openrouter_key_source: OpenRouterKeySource,
        openrouter_key_version: Option<String>,
    ) -> Result<Self, MutationError> {
        let canonical = std::fs::canonicalize(vault_path)?;
        let canonical_vault_path = canonical
            .to_str()
            .ok_or_else(|| MutationError::Invalid("canonical vault path must be Unicode".into()))?
            .to_string();
        let root_scope = root_identity_for_path(&canonical)?;
        let authority = Self {
            canonical_vault_path,
            root_scope,
            lease,
            nonsecret_settings: NonsecretSettingsV1::from_settings(settings),
            openrouter_key_source,
            openrouter_key_version,
        };
        authority.validate()?;
        Ok(authority)
    }

    fn validate(&self) -> Result<(), MutationError> {
        self.nonsecret_settings.validate()?;
        if self.lease.schema_version != LEASE_SCHEMA_VERSION
            || self.lease.root_scope != self.root_scope
            || Uuid::parse_str(&self.lease.epoch_uuid).is_err()
        {
            return Err(MutationError::Invalid(
                "root transition contains an invalid lease".into(),
            ));
        }
        validate_key_version(self.openrouter_key_version.as_deref())?;
        validate_key_authority(
            self.openrouter_key_source,
            self.openrouter_key_version.as_deref(),
        )?;
        let canonical = std::fs::canonicalize(&self.canonical_vault_path)?;
        let settings_vault = std::fs::canonicalize(
            self.nonsecret_settings
                .clone()
                .into_settings()
                .effective_vault_path(),
        )?;
        if canonical.to_string_lossy() != self.canonical_vault_path
            || root_identity_for_path(&canonical)? != self.root_scope
            || settings_vault != canonical
        {
            return Err(MutationError::Invalid(
                "root transition vault identity is inconsistent".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OpenRouterKeySource {
    Unset,
    Versioned,
    Cleared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootTransitionDecision {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootTransitionV1 {
    pub(crate) schema_version: u16,
    pub(crate) transaction_id: String,
    pub(crate) decision: RootTransitionDecision,
    pub(crate) authority_binding: ContentDigest,
    pub(crate) root_changed: bool,
    pub(crate) before: RootAuthorityV1,
    pub(crate) after: RootAuthorityV1,
    pub(crate) rollback_lease: ActiveMarkdownRootLeaseV1,
}

impl RootTransitionV1 {
    pub(crate) fn prepared(
        before: RootAuthorityV1,
        after: RootAuthorityV1,
        authority_binding: ContentDigest,
    ) -> Result<Self, MutationError> {
        let rollback_lease = ActiveMarkdownRootLeaseV1::new(before.root_scope.clone());
        let transition = Self {
            schema_version: ROOT_TRANSITION_SCHEMA_VERSION,
            transaction_id: Uuid::new_v4().to_string(),
            decision: RootTransitionDecision::Prepared,
            authority_binding,
            root_changed: before.root_scope != after.root_scope,
            before,
            after,
            rollback_lease,
        };
        transition.validate()?;
        Ok(transition)
    }

    fn validate(&self) -> Result<(), MutationError> {
        if self.schema_version != ROOT_TRANSITION_SCHEMA_VERSION
            || Uuid::parse_str(&self.transaction_id).is_err()
            || self.root_changed != (self.before.root_scope != self.after.root_scope)
            || self.rollback_lease.schema_version != LEASE_SCHEMA_VERSION
            || self.rollback_lease.root_scope != self.before.root_scope
            || Uuid::parse_str(&self.rollback_lease.epoch_uuid).is_err()
        {
            return Err(MutationError::Invalid(
                "invalid root transition metadata".into(),
            ));
        }
        self.before.validate()?;
        self.after.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenRouterKeyRefV1 {
    schema_version: u16,
    source: OpenRouterKeySource,
    active_version: Option<String>,
}

pub(crate) trait VersionedSecretStore: Send + Sync {
    fn put_new(&self, version: &str, secret: &str) -> Result<(), MutationError>;
    fn get(&self, version: &str) -> Result<Option<String>, MutationError>;
    fn delete(&self, version: &str) -> Result<(), MutationError>;
}

#[derive(Debug, Default)]
pub(crate) struct KeyringVersionedSecretStore;

impl VersionedSecretStore for KeyringVersionedSecretStore {
    fn put_new(&self, version: &str, secret: &str) -> Result<(), MutationError> {
        validate_key_version(Some(version))?;
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, &format!("{VERSIONED_KEY_PREFIX}{version}"))
                .map_err(|error| MutationError::Io(error.to_string()))?;
        match entry.get_password() {
            Ok(_) => {
                return Err(MutationError::Invalid(
                    "OpenRouter key version already exists".into(),
                ));
            }
            Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(MutationError::Io(error.to_string())),
        }
        entry
            .set_password(secret)
            .map_err(|error| MutationError::Io(error.to_string()))?;
        Ok(())
    }

    fn get(&self, version: &str) -> Result<Option<String>, MutationError> {
        validate_key_version(Some(version))?;
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, &format!("{VERSIONED_KEY_PREFIX}{version}"))
                .map_err(|error| MutationError::Io(error.to_string()))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(MutationError::Io(error.to_string())),
        }
    }

    fn delete(&self, version: &str) -> Result<(), MutationError> {
        validate_key_version(Some(version))?;
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, &format!("{VERSIONED_KEY_PREFIX}{version}"))
                .map_err(|error| MutationError::Io(error.to_string()))?;
        match entry.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(MutationError::Io(error.to_string())),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MemoryVersionedSecretStore {
    values: Mutex<BTreeMap<String, String>>,
}

#[cfg(test)]
impl VersionedSecretStore for MemoryVersionedSecretStore {
    fn put_new(&self, version: &str, secret: &str) -> Result<(), MutationError> {
        validate_key_version(Some(version))?;
        let mut values = self
            .values
            .lock()
            .map_err(|_| MutationError::Invalid("secret store lock poisoned".into()))?;
        if values.contains_key(version) {
            return Err(MutationError::Invalid(
                "secret version already exists".into(),
            ));
        }
        values.insert(version.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, version: &str) -> Result<Option<String>, MutationError> {
        Ok(self
            .values
            .lock()
            .map_err(|_| MutationError::Invalid("secret store lock poisoned".into()))?
            .get(version)
            .cloned())
    }

    fn delete(&self, version: &str) -> Result<(), MutationError> {
        self.values
            .lock()
            .map_err(|_| MutationError::Invalid("secret store lock poisoned".into()))?
            .remove(version);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryWork {
    None,
    RolledBack,
    RolledForward,
}

#[derive(Debug)]
pub(crate) enum MarkCommittedResult {
    Committed(Box<RootTransitionV1>),
    DefinitelyPrepared(MutationError),
    Uncertain(MutationError),
}

#[derive(Debug, Clone)]
pub(crate) struct DurableSettingsSnapshot {
    pub(crate) settings: UserSettings,
    pub(crate) settings_generation: ContentDigest,
    pub(crate) active_key_version: Option<String>,
    pub(crate) key_source: OpenRouterKeySource,
    pub(crate) resolved_secret: Option<String>,
}

#[derive(Clone)]
pub(crate) struct DurableRootAuthoritySnapshot {
    pub(crate) authority: RootAuthorityV1,
    pub(crate) settings: UserSettings,
    pub(crate) settings_generation: ContentDigest,
    pub(crate) resolved_secret: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootTransitionFaultPoint {
    AfterPrepared,
    AfterSecretStage,
    AfterLease,
    AfterSettings,
    AfterKeyRef,
    BeforeCommitWalWrite,
    AfterCommitWalWrite,
    CommitWalReadbackUnavailable,
    AfterCommitted,
    AfterOldKeyDelete,
    AfterWalDelete,
    AfterRollbackLease,
    AfterRollbackSettings,
    AfterRollbackKeyRef,
    AfterRollbackSecretDelete,
    AfterRecoveryWalDelete,
}

pub(crate) struct RootTransitionStore {
    data_path: std::path::PathBuf,
    data_root: AnchoredRoot,
    config_root: AnchoredRoot,
    settings_key: String,
    authority_binding: ContentDigest,
    secrets: Arc<dyn VersionedSecretStore>,
    fault_once: Mutex<Option<RootTransitionFaultPoint>>,
}

impl RootTransitionStore {
    pub(crate) fn new(
        data_path: impl AsRef<Path>,
        config_path: impl AsRef<Path>,
        secrets: Arc<dyn VersionedSecretStore>,
    ) -> Result<Self, MutationError> {
        let data_path = std::fs::canonicalize(data_path.as_ref())?;
        let data_root = AnchoredRoot::open(&data_path)?;
        data_root.open_directory("twin/events", true)?;
        let config_path = config_path.as_ref();
        let config_parent = config_path
            .parent()
            .ok_or_else(|| MutationError::Invalid("settings file has no parent".into()))?;
        let settings_key = config_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| MutationError::Invalid("settings filename must be UTF-8".into()))?
            .to_string();
        crate::services::twin_events::validate_relative_key(&settings_key)?;
        let config_root = AnchoredRoot::open(config_parent)?;
        let authority_binding = root_authority_binding(&data_path, config_parent, &settings_key)?;
        Ok(Self {
            data_path,
            data_root,
            config_root,
            settings_key,
            authority_binding,
            secrets,
            fault_once: Mutex::new(None),
        })
    }

    pub(crate) fn authority_binding(&self) -> ContentDigest {
        self.authority_binding.clone()
    }

    fn encoded_transition(&self, transition: &RootTransitionV1) -> Result<Vec<u8>, MutationError> {
        transition.validate()?;
        let mut bytes = serde_json::to_vec_pretty(transition)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > ROOT_TRANSITION_LIMIT {
            return Err(MutationError::Invalid(
                "root transition exceeds its size limit".into(),
            ));
        }
        Ok(bytes)
    }

    pub(crate) fn prepare_transition(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        if transition.decision != RootTransitionDecision::Prepared {
            return Err(MutationError::Invalid(
                "new root transition must be prepared".into(),
            ));
        }
        if transition.authority_binding != self.authority_binding {
            return Err(MutationError::RecoveryConflict(
                "root-transition-authority-binding-mismatch".into(),
            ));
        }
        let bytes = self.encoded_transition(transition)?;
        if self
            .data_root
            .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
            .is_some()
        {
            return Err(MutationError::RecoveryConflict(
                "root-transition-already-pending".into(),
            ));
        }
        self.data_root
            .install_no_clobber(ROOT_TRANSITION_KEY, "twin/events/staging-v1", &bytes)?;
        if self.read_transition()?.as_ref() != Some(transition) {
            return Err(MutationError::RecoveryConflict(
                "root-transition-create-collision".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn read_authority_locked(
        &self,
        process_lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<DurableRootAuthoritySnapshot, MutationError> {
        self.require_matching_process_lock(process_lock)?;
        if self.read_transition()?.is_some() {
            return Err(MutationError::RecoveryConflict(
                "root-transition-already-pending".into(),
            ));
        }
        let settings = self.read_settings_snapshot()?;
        let lease = self.read_lease()?;
        let authority = RootAuthorityV1::new_with_key_source(
            &settings.settings.effective_vault_path(),
            settings.settings.clone(),
            lease,
            settings.key_source,
            settings.active_key_version.clone(),
        )?;
        Ok(DurableRootAuthoritySnapshot {
            authority,
            settings: settings.settings,
            settings_generation: settings.settings_generation,
            resolved_secret: settings.resolved_secret,
        })
    }

    pub(crate) fn prepare_transition_cas_locked(
        &self,
        process_lock: &crate::services::twin_events::CoordinatorProcessLock,
        expected: &DurableRootAuthoritySnapshot,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        self.require_matching_process_lock(process_lock)?;
        if transition.before != expected.authority {
            return Err(MutationError::RecoveryConflict(
                "root-transition-before-authority-mismatch".into(),
            ));
        }
        let current = self.read_authority_locked(process_lock)?;
        if current.authority != expected.authority
            || current.settings_generation != expected.settings_generation
            || current.resolved_secret != expected.resolved_secret
        {
            return Err(MutationError::RecoveryConflict(
                "root-transition-authority-cas-failed".into(),
            ));
        }
        self.prepare_transition(transition)
    }

    fn require_matching_process_lock(
        &self,
        process_lock: &crate::services::twin_events::CoordinatorProcessLock,
    ) -> Result<(), MutationError> {
        if !process_lock.covers_data_path(&self.data_path)? {
            return Err(MutationError::Invalid(
                "root transition lock token belongs to another data root".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn mark_committed(&self, transition: &RootTransitionV1) -> MarkCommittedResult {
        if transition.decision != RootTransitionDecision::Prepared {
            return MarkCommittedResult::Uncertain(MutationError::RecoveryConflict(
                "root-transition-commit-cas-failed".into(),
            ));
        }
        match self.read_transition() {
            Ok(Some(durable)) if durable == *transition => {}
            Ok(_) => {
                return MarkCommittedResult::Uncertain(MutationError::RecoveryConflict(
                    "root-transition-commit-cas-failed".into(),
                ));
            }
            Err(error) => return MarkCommittedResult::Uncertain(error),
        }
        let mut committed = transition.clone();
        committed.decision = RootTransitionDecision::Committed;
        let bytes = match self.encoded_transition(&committed) {
            Ok(bytes) => bytes,
            Err(error) => return MarkCommittedResult::DefinitelyPrepared(error),
        };
        if let Err(error) = self.checkpoint(RootTransitionFaultPoint::BeforeCommitWalWrite) {
            return MarkCommittedResult::DefinitelyPrepared(error);
        }
        let write_error = self
            .data_root
            .put_atomic(ROOT_TRANSITION_KEY, &bytes)
            .err()
            .or_else(|| {
                self.checkpoint(RootTransitionFaultPoint::AfterCommitWalWrite)
                    .err()
            });
        if let Err(error) = self.checkpoint(RootTransitionFaultPoint::CommitWalReadbackUnavailable)
        {
            return MarkCommittedResult::Uncertain(write_error.unwrap_or(error));
        }
        match self.read_transition() {
            Ok(Some(durable)) if durable == committed => {
                MarkCommittedResult::Committed(Box::new(committed))
            }
            Ok(Some(durable)) if durable == *transition => {
                MarkCommittedResult::DefinitelyPrepared(write_error.unwrap_or_else(|| {
                    MutationError::RecoveryConflict(
                        "root-transition-commit-remained-prepared".into(),
                    )
                }))
            }
            Ok(_) => MarkCommittedResult::Uncertain(write_error.unwrap_or_else(|| {
                MutationError::RecoveryConflict("root-transition-commit-third-state".into())
            })),
            Err(read_error) => MarkCommittedResult::Uncertain(write_error.unwrap_or(read_error)),
        }
    }

    pub(crate) fn remove_transition(&self) -> Result<(), MutationError> {
        self.data_root.delete(ROOT_TRANSITION_KEY)
    }

    pub(crate) fn stage_secret(&self, version: &str, secret: &str) -> Result<(), MutationError> {
        if secret.is_empty() {
            return Err(MutationError::Invalid(
                "OpenRouter secret must not be empty when staged".into(),
            ));
        }
        self.secrets.put_new(version, secret)?;
        if self.secrets.get(version)?.as_deref() != Some(secret) {
            let _ = self.secrets.delete(version);
            return Err(MutationError::Invalid(
                "staged OpenRouter secret failed readback".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn resolve_secret(
        &self,
        version: Option<&str>,
    ) -> Result<Option<String>, MutationError> {
        version.map_or(Ok(None), |version| self.secrets.get(version))
    }

    pub(crate) fn delete_secret(&self, version: &str) -> Result<(), MutationError> {
        self.secrets.delete(version)
    }

    pub(crate) fn write_settings(
        &self,
        settings: &NonsecretSettingsV1,
    ) -> Result<(), MutationError> {
        settings.validate()?;
        let mut bytes = serde_json::to_vec_pretty(settings)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > SETTINGS_LIMIT {
            return Err(MutationError::Invalid(
                "settings exceed their size limit".into(),
            ));
        }
        self.config_root.put_atomic(&self.settings_key, &bytes)
    }

    #[cfg(test)]
    pub(crate) fn write_settings_guarded(
        &self,
        settings: &NonsecretSettingsV1,
    ) -> Result<(), MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = if self.read_transition()?.is_some() {
            Err(MutationError::RecoveryConflict(
                "root-transition-already-pending".into(),
            ))
        } else {
            self.write_settings(settings)
        };
        process_lock.unlock()?;
        result
    }

    #[cfg(test)]
    pub(crate) fn patch_settings_guarded<F>(
        &self,
        patch: F,
    ) -> Result<DurableSettingsSnapshot, MutationError>
    where
        F: FnOnce(&mut UserSettings) -> Result<(), MutationError>,
    {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = (|| {
            if self.read_transition()?.is_some() {
                return Err(MutationError::RecoveryConflict(
                    "root-transition-already-pending".into(),
                ));
            }
            let mut snapshot = self.read_settings_snapshot()?;
            patch(&mut snapshot.settings)?;
            let before_generation = snapshot.settings_generation.clone();
            if self.read_settings_generation()? != before_generation {
                return Err(MutationError::RecoveryConflict(
                    "settings-generation-changed-before-publish".into(),
                ));
            }
            self.write_settings(&NonsecretSettingsV1::from_settings(
                snapshot.settings.clone(),
            ))?;
            let published = self.read_settings_snapshot()?;
            if NonsecretSettingsV1::from_settings(published.settings.clone())
                != NonsecretSettingsV1::from_settings(snapshot.settings.clone())
            {
                return Err(MutationError::RecoveryConflict(
                    "settings-publish-readback-mismatch".into(),
                ));
            }
            Ok(published)
        })();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn load_startup_settings<L, C>(
        &self,
        load_legacy_key: L,
        clear_legacy_key: C,
    ) -> Result<DurableSettingsSnapshot, MutationError>
    where
        L: FnOnce() -> Result<Option<String>, MutationError>,
        C: FnOnce() -> Result<(), MutationError>,
    {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = (|| {
            self.recover_locked()?;
            let raw_bytes = self.read_settings_bytes()?;
            let raw_settings = match raw_bytes.as_deref() {
                Some(bytes) => serde_json::from_slice::<UserSettings>(bytes).map_err(|error| {
                    MutationError::Invalid(format!("invalid settings: {error}"))
                })?,
                None => UserSettings::default(),
            };
            let legacy_plaintext = raw_settings
                .openrouter_api_key
                .clone()
                .filter(|secret| !secret.is_empty());
            let legacy_keyring = load_legacy_key()?;
            let key_ref = self.read_key_ref()?;
            let legacy_present = legacy_plaintext.is_some() || legacy_keyring.is_some();

            match key_ref.source {
                OpenRouterKeySource::Versioned => {
                    let version = key_ref.active_version.as_deref().ok_or_else(|| {
                        MutationError::Invalid("versioned key authority has no version".into())
                    })?;
                    if self.secrets.get(version)?.is_none() {
                        return Err(MutationError::RecoveryConflict(
                            "active-openrouter-key-version-missing".into(),
                        ));
                    }
                }
                OpenRouterKeySource::Unset => {
                    if let Some(legacy_secret) = legacy_keyring.or(legacy_plaintext) {
                        match self.secrets.get(LEGACY_MIGRATION_KEY_VERSION)? {
                            Some(existing) if existing == legacy_secret => {}
                            Some(_) => {
                                return Err(MutationError::RecoveryConflict(
                                    "legacy-openrouter-migration-version-conflict".into(),
                                ));
                            }
                            None => self.stage_secret(LEGACY_MIGRATION_KEY_VERSION, &legacy_secret)?,
                        }
                        self.write_key_authority(
                            OpenRouterKeySource::Versioned,
                            Some(LEGACY_MIGRATION_KEY_VERSION),
                        )?;
                    }
                }
                OpenRouterKeySource::Cleared => {}
            }

            if legacy_present {
                self.write_settings(&NonsecretSettingsV1::from_settings(raw_settings))?;
                clear_legacy_key()?;
            }
            self.read_settings_snapshot()
        })();
        process_lock.unlock()?;
        result
    }

    #[cfg(test)]
    pub(crate) fn migrate_legacy_secret_authority(
        &self,
        legacy_secret: &str,
    ) -> Result<(String, String), MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = (|| {
            if self.read_transition()?.is_some() {
                return Err(MutationError::RecoveryConflict(
                    "root-transition-already-pending".into(),
                ));
            }
            let current = self.read_key_ref()?;
            let (version, secret) = if let Some(version) = current.active_version {
                let secret = self.secrets.get(&version)?.ok_or_else(|| {
                    MutationError::RecoveryConflict("active-openrouter-key-version-missing".into())
                })?;
                (version, secret)
            } else {
                let version = LEGACY_MIGRATION_KEY_VERSION.to_string();
                match self.secrets.get(&version)? {
                    Some(existing) if existing == legacy_secret => {}
                    Some(_) => {
                        self.secrets.delete(&version)?;
                        self.stage_secret(&version, legacy_secret)?;
                    }
                    None => self.stage_secret(&version, legacy_secret)?,
                }
                self.write_key_authority(OpenRouterKeySource::Versioned, Some(&version))?;
                (version, legacy_secret.to_string())
            };
            let current_settings = self.read_settings()?;
            self.write_settings(&current_settings)?;
            Ok((version, secret))
        })();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn write_lease(
        &self,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<(), MutationError> {
        if lease.schema_version != LEASE_SCHEMA_VERSION
            || Uuid::parse_str(&lease.epoch_uuid).is_err()
        {
            return Err(MutationError::Invalid("invalid active root lease".into()));
        }
        let mut bytes = serde_json::to_vec_pretty(lease)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > KEY_REF_LIMIT {
            return Err(MutationError::Invalid(
                "active root lease is too large".into(),
            ));
        }
        self.data_root.put_atomic(ACTIVE_ROOT_LEASE_KEY, &bytes)
    }

    #[cfg(test)]
    pub(crate) fn write_key_ref(&self, version: Option<&str>) -> Result<(), MutationError> {
        let source = if version.is_some() {
            OpenRouterKeySource::Versioned
        } else {
            OpenRouterKeySource::Unset
        };
        self.write_key_authority(source, version)
    }

    pub(crate) fn write_key_authority(
        &self,
        source: OpenRouterKeySource,
        version: Option<&str>,
    ) -> Result<(), MutationError> {
        validate_key_version(version)?;
        validate_key_authority(source, version)?;
        if let Some(version) = version {
            if self.secrets.get(version)?.is_none() {
                return Err(MutationError::Invalid(
                    "active OpenRouter key version does not resolve".into(),
                ));
            }
        }
        let key_ref = OpenRouterKeyRefV1 {
            schema_version: KEY_REF_SCHEMA_VERSION,
            source,
            active_version: version.map(str::to_string),
        };
        let mut bytes = serde_json::to_vec_pretty(&key_ref)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        self.data_root.put_atomic(OPENROUTER_KEY_REF, &bytes)
    }

    #[cfg(test)]
    pub(crate) fn active_key_version(&self) -> Result<Option<String>, MutationError> {
        Ok(self.read_key_ref()?.active_version)
    }

    #[cfg(any(test, feature = "mcp"))]
    pub(crate) fn recover(&self) -> Result<RecoveryWork, MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = self.recover_locked();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn recover_while_process_locked(&self) -> Result<RecoveryWork, MutationError> {
        self.recover_locked()
    }

    pub(crate) fn restore_prepared_authorities(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        self.require_exact_prepared_transition(transition)?;
        self.write_lease(&transition.rollback_lease)?;
        self.write_settings(&transition.before.nonsecret_settings)?;
        self.write_key_authority(
            transition.before.openrouter_key_source,
            transition.before.openrouter_key_version.as_deref(),
        )?;
        Ok(())
    }

    pub(crate) fn finalize_prepared_rollback(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        self.require_exact_prepared_transition(transition)?;
        if transition.after.openrouter_key_version != transition.before.openrouter_key_version {
            if let Some(version) = &transition.after.openrouter_key_version {
                self.secrets.delete(version)?;
            }
        }
        self.remove_transition()
    }

    fn require_exact_prepared_transition(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        if transition.decision != RootTransitionDecision::Prepared
            || self.read_transition()?.as_ref() != Some(transition)
        {
            return Err(MutationError::RecoveryConflict(
                "root-transition-prepared-cas-failed".into(),
            ));
        }
        Ok(())
    }

    fn recover_locked(&self) -> Result<RecoveryWork, MutationError> {
        let Some(transition) = self.read_transition()? else {
            return Ok(RecoveryWork::None);
        };
        transition.validate()?;
        let current_settings = self.read_settings()?;
        let current_lease = self.read_lease()?;
        let current_key_ref = self.read_key_ref()?;
        classify_component(
            &current_settings,
            &transition.before.nonsecret_settings,
            &transition.after.nonsecret_settings,
            "settings",
        )?;
        if transition.decision == RootTransitionDecision::Prepared {
            if current_lease != transition.before.lease
                && current_lease != transition.after.lease
                && current_lease != transition.rollback_lease
            {
                return Err(MutationError::RecoveryConflict(
                    "root-transition-third-state-active root lease".into(),
                ));
            }
        } else {
            classify_component(
                &current_lease,
                &transition.before.lease,
                &transition.after.lease,
                "active root lease",
            )?;
        }
        classify_component(
            &(current_key_ref.source, current_key_ref.active_version),
            &(
                transition.before.openrouter_key_source,
                transition.before.openrouter_key_version.clone(),
            ),
            &(
                transition.after.openrouter_key_source,
                transition.after.openrouter_key_version.clone(),
            ),
            "OpenRouter key reference",
        )?;

        match transition.decision {
            RootTransitionDecision::Prepared => {
                self.write_lease(&transition.rollback_lease)?;
                self.checkpoint(RootTransitionFaultPoint::AfterRollbackLease)?;
                self.write_settings(&transition.before.nonsecret_settings)?;
                self.checkpoint(RootTransitionFaultPoint::AfterRollbackSettings)?;
                self.write_key_authority(
                    transition.before.openrouter_key_source,
                    transition.before.openrouter_key_version.as_deref(),
                )?;
                self.checkpoint(RootTransitionFaultPoint::AfterRollbackKeyRef)?;
                if transition.after.openrouter_key_version
                    != transition.before.openrouter_key_version
                {
                    if let Some(version) = &transition.after.openrouter_key_version {
                        self.secrets.delete(version)?;
                    }
                }
                self.checkpoint(RootTransitionFaultPoint::AfterRollbackSecretDelete)?;
                self.remove_transition()?;
                self.checkpoint(RootTransitionFaultPoint::AfterRecoveryWalDelete)?;
                Ok(RecoveryWork::RolledBack)
            }
            RootTransitionDecision::Committed => {
                if let Some(version) = &transition.after.openrouter_key_version {
                    if self.secrets.get(version)?.is_none() {
                        return Err(MutationError::RecoveryConflict(
                            "committed-openrouter-key-version-missing".into(),
                        ));
                    }
                }
                self.write_lease(&transition.after.lease)?;
                self.checkpoint(RootTransitionFaultPoint::AfterLease)?;
                self.write_settings(&transition.after.nonsecret_settings)?;
                self.checkpoint(RootTransitionFaultPoint::AfterSettings)?;
                self.write_key_authority(
                    transition.after.openrouter_key_source,
                    transition.after.openrouter_key_version.as_deref(),
                )?;
                self.checkpoint(RootTransitionFaultPoint::AfterKeyRef)?;
                if transition.after.openrouter_key_version
                    != transition.before.openrouter_key_version
                {
                    if let Some(version) = &transition.before.openrouter_key_version {
                        self.secrets.delete(version)?;
                    }
                }
                self.checkpoint(RootTransitionFaultPoint::AfterOldKeyDelete)?;
                self.remove_transition()?;
                self.checkpoint(RootTransitionFaultPoint::AfterRecoveryWalDelete)?;
                Ok(RecoveryWork::RolledForward)
            }
        }
    }

    pub(crate) fn checkpoint(&self, point: RootTransitionFaultPoint) -> Result<(), MutationError> {
        let mut configured = self
            .fault_once
            .lock()
            .map_err(|_| MutationError::Invalid("root transition fault lock poisoned".into()))?;
        if configured.as_ref() == Some(&point) {
            *configured = None;
            return Err(MutationError::Io(format!(
                "injected root transition crash at {point:?}"
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_once_at(&self, point: RootTransitionFaultPoint) {
        *self.fault_once.lock().expect("root transition fault lock") = Some(point);
    }

    fn read_transition(&self) -> Result<Option<RootTransitionV1>, MutationError> {
        let Some(bytes) = self
            .data_root
            .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
        else {
            return Ok(None);
        };
        let transition: RootTransitionV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid root transition: {error}")))?;
        transition.validate()?;
        if transition.authority_binding != self.authority_binding {
            return Err(MutationError::RecoveryConflict(
                "root-transition-authority-binding-mismatch".into(),
            ));
        }
        Ok(Some(transition))
    }

    fn read_settings(&self) -> Result<NonsecretSettingsV1, MutationError> {
        let Some(bytes) = self.read_settings_bytes()? else {
            return Ok(NonsecretSettingsV1::from_settings(UserSettings::default()));
        };
        let settings: UserSettings = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid settings: {error}")))?;
        let settings = NonsecretSettingsV1::from_settings(settings);
        settings.validate()?;
        Ok(settings)
    }

    fn read_settings_bytes(&self) -> Result<Option<Vec<u8>>, MutationError> {
        self.config_root
            .read_bounded(&self.settings_key, SETTINGS_LIMIT)
    }

    #[cfg(test)]
    fn read_settings_generation(&self) -> Result<ContentDigest, MutationError> {
        Ok(settings_generation(self.read_settings_bytes()?.as_deref()))
    }

    fn read_settings_snapshot(&self) -> Result<DurableSettingsSnapshot, MutationError> {
        let bytes = self.read_settings_bytes()?;
        let nonsecret_settings = match bytes.as_deref() {
            Some(bytes) => {
                let settings: UserSettings = serde_json::from_slice(bytes).map_err(|error| {
                    MutationError::Invalid(format!("invalid settings: {error}"))
                })?;
                let settings = NonsecretSettingsV1::from_settings(settings);
                settings.validate()?;
                settings
            }
            None => NonsecretSettingsV1::from_settings(UserSettings::default()),
        };
        let key_ref = self.read_key_ref()?;
        let active_key_version = key_ref.active_version.clone();
        let resolved_secret = self.resolve_secret(active_key_version.as_deref())?;
        if active_key_version.is_some() && resolved_secret.is_none() {
            return Err(MutationError::RecoveryConflict(
                "active-openrouter-key-version-missing".into(),
            ));
        }
        let mut settings = nonsecret_settings.into_settings();
        settings.openrouter_api_key = resolved_secret.clone();
        Ok(DurableSettingsSnapshot {
            settings,
            settings_generation: settings_generation(bytes.as_deref()),
            active_key_version,
            key_source: key_ref.source,
            resolved_secret,
        })
    }

    fn read_lease(&self) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
        let bytes = self
            .data_root
            .read_bounded(ACTIVE_ROOT_LEASE_KEY, KEY_REF_LIMIT)?
            .ok_or_else(|| MutationError::Invalid("active root lease is missing".into()))?;
        let lease: ActiveMarkdownRootLeaseV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid root lease: {error}")))?;
        if lease.schema_version != LEASE_SCHEMA_VERSION
            || Uuid::parse_str(&lease.epoch_uuid).is_err()
        {
            return Err(MutationError::Invalid("invalid active root lease".into()));
        }
        Ok(lease)
    }

    fn read_key_ref(&self) -> Result<OpenRouterKeyRefV1, MutationError> {
        let Some(bytes) = self
            .data_root
            .read_bounded(OPENROUTER_KEY_REF, KEY_REF_LIMIT)?
        else {
            return Ok(OpenRouterKeyRefV1 {
                schema_version: KEY_REF_SCHEMA_VERSION,
                source: OpenRouterKeySource::Unset,
                active_version: None,
            });
        };
        let key_ref: OpenRouterKeyRefV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid key reference: {error}")))?;
        if key_ref.schema_version != KEY_REF_SCHEMA_VERSION {
            return Err(MutationError::Invalid(
                "unsupported OpenRouter key-reference schema".into(),
            ));
        }
        validate_key_version(key_ref.active_version.as_deref())?;
        validate_key_authority(key_ref.source, key_ref.active_version.as_deref())?;
        Ok(key_ref)
    }

    #[cfg(test)]
    fn seed_secret_for_test(&self, version: &str, secret: &str) -> Result<(), MutationError> {
        self.secrets.put_new(version, secret)
    }

    #[cfg(test)]
    fn read_secret_for_test(&self, version: &str) -> Result<Option<String>, MutationError> {
        self.secrets.get(version)
    }

    #[cfg(test)]
    fn write_authority_for_test(&self, authority: &RootAuthorityV1) -> Result<(), MutationError> {
        self.write_lease(&authority.lease)?;
        self.write_settings(&authority.nonsecret_settings)?;
        self.write_key_authority(
            authority.openrouter_key_source,
            authority.openrouter_key_version.as_deref(),
        )
    }

    #[cfg(test)]
    fn read_authority_for_test(&self) -> Result<RootAuthorityV1, MutationError> {
        let settings = self.read_settings()?;
        let vault = settings
            .vault_path
            .clone()
            .ok_or_else(|| MutationError::Invalid("test settings have no vault".into()))?;
        let key_ref = self.read_key_ref()?;
        RootAuthorityV1::new_with_key_source(
            Path::new(&vault),
            settings.into_settings(),
            self.read_lease()?,
            key_ref.source,
            key_ref.active_version,
        )
    }

    #[cfg(test)]
    fn read_settings_bytes_for_test(&self) -> Result<Vec<u8>, MutationError> {
        self.config_root
            .read_bounded(&self.settings_key, SETTINGS_LIMIT)?
            .ok_or_else(|| MutationError::Invalid("settings are missing".into()))
    }

    #[cfg(test)]
    fn write_raw_transition_for_test(&self, bytes: &[u8]) -> Result<(), MutationError> {
        self.data_root.put_atomic(ROOT_TRANSITION_KEY, bytes)
    }

    #[cfg(test)]
    fn write_transition(&self, transition: &RootTransitionV1) -> Result<(), MutationError> {
        let bytes = self.encoded_transition(transition)?;
        self.data_root.put_atomic(ROOT_TRANSITION_KEY, &bytes)
    }

    #[cfg(test)]
    fn read_transition_bytes_for_test(&self) -> Result<Vec<u8>, MutationError> {
        self.data_root
            .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
            .ok_or_else(|| MutationError::Invalid("root transition is missing".into()))
    }

    #[cfg(test)]
    fn transition_exists_for_test(&self) -> Result<bool, MutationError> {
        Ok(self
            .data_root
            .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
            .is_some())
    }
}

fn classify_component<T: PartialEq>(
    current: &T,
    before: &T,
    after: &T,
    label: &str,
) -> Result<(), MutationError> {
    if current == before || current == after {
        Ok(())
    } else {
        Err(MutationError::RecoveryConflict(format!(
            "root-transition-third-state-{label}"
        )))
    }
}

fn validate_key_version(version: Option<&str>) -> Result<(), MutationError> {
    if version.is_some_and(|version| Uuid::parse_str(version).is_err()) {
        return Err(MutationError::Invalid(
            "OpenRouter key version must be a UUID".into(),
        ));
    }
    Ok(())
}

fn validate_key_authority(
    source: OpenRouterKeySource,
    version: Option<&str>,
) -> Result<(), MutationError> {
    let valid = match source {
        OpenRouterKeySource::Versioned => version.is_some(),
        OpenRouterKeySource::Unset | OpenRouterKeySource::Cleared => version.is_none(),
    };
    if !valid {
        return Err(MutationError::Invalid(
            "OpenRouter key source and version disagree".into(),
        ));
    }
    Ok(())
}

fn settings_generation(bytes: Option<&[u8]>) -> ContentDigest {
    let mut domain = b"grafyn.settings_generation.v1".to_vec();
    match bytes {
        Some(bytes) => {
            domain.push(1);
            domain.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            domain.extend_from_slice(bytes);
        }
        None => domain.push(0),
    }
    crate::services::twin_events::digest_bytes(&domain)
}

fn root_authority_binding(
    data_path: &Path,
    config_path: &Path,
    settings_key: &str,
) -> Result<ContentDigest, MutationError> {
    let data_scope = root_identity_for_path(data_path)?;
    let config_scope = root_identity_for_path(config_path)?;
    let mut domain = b"grafyn.root_transition_authority.v1".to_vec();
    for value in [
        data_scope.as_str(),
        config_scope.as_str(),
        settings_key,
        KEYRING_SERVICE,
        VERSIONED_KEY_PREFIX,
    ] {
        domain.extend_from_slice(&(value.len() as u64).to_be_bytes());
        domain.extend_from_slice(value.as_bytes());
    }
    Ok(crate::services::twin_events::digest_bytes(&domain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::UserSettings;
    use crate::services::twin_events::{root_identity_for_path, ActiveMarkdownRootLeaseV1};
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, RootTransitionStore, RootTransitionV1) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let config = temp.path().join("config");
        let old = temp.path().join("vault-a");
        let new = temp.path().join("vault-b");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        let secrets = Arc::new(MemoryVersionedSecretStore::default());
        let store = RootTransitionStore::new(&data, config.join("settings.json"), secrets).unwrap();
        let before_settings = UserSettings {
            vault_path: Some(old.to_string_lossy().into_owned()),
            ..UserSettings::default()
        };
        let after_settings = UserSettings {
            vault_path: Some(new.to_string_lossy().into_owned()),
            theme: "dark".into(),
            ..UserSettings::default()
        };
        let old_key = Uuid::new_v4().to_string();
        let new_key = Uuid::new_v4().to_string();
        let before = RootAuthorityV1::new(
            &old,
            before_settings,
            ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&old).unwrap()),
            Some(old_key),
        )
        .unwrap();
        let after = RootAuthorityV1::new(
            &new,
            after_settings,
            ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&new).unwrap()),
            Some(new_key),
        )
        .unwrap();
        let transition =
            RootTransitionV1::prepared(before, after, store.authority_binding()).unwrap();
        (temp, store, transition)
    }

    #[test]
    fn guarded_settings_patch_uses_fresh_durable_state_instead_of_stale_process_state() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let config = temp.path().join("config");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let secrets = Arc::new(MemoryVersionedSecretStore::default());
        let first = RootTransitionStore::new(
            &data,
            config.join("settings.json"),
            secrets.clone(),
        )
        .unwrap();
        let stale = RootTransitionStore::new(&data, config.join("settings.json"), secrets).unwrap();
        let initial = UserSettings {
            vault_path: Some(vault.to_string_lossy().into_owned()),
            theme: "light".into(),
            mcp_enabled: false,
            ..UserSettings::default()
        };
        first
            .write_settings_guarded(&NonsecretSettingsV1::from_settings(initial))
            .unwrap();

        first
            .patch_settings_guarded(|fresh| {
                fresh.mcp_enabled = true;
                Ok(())
            })
            .unwrap();
        let patched = stale
            .patch_settings_guarded(|fresh| {
                fresh.theme = "dark".into();
                Ok(())
            })
            .unwrap();

        assert_eq!(patched.settings.theme, "dark");
        assert!(patched.settings.mcp_enabled);
        assert_eq!(patched.settings.effective_vault_path(), vault);
    }

    #[test]
    fn copied_transition_wal_cannot_act_for_another_data_config_or_key_authority() {
        let (_fixture, source, transition) = fixture();
        let other = tempfile::tempdir().unwrap();
        let data = other.path().join("data");
        let config = other.path().join("config");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&config).unwrap();
        let destination = RootTransitionStore::new(
            &data,
            config.join("settings.json"),
            Arc::new(MemoryVersionedSecretStore::default()),
        )
        .unwrap();

        assert_ne!(source.authority_binding(), destination.authority_binding());
        let error = destination.prepare_transition(&transition).unwrap_err();
        assert!(error.to_string().contains("authority-binding"));
        assert!(!destination.transition_exists_for_test().unwrap());
    }

    #[test]
    fn prepared_recovery_rolls_back_mixed_authorities_and_second_restart_is_zero_work() {
        let (_temp, store, transition) = fixture();
        let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
        let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
        store.seed_secret_for_test(old_key, "old-secret").unwrap();
        store.seed_secret_for_test(new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        store.write_transition(&transition).unwrap();
        store.write_lease(&transition.after.lease).unwrap();
        store
            .write_key_ref(transition.after.openrouter_key_version.as_deref())
            .unwrap();

        assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);
        let recovered = store.read_authority_for_test().unwrap();
        assert_eq!(
            recovered.nonsecret_settings,
            transition.before.nonsecret_settings
        );
        assert_eq!(recovered.root_scope, transition.before.root_scope);
        assert_eq!(
            recovered.openrouter_key_version,
            transition.before.openrouter_key_version
        );
        assert_eq!(recovered.lease, transition.rollback_lease);
        assert!(store.read_secret_for_test(new_key).unwrap().is_none());
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }

    #[test]
    fn committed_recovery_rolls_forward_and_second_restart_is_zero_work() {
        let (_temp, store, mut transition) = fixture();
        let old_key = transition.before.openrouter_key_version.clone().unwrap();
        let new_key = transition.after.openrouter_key_version.clone().unwrap();
        store.seed_secret_for_test(&old_key, "old-secret").unwrap();
        store.seed_secret_for_test(&new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        transition.decision = RootTransitionDecision::Committed;
        store.write_transition(&transition).unwrap();
        store
            .write_settings(&transition.after.nonsecret_settings)
            .unwrap();

        assert_eq!(store.recover().unwrap(), RecoveryWork::RolledForward);
        assert_eq!(store.read_authority_for_test().unwrap(), transition.after);
        assert!(store.read_secret_for_test(&old_key).unwrap().is_none());
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }

    #[test]
    fn unexpected_third_settings_state_preserves_every_byte_and_wal() {
        let (_temp, store, transition) = fixture();
        let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
        let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
        store.seed_secret_for_test(old_key, "old-secret").unwrap();
        store.seed_secret_for_test(new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        store.write_transition(&transition).unwrap();
        let third = UserSettings {
            theme: "third".into(),
            ..UserSettings::default()
        };
        store
            .write_settings(&NonsecretSettingsV1::from_settings(third))
            .unwrap();
        let before = store.read_settings_bytes_for_test().unwrap();
        assert!(store.recover().is_err());
        assert_eq!(store.read_settings_bytes_for_test().unwrap(), before);
        assert!(store.transition_exists_for_test().unwrap());
    }

    #[test]
    fn strict_and_bounded_transition_read_fails_closed() {
        let (_temp, store, transition) = fixture();
        let mut value = serde_json::to_value(&transition).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), true.into());
        store
            .write_raw_transition_for_test(&serde_json::to_vec(&value).unwrap())
            .unwrap();
        assert!(store.recover().is_err());
        store
            .write_raw_transition_for_test(&vec![b'x'; ROOT_TRANSITION_LIMIT + 1])
            .unwrap();
        assert!(store.recover().is_err());
    }

    #[test]
    fn prepared_recovery_restarts_after_every_rollback_durable_phase() {
        for point in [
            RootTransitionFaultPoint::AfterRollbackLease,
            RootTransitionFaultPoint::AfterRollbackSettings,
            RootTransitionFaultPoint::AfterRollbackKeyRef,
            RootTransitionFaultPoint::AfterRollbackSecretDelete,
            RootTransitionFaultPoint::AfterRecoveryWalDelete,
        ] {
            let (_temp, store, transition) = fixture();
            let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
            let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
            store.seed_secret_for_test(old_key, "old-secret").unwrap();
            store.seed_secret_for_test(new_key, "new-secret").unwrap();
            store.write_authority_for_test(&transition.before).unwrap();
            store.write_transition(&transition).unwrap();
            store.write_lease(&transition.after.lease).unwrap();
            store
                .write_settings(&transition.after.nonsecret_settings)
                .unwrap();
            store
                .write_key_ref(transition.after.openrouter_key_version.as_deref())
                .unwrap();
            store.fail_once_at(point);
            assert!(
                store.recover().is_err(),
                "fault {point:?} must interrupt recovery"
            );
            let resumed = store.recover().unwrap();
            if point == RootTransitionFaultPoint::AfterRecoveryWalDelete {
                assert_eq!(resumed, RecoveryWork::None);
            } else {
                assert_eq!(resumed, RecoveryWork::RolledBack);
            }
            assert_eq!(store.recover().unwrap(), RecoveryWork::None);
        }
    }

    #[test]
    fn committed_recovery_restarts_after_every_rollforward_durable_phase() {
        for point in [
            RootTransitionFaultPoint::AfterLease,
            RootTransitionFaultPoint::AfterSettings,
            RootTransitionFaultPoint::AfterKeyRef,
            RootTransitionFaultPoint::AfterOldKeyDelete,
            RootTransitionFaultPoint::AfterRecoveryWalDelete,
        ] {
            let (_temp, store, mut transition) = fixture();
            let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
            let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
            store.seed_secret_for_test(old_key, "old-secret").unwrap();
            store.seed_secret_for_test(new_key, "new-secret").unwrap();
            store.write_authority_for_test(&transition.before).unwrap();
            transition.decision = RootTransitionDecision::Committed;
            store.write_transition(&transition).unwrap();
            store.fail_once_at(point);
            assert!(
                store.recover().is_err(),
                "fault {point:?} must interrupt recovery"
            );
            let resumed = store.recover().unwrap();
            if point == RootTransitionFaultPoint::AfterRecoveryWalDelete {
                assert_eq!(resumed, RecoveryWork::None);
            } else {
                assert_eq!(resumed, RecoveryWork::RolledForward);
            }
            assert_eq!(store.recover().unwrap(), RecoveryWork::None);
        }
    }

    #[test]
    fn prepared_wal_is_no_clobber_and_commit_is_same_transaction_cas() {
        let (_temp, store, first) = fixture();
        let (_second_temp, _second_store, second) = fixture();
        store.prepare_transition(&first).unwrap();
        let first_bytes = store.read_transition_bytes_for_test().unwrap();
        assert!(store.prepare_transition(&second).is_err());
        assert_eq!(store.read_transition_bytes_for_test().unwrap(), first_bytes);

        let wrong = second;
        assert!(matches!(
            store.mark_committed(&wrong),
            MarkCommittedResult::Uncertain(_)
        ));
        assert_eq!(store.read_transition_bytes_for_test().unwrap(), first_bytes);
        let exact = first;
        let committed = match store.mark_committed(&exact) {
            MarkCommittedResult::Committed(committed) => committed,
            other => panic!("expected committed WAL, got {other:?}"),
        };
        assert_eq!(committed.decision, RootTransitionDecision::Committed);
        let committed_bytes = store.read_transition_bytes_for_test().unwrap();
        assert!(matches!(
            store.mark_committed(&committed),
            MarkCommittedResult::Uncertain(_)
        ));
        assert_eq!(
            store.read_transition_bytes_for_test().unwrap(),
            committed_bytes
        );
    }

    #[test]
    fn commit_result_distinguishes_prepared_committed_and_uncertain_durability() {
        let (_temp, store, transition) = fixture();
        store.prepare_transition(&transition).unwrap();
        store.fail_once_at(RootTransitionFaultPoint::BeforeCommitWalWrite);
        assert!(matches!(
            store.mark_committed(&transition),
            MarkCommittedResult::DefinitelyPrepared(_)
        ));
        assert_eq!(store.read_transition().unwrap(), Some(transition.clone()));

        store.fail_once_at(RootTransitionFaultPoint::AfterCommitWalWrite);
        let committed = match store.mark_committed(&transition) {
            MarkCommittedResult::Committed(committed) => committed,
            other => panic!("durable committed WAL must win after an uncertain write: {other:?}"),
        };
        assert_eq!(committed.decision, RootTransitionDecision::Committed);

        let (_other_temp, other_store, other) = fixture();
        other_store.prepare_transition(&other).unwrap();
        let mut wrong = other.clone();
        wrong.transaction_id = Uuid::new_v4().to_string();
        assert!(matches!(
            other_store.mark_committed(&wrong),
            MarkCommittedResult::Uncertain(_)
        ));
        assert_eq!(other_store.read_transition().unwrap(), Some(other));
    }

    #[test]
    fn prepared_rollback_helpers_require_the_exact_durable_transaction() {
        let (_temp, store, transition) = fixture();
        let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
        let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
        store.seed_secret_for_test(old_key, "old-secret").unwrap();
        store.seed_secret_for_test(new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        store.prepare_transition(&transition).unwrap();
        let authority_before = store.read_authority_for_test().unwrap();

        let mut wrong = transition.clone();
        wrong.transaction_id = Uuid::new_v4().to_string();
        assert!(store.restore_prepared_authorities(&wrong).is_err());
        assert!(store.finalize_prepared_rollback(&wrong).is_err());
        assert_eq!(store.read_authority_for_test().unwrap(), authority_before);
        assert!(store.transition_exists_for_test().unwrap());
        assert!(store.read_secret_for_test(new_key).unwrap().is_some());
    }

    #[test]
    fn legacy_migration_uses_one_discoverable_version_and_sanitized_settings() {
        let (_temp, store, _transition) = fixture();

        let (version, secret) = store
            .migrate_legacy_secret_authority("new-keychain")
            .unwrap();
        assert_eq!(version, LEGACY_MIGRATION_KEY_VERSION);
        assert_eq!(secret, "new-keychain");
        assert_eq!(
            store.active_key_version().unwrap().as_deref(),
            Some(version.as_str())
        );
        assert_eq!(
            store.resolve_secret(Some(&version)).unwrap().as_deref(),
            Some("new-keychain")
        );
        assert!(
            !String::from_utf8(store.read_settings_bytes_for_test().unwrap())
                .unwrap()
                .contains("stale-plaintext")
        );

        let repeated = store
            .migrate_legacy_secret_authority("new-keychain")
            .unwrap();
        assert_eq!(repeated, (version, "new-keychain".into()));
    }
}
