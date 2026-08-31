use crate::models::settings::{CanvasModelPreset, UserSettings};
use crate::models::sync::VaultDescriptorV1;
use crate::models::twin_event::ContentDigest;
#[cfg(test)]
use crate::services::sync::secrets::MemorySecretStore;
use crate::services::sync::secrets::{
    SecretAccount, SecretBytes, SecretStore, SecretStoreError, SECRET_STORE_SERVICE,
};
use crate::services::twin_events::{
    root_identity_for_path, ActiveMarkdownRootLeaseV1, AnchoredRoot, MutationError,
};
use serde::{Deserialize, Serialize};
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
const STABLE_ROOT_TRANSITION_SCHEMA_VERSION: u16 = 2;
const FORWARD_REATTACH_TRANSITION_SCHEMA_VERSION: u16 = 3;
const KEY_REF_SCHEMA_VERSION: u16 = 1;
const LEASE_SCHEMA_VERSION: u16 = 1;
const STABLE_LEASE_SCHEMA_VERSION: u16 = 2;
const VERSIONED_KEY_PREFIX: &str = "openrouter_api_key/";
const LEGACY_MIGRATION_KEY_VERSION: &str = "00000000-0000-4000-8000-000000000001";

#[cfg(test)]
pub(crate) type MemoryVersionedSecretStore = MemorySecretStore;

pub(crate) fn reject_transition_wal_locked(
    data_path: &Path,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if !process_lock.covers_data_path(data_path)? {
        return Err(MutationError::Invalid(
            "root transition check lock belongs to another data root".into(),
        ));
    }
    let root = AnchoredRoot::open(data_path)?;
    if root
        .read_bounded(ROOT_TRANSITION_KEY, ROOT_TRANSITION_LIMIT)?
        .is_some()
    {
        return Err(MutationError::RecoveryConflict(
            "root-transition-requires-authority-recovery".into(),
        ));
    }
    Ok(())
}

#[cfg(feature = "mcp")]
pub(crate) fn stable_root_scope_locked(
    data_path: &Path,
    process_lock: &crate::services::twin_events::CoordinatorProcessLock,
) -> Result<Option<ContentDigest>, MutationError> {
    if !process_lock.covers_data_path(data_path)? {
        return Err(MutationError::Invalid(
            "stable root probe lock belongs to another data root".into(),
        ));
    }
    let root = AnchoredRoot::open(data_path)?;
    root.open_directory("twin/events", true)?;
    let Some(bytes) = root.read_bounded(ACTIVE_ROOT_LEASE_KEY, KEY_REF_LIMIT)? else {
        return Ok(None);
    };
    let lease = parse_active_root_lease(&bytes)?;
    Ok(lease.is_stable().then_some(lease.root_scope))
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
        let root_scope = authority_scope_for(&canonical, &lease)?;
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

    pub(crate) fn new_with_pending_stable_descriptor(
        vault_path: &Path,
        settings: UserSettings,
        lease: ActiveMarkdownRootLeaseV1,
        openrouter_key_source: OpenRouterKeySource,
        openrouter_key_version: Option<String>,
    ) -> Result<Self, MutationError> {
        if !lease.is_stable() {
            return Err(MutationError::Invalid(
                "pending vault descriptor requires a stable lease".into(),
            ));
        }
        let canonical = std::fs::canonicalize(vault_path)?;
        let canonical_vault_path = canonical
            .to_str()
            .ok_or_else(|| MutationError::Invalid("canonical vault path must be Unicode".into()))?
            .to_string();
        let settings_vault = std::fs::canonicalize(settings.effective_vault_path())?;
        if settings_vault != canonical {
            return Err(MutationError::Invalid(
                "pending vault descriptor settings path is inconsistent".into(),
            ));
        }
        let authority = Self {
            canonical_vault_path,
            root_scope: lease.root_scope.clone(),
            lease,
            nonsecret_settings: NonsecretSettingsV1::from_settings(settings),
            openrouter_key_source,
            openrouter_key_version,
        };
        authority.validate_serialized_stable()?;
        Ok(authority)
    }

    fn validate(&self) -> Result<(), MutationError> {
        self.nonsecret_settings.validate()?;
        if !matches!(
            self.lease.schema_version,
            LEASE_SCHEMA_VERSION | STABLE_LEASE_SCHEMA_VERSION
        ) || self.lease.root_scope != self.root_scope
            || !is_canonical_non_nil_uuid(&self.lease.epoch_uuid)
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
            || authority_scope_for(&canonical, &self.lease)? != self.root_scope
            || settings_vault != canonical
        {
            return Err(MutationError::Invalid(
                "root transition vault identity is inconsistent".into(),
            ));
        }
        Ok(())
    }

    fn validate_detached_stable(&self) -> Result<(), MutationError> {
        self.nonsecret_settings.validate()?;
        if !self.lease.is_stable()
            || self.lease.root_scope != self.root_scope
            || !is_canonical_non_nil_uuid(&self.lease.epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "forward reattach contains an invalid detached lease".into(),
            ));
        }
        validate_key_version(self.openrouter_key_version.as_deref())?;
        validate_key_authority(
            self.openrouter_key_source,
            self.openrouter_key_version.as_deref(),
        )?;
        let configured = self
            .nonsecret_settings
            .clone()
            .into_settings()
            .effective_vault_path();
        let recorded = Path::new(&self.canonical_vault_path);
        if configured != recorded || !is_normal_absolute_path(recorded) {
            return Err(MutationError::Invalid(
                "forward reattach detached vault path is invalid".into(),
            ));
        }
        Ok(())
    }

    fn validate_serialized_stable(&self) -> Result<(), MutationError> {
        self.nonsecret_settings.validate()?;
        if !self.lease.is_stable()
            || self.lease.root_scope != self.root_scope
            || !is_canonical_non_nil_uuid(&self.lease.epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "root transition contains an invalid serialized stable lease".into(),
            ));
        }
        validate_key_version(self.openrouter_key_version.as_deref())?;
        validate_key_authority(
            self.openrouter_key_source,
            self.openrouter_key_version.as_deref(),
        )?;
        let recorded = Path::new(&self.canonical_vault_path);
        if !is_normal_absolute_path(recorded) {
            return Err(MutationError::Invalid(
                "root transition contains an invalid serialized stable vault path".into(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnedCandidateDescriptorWitnessV1 {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    #[serde(default)]
    pub(crate) live_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootTransitionV1 {
    pub(crate) schema_version: u16,
    pub(crate) transaction_id: String,
    pub(crate) decision: RootTransitionDecision,
    pub(crate) authority_binding: ContentDigest,
    pub(crate) root_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path_changed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) vault_identity_changed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) forward_only_reattach: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owned_candidate_vault_descriptor: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owned_candidate_vault_descriptor_witness: Option<OwnedCandidateDescriptorWitnessV1>,
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
        Self::prepared_with_owned_candidate_descriptor(before, after, authority_binding, None)
    }

    pub(crate) fn prepared_with_owned_candidate_descriptor(
        before: RootAuthorityV1,
        after: RootAuthorityV1,
        authority_binding: ContentDigest,
        owned_candidate_vault_descriptor: Option<Vec<u8>>,
    ) -> Result<Self, MutationError> {
        let stable = before.lease.is_stable() && after.lease.is_stable();
        let path_changed = before.canonical_vault_path != after.canonical_vault_path;
        let vault_identity_changed = before.root_scope != after.root_scope;
        let rollback_lease = if stable {
            ActiveMarkdownRootLeaseV1::new_stable(before.root_scope.clone())
        } else {
            ActiveMarkdownRootLeaseV1::new(before.root_scope.clone())
        };
        let transition = Self {
            schema_version: if stable {
                STABLE_ROOT_TRANSITION_SCHEMA_VERSION
            } else {
                ROOT_TRANSITION_SCHEMA_VERSION
            },
            transaction_id: Uuid::new_v4().to_string(),
            decision: RootTransitionDecision::Prepared,
            authority_binding,
            root_changed: if stable {
                path_changed || vault_identity_changed
            } else {
                vault_identity_changed
            },
            path_changed: stable.then_some(path_changed),
            vault_identity_changed: stable.then_some(vault_identity_changed),
            forward_only_reattach: None,
            owned_candidate_vault_descriptor,
            owned_candidate_vault_descriptor_witness: None,
            before,
            after,
            rollback_lease,
        };
        transition.validate_preparation()?;
        Ok(transition)
    }

    fn validate_preparation(&self) -> Result<(), MutationError> {
        self.validate()?;
        if self.schema_version == STABLE_ROOT_TRANSITION_SCHEMA_VERSION
            && self.decision == RootTransitionDecision::Prepared
        {
            if self.owned_candidate_vault_descriptor_witness.is_some() {
                return Err(MutationError::Invalid(
                    "new root transition cannot already claim a candidate descriptor witness"
                        .into(),
                ));
            }
            if self.owned_candidate_vault_descriptor.is_some() {
                self.validate_owned_candidate_descriptor_state(true)?;
            } else {
                self.after.validate()?;
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), MutationError> {
        let transition_flags_valid = match self.schema_version {
            ROOT_TRANSITION_SCHEMA_VERSION => {
                self.path_changed.is_none()
                    && self.vault_identity_changed.is_none()
                    && self.forward_only_reattach.is_none()
                    && self.owned_candidate_vault_descriptor.is_none()
                    && self.owned_candidate_vault_descriptor_witness.is_none()
                    && self.root_changed == (self.before.root_scope != self.after.root_scope)
                    && !self.before.lease.is_stable()
                    && !self.after.lease.is_stable()
                    && self.rollback_lease.schema_version == LEASE_SCHEMA_VERSION
            }
            STABLE_ROOT_TRANSITION_SCHEMA_VERSION => {
                let path_changed =
                    self.before.canonical_vault_path != self.after.canonical_vault_path;
                let vault_identity_changed = self.before.root_scope != self.after.root_scope;
                self.path_changed == Some(path_changed)
                    && self.vault_identity_changed == Some(vault_identity_changed)
                    && self.forward_only_reattach.is_none()
                    && self.root_changed == (path_changed || vault_identity_changed)
                    && self
                        .owned_candidate_vault_descriptor
                        .as_ref()
                        .is_none_or(|_| path_changed && vault_identity_changed)
                    && self
                        .owned_candidate_vault_descriptor_witness
                        .as_ref()
                        .is_none_or(|_| self.owned_candidate_vault_descriptor.is_some())
                    && (self.decision != RootTransitionDecision::Committed
                        || self.owned_candidate_vault_descriptor.is_none()
                        || self
                            .owned_candidate_vault_descriptor_witness
                            .is_some_and(|witness| witness.live_installed))
                    && self.before.lease.is_stable()
                    && self.after.lease.is_stable()
                    && self.rollback_lease.is_stable()
            }
            FORWARD_REATTACH_TRANSITION_SCHEMA_VERSION => {
                self.forward_only_reattach == Some(true)
                    && self.owned_candidate_vault_descriptor.is_none()
                    && self.owned_candidate_vault_descriptor_witness.is_none()
                    && self.path_changed == Some(true)
                    && self.vault_identity_changed == Some(false)
                    && self.root_changed
                    && self.before.lease.is_stable()
                    && self.after.lease.is_stable()
                    && self.before.root_scope == self.after.root_scope
                    && self.before.canonical_vault_path != self.after.canonical_vault_path
                    && self.before.lease.epoch_uuid != self.after.lease.epoch_uuid
                    && self.rollback_lease == self.after.lease
                    && same_nonsecret_settings_except_vault(
                        &self.before.nonsecret_settings,
                        &self.after.nonsecret_settings,
                    )
                    && self.before.openrouter_key_source == self.after.openrouter_key_source
                    && self.before.openrouter_key_version == self.after.openrouter_key_version
            }
            _ => false,
        };
        if !transition_flags_valid
            || Uuid::parse_str(&self.transaction_id).is_err()
            || self.rollback_lease.root_scope != self.before.root_scope
            || !is_canonical_non_nil_uuid(&self.rollback_lease.epoch_uuid)
        {
            return Err(MutationError::Invalid(
                "invalid root transition metadata".into(),
            ));
        }
        match self.schema_version {
            ROOT_TRANSITION_SCHEMA_VERSION => {
                self.before.validate()?;
                self.after.validate()?;
            }
            STABLE_ROOT_TRANSITION_SCHEMA_VERSION => match self.decision {
                RootTransitionDecision::Prepared => {
                    self.before.validate()?;
                    self.after.validate_serialized_stable()?;
                    if self.owned_candidate_vault_descriptor.is_some() {
                        self.validate_owned_candidate_descriptor_state(false)?;
                    }
                }
                RootTransitionDecision::Committed => {
                    self.before.validate_serialized_stable()?;
                    self.after.validate()?;
                    if self.owned_candidate_vault_descriptor.is_some() {
                        self.validate_owned_candidate_descriptor_state(false)?;
                    }
                }
            },
            FORWARD_REATTACH_TRANSITION_SCHEMA_VERSION => {
                self.before.validate_detached_stable()?;
                self.after.validate()?;
            }
            _ => unreachable!("unsupported transition schema passed metadata validation"),
        }
        Ok(())
    }

    fn owned_candidate_descriptor_bytes(&self) -> Result<Option<&[u8]>, MutationError> {
        let Some(bytes) = self.owned_candidate_vault_descriptor.as_deref() else {
            return Ok(None);
        };
        if bytes.len() > crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT {
            return Err(MutationError::Invalid(
                "owned candidate vault descriptor exceeds its size limit".into(),
            ));
        }
        let descriptor: VaultDescriptorV1 = serde_json::from_slice(bytes).map_err(|error| {
            MutationError::Invalid(format!("invalid owned candidate vault descriptor: {error}"))
        })?;
        let mut canonical = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        canonical.push(b'\n');
        if canonical != bytes
            || crate::services::sync::identity::stable_vault_scope(descriptor.vault_id())
                != self.after.root_scope
        {
            return Err(MutationError::Invalid(
                "owned candidate vault descriptor is not canonical for the after authority".into(),
            ));
        }
        Ok(Some(bytes))
    }

    fn candidate_descriptor_rollback_key(&self) -> String {
        format!(
            "_grafyn/.root-transition-{}.vault-descriptor-rollback",
            self.transaction_id
        )
    }

    fn validate_owned_candidate_descriptor_state(
        &self,
        preparing: bool,
    ) -> Result<(), MutationError> {
        let expected = self
            .owned_candidate_descriptor_bytes()?
            .ok_or_else(|| MutationError::Invalid("owned vault descriptor is missing".into()))?;
        let candidate = Path::new(&self.after.canonical_vault_path);
        match std::fs::symlink_metadata(candidate) {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && !preparing
                    && self.decision == RootTransitionDecision::Prepared =>
            {
                return Ok(())
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(MutationError::RecoveryConflict(
                    "owned candidate vault path is not a real directory".into(),
                ));
            }
            Ok(_) => {}
        }
        let canonical = std::fs::canonicalize(candidate)?;
        let settings_vault = std::fs::canonicalize(
            self.after
                .nonsecret_settings
                .clone()
                .into_settings()
                .effective_vault_path(),
        )?;
        if canonical.to_string_lossy() != self.after.canonical_vault_path
            || settings_vault != canonical
        {
            return Err(MutationError::Invalid(
                "owned candidate vault descriptor path is inconsistent".into(),
            ));
        }
        let root = AnchoredRoot::open(&canonical)?;
        let live = root.read_bounded(
            crate::services::sync::identity::VAULT_DESCRIPTOR_KEY,
            crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
        )?;
        let witness_key = self.candidate_descriptor_rollback_key();
        let rollback = root.read_bounded(
            &witness_key,
            crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
        )?;
        if preparing {
            if live.is_some() || rollback.is_some() {
                return Err(MutationError::RecoveryConflict(
                    "owned candidate vault descriptor destination is not empty".into(),
                ));
            }
            return Ok(());
        }

        let Some(witness) = self.owned_candidate_vault_descriptor_witness else {
            if self.decision == RootTransitionDecision::Committed {
                return Err(MutationError::Invalid(
                    "committed owned candidate vault descriptor has no durable witness identity"
                        .into(),
                ));
            }
            return Ok(());
        };
        let witness_identity = crate::services::twin_events::RegularFileIdentity {
            device: witness.device,
            inode: witness.inode,
        };
        if let Some(rollback) = rollback {
            if rollback != expected
                || root.regular_file_identity(&witness_key)? != Some(witness_identity)
            {
                return Err(MutationError::RecoveryConflict(
                    "owned candidate vault descriptor witness changed".into(),
                ));
            }
        }
        let live_identity =
            root.regular_file_identity(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)?;
        match self.decision {
            RootTransitionDecision::Prepared => {
                if live_identity == Some(witness_identity) && live.as_deref() != Some(expected) {
                    return Err(MutationError::RecoveryConflict(
                        "owned candidate vault descriptor changed".into(),
                    ));
                }
            }
            RootTransitionDecision::Committed => {
                if live.as_deref() != Some(expected) || live_identity != Some(witness_identity) {
                    return Err(MutationError::RecoveryConflict(
                        "committed owned candidate vault descriptor is missing or changed".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn prepared_forward_reattach(
        before: RootAuthorityV1,
        after: RootAuthorityV1,
        authority_binding: ContentDigest,
    ) -> Result<Self, MutationError> {
        let transition = Self {
            schema_version: FORWARD_REATTACH_TRANSITION_SCHEMA_VERSION,
            transaction_id: Uuid::new_v4().to_string(),
            decision: RootTransitionDecision::Prepared,
            authority_binding,
            root_changed: true,
            path_changed: Some(true),
            vault_identity_changed: Some(false),
            forward_only_reattach: Some(true),
            owned_candidate_vault_descriptor: None,
            owned_candidate_vault_descriptor_witness: None,
            rollback_lease: after.lease.clone(),
            before,
            after,
        };
        transition.validate_preparation()?;
        Ok(transition)
    }

    fn is_forward_only_reattach(&self) -> bool {
        self.schema_version == FORWARD_REATTACH_TRANSITION_SCHEMA_VERSION
            && self.forward_only_reattach == Some(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenRouterKeyRefV1 {
    schema_version: u16,
    source: OpenRouterKeySource,
    active_version: Option<String>,
}

#[derive(Clone)]
struct VersionedOpenRouterSecrets {
    store: Arc<dyn SecretStore>,
}

impl VersionedOpenRouterSecrets {
    fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    fn account(version: &str) -> Result<SecretAccount, MutationError> {
        validate_key_version(Some(version))?;
        SecretAccount::openrouter_key(version).map_err(secret_store_error)
    }

    fn put_new(&self, version: &str, secret: &str) -> Result<(), MutationError> {
        let account = Self::account(version)?;
        let secret = SecretBytes::from_slice(secret.as_bytes()).map_err(secret_store_error)?;
        self.store
            .put(&account, &secret)
            .map_err(secret_store_error)
    }

    fn get(&self, version: &str) -> Result<Option<String>, MutationError> {
        let account = Self::account(version)?;
        let Some(secret) = self.store.get(&account).map_err(secret_store_error)? else {
            return Ok(None);
        };
        String::from_utf8(secret.expose().to_vec())
            .map(Some)
            .map_err(|_| MutationError::Invalid("stored OpenRouter secret is not UTF-8".into()))
    }

    fn delete(&self, version: &str) -> Result<(), MutationError> {
        let account = Self::account(version)?;
        self.store.delete(&account).map_err(secret_store_error)
    }
}

fn secret_store_error(error: SecretStoreError) -> MutationError {
    match error {
        SecretStoreError::BackendUnavailable => {
            MutationError::Io("secret store is unavailable".into())
        }
        other => MutationError::Invalid(other.to_string()),
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

#[derive(Clone)]
pub(crate) struct DurableSettingsSnapshot {
    pub(crate) settings: UserSettings,
    pub(crate) settings_generation: ContentDigest,
    pub(crate) active_key_version: Option<String>,
    pub(crate) key_source: OpenRouterKeySource,
    pub(crate) resolved_secret: Option<String>,
}

impl std::fmt::Debug for DurableSettingsSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableSettingsSnapshot")
            .field("settings_generation", &self.settings_generation)
            .field("active_key_version", &self.active_key_version)
            .field("key_source", &self.key_source)
            .field("resolved_secret_present", &self.resolved_secret.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct DurableRootAuthoritySnapshot {
    pub(crate) authority: RootAuthorityV1,
    pub(crate) settings: UserSettings,
    pub(crate) settings_generation: ContentDigest,
    pub(crate) resolved_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetachedStableVault {
    pub(crate) configured_path: std::path::PathBuf,
    pub(crate) root_scope: ContentDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootTransitionFaultPoint {
    AfterPrepared,
    AfterCandidateDescriptorLiveInstall,
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
    AfterRollbackCandidateDescriptor,
    AfterCommittedCandidateDescriptor,
    AfterRecoveryWalDelete,
}

pub(crate) struct RootTransitionStore {
    data_path: std::path::PathBuf,
    data_root: AnchoredRoot,
    config_root: AnchoredRoot,
    settings_key: String,
    authority_binding: ContentDigest,
    secrets: VersionedOpenRouterSecrets,
    fault_once: Mutex<Option<RootTransitionFaultPoint>>,
}

impl RootTransitionStore {
    pub(crate) fn new(
        data_path: impl AsRef<Path>,
        config_path: impl AsRef<Path>,
        secrets: Arc<dyn SecretStore>,
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
            secrets: VersionedOpenRouterSecrets::new(secrets),
            fault_once: Mutex::new(None),
        })
    }

    pub(crate) fn authority_binding(&self) -> ContentDigest {
        self.authority_binding.clone()
    }

    pub(crate) fn detached_stable_vault(
        &self,
    ) -> Result<Option<DetachedStableVault>, MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = self.detached_stable_vault_locked();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn prepare_runtime_vault_path(
        &self,
        configured_vault: &Path,
    ) -> Result<std::path::PathBuf, MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = (|| {
            reject_transition_wal_locked(&self.data_path, &process_lock)?;
            let durable_vault = self
                .read_settings_snapshot()?
                .settings
                .effective_vault_path();
            if durable_vault != configured_vault {
                return Err(MutationError::RecoveryConflict(
                    "runtime vault path does not match durable settings".into(),
                ));
            }
            let stable = self
                .read_optional_lease()?
                .is_some_and(|lease| lease.is_stable());
            if stable {
                crate::services::twin_events::validate_real_directory(
                    configured_vault,
                    "configured stable vault",
                )?;
            } else {
                std::fs::create_dir_all(configured_vault)?;
                crate::services::twin_events::validate_real_directory(
                    configured_vault,
                    "vault runtime directory",
                )?;
            }
            Ok(configured_vault.to_path_buf())
        })();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn reattach_missing_stable_vault(
        &self,
        candidate: &Path,
    ) -> Result<DurableSettingsSnapshot, MutationError> {
        self.reattach_missing_stable_vault_with_metadata(candidate, |path| {
            std::fs::symlink_metadata(path)
        })
    }

    fn reattach_missing_stable_vault_with_metadata(
        &self,
        candidate: &Path,
        metadata: impl FnOnce(&Path) -> std::io::Result<std::fs::Metadata>,
    ) -> Result<DurableSettingsSnapshot, MutationError> {
        let process_lock =
            crate::services::twin_events::acquire_shared_coordinator_process_lock(&self.data_path)?;
        let result = (|| {
            crate::services::twin_events::reject_prepared_stable_migration_locked(
                &self.data_path,
                &process_lock,
            )?;
            let detached = self
                .detached_stable_vault_locked_with_metadata(metadata)?
                .ok_or_else(|| {
                    MutationError::RecoveryConflict(
                        "forward reattach requires a genuinely unavailable stable vault".into(),
                    )
                })?;
            crate::services::twin_events::validate_real_directory(
                candidate,
                "forward reattach candidate vault",
            )?;
            let candidate = std::fs::canonicalize(candidate)?;
            if candidate == detached.configured_path {
                return Err(MutationError::RecoveryConflict(
                    "forward reattach candidate is the unavailable configured path".into(),
                ));
            }
            let candidate_identity =
                crate::services::sync::identity::load_vault_identity(&candidate)?;
            if candidate_identity.root_scope != detached.root_scope {
                return Err(MutationError::RecoveryConflict(
                    "forward reattach candidate has a different vault identity".into(),
                ));
            }

            let current = self.read_settings_snapshot()?;
            let current_lease = self.read_lease()?;
            if !current_lease.is_stable() || current_lease.root_scope != detached.root_scope {
                return Err(MutationError::RecoveryConflict(
                    "forward reattach stable lease changed".into(),
                ));
            }
            let before_path = detached.configured_path.to_str().ok_or_else(|| {
                MutationError::Invalid("detached vault path must be Unicode".into())
            })?;
            let before = RootAuthorityV1 {
                canonical_vault_path: before_path.to_string(),
                root_scope: detached.root_scope.clone(),
                lease: current_lease,
                nonsecret_settings: NonsecretSettingsV1::from_settings(current.settings.clone()),
                openrouter_key_source: current.key_source,
                openrouter_key_version: current.active_key_version.clone(),
            };
            before.validate_detached_stable()?;

            let mut after_settings = current.settings;
            after_settings.vault_path = Some(
                candidate
                    .to_str()
                    .ok_or_else(|| {
                        MutationError::Invalid("reattached vault path must be Unicode".into())
                    })?
                    .to_string(),
            );
            let after = RootAuthorityV1::new_with_key_source(
                &candidate,
                after_settings,
                ActiveMarkdownRootLeaseV1::new_stable(detached.root_scope),
                current.key_source,
                current.active_key_version,
            )?;
            let transition = RootTransitionV1::prepared_forward_reattach(
                before,
                after,
                self.authority_binding.clone(),
            )?;
            self.prepare_transition(&transition)?;
            self.checkpoint(RootTransitionFaultPoint::AfterPrepared)?;
            if self.recover_locked()? != RecoveryWork::RolledForward {
                return Err(MutationError::RecoveryConflict(
                    "forward reattach did not roll forward".into(),
                ));
            }
            self.read_settings_snapshot()
        })();
        process_lock.unlock()?;
        result
    }

    fn detached_stable_vault_locked(&self) -> Result<Option<DetachedStableVault>, MutationError> {
        self.detached_stable_vault_locked_with_metadata(|path| std::fs::symlink_metadata(path))
    }

    fn detached_stable_vault_locked_with_metadata(
        &self,
        metadata: impl FnOnce(&Path) -> std::io::Result<std::fs::Metadata>,
    ) -> Result<Option<DetachedStableVault>, MutationError> {
        if self.read_transition()?.is_some() {
            return Err(MutationError::RecoveryConflict(
                "root-transition-already-pending".into(),
            ));
        }
        let settings = self.read_settings_snapshot()?;
        let Some(lease) = self.read_optional_lease()? else {
            return Ok(None);
        };
        if !lease.is_stable() {
            return Ok(None);
        }
        let configured_path = settings.settings.effective_vault_path();
        match metadata(&configured_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Some(DetachedStableVault {
                    configured_path,
                    root_scope: lease.root_scope,
                }))
            }
            Err(error) => Err(MutationError::Io(error.to_string())),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                Err(MutationError::RecoveryConflict(
                    "configured stable vault path is not a real directory".into(),
                ))
            }
            Ok(_) => {
                crate::services::twin_events::validate_real_directory(
                    &configured_path,
                    "configured stable vault",
                )?;
                let identity =
                    crate::services::sync::identity::load_vault_identity(&configured_path)?;
                if identity.root_scope != lease.root_scope {
                    return Err(MutationError::RecoveryConflict(
                        "configured stable vault descriptor was replaced".into(),
                    ));
                }
                Ok(None)
            }
        }
    }

    fn encoded_transition(&self, transition: &RootTransitionV1) -> Result<Vec<u8>, MutationError> {
        match transition.decision {
            RootTransitionDecision::Prepared => transition.validate_preparation()?,
            RootTransitionDecision::Committed => transition.validate()?,
        }
        self.serialize_transition(transition)
    }

    fn encoded_durable_transition(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<Vec<u8>, MutationError> {
        transition.validate()?;
        self.serialize_transition(transition)
    }

    fn serialize_transition(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<Vec<u8>, MutationError> {
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

    pub(crate) fn install_owned_candidate_vault_descriptor(
        &self,
        transition: &mut RootTransitionV1,
    ) -> Result<(), MutationError> {
        self.require_exact_prepared_transition(transition)?;
        let Some(expected) = transition
            .owned_candidate_descriptor_bytes()?
            .map(<[u8]>::to_vec)
        else {
            return Ok(());
        };
        let root = AnchoredRoot::open(&transition.after.canonical_vault_path)?;
        let witness_key = transition.candidate_descriptor_rollback_key();
        let witness_outcome =
            root.install_no_clobber_with_outcome(&witness_key, "_grafyn", &expected)?;
        if witness_outcome != crate::services::twin_events::NoClobberInstallOutcome::Installed {
            return Err(MutationError::RecoveryConflict(
                "owned candidate vault descriptor witness was already occupied".into(),
            ));
        }
        if root
            .read_bounded(
                &witness_key,
                crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
            )?
            .as_deref()
            != Some(expected.as_slice())
        {
            return Err(MutationError::RecoveryConflict(
                "owned candidate vault descriptor witness changed before ownership was recorded"
                    .into(),
            ));
        }
        let witness_identity = root.regular_file_identity(&witness_key)?.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "owned candidate vault descriptor witness disappeared before ownership was recorded"
                    .into(),
            )
        })?;
        self.record_owned_candidate_descriptor_witness(transition, witness_identity)?;
        // Moving the owned witness makes its absence durable proof that this transaction
        // published the live descriptor, unlike an external same-inode hard-link race.
        root.rename_no_replace(
            &witness_key,
            crate::services::sync::identity::VAULT_DESCRIPTOR_KEY,
            false,
        )?;
        self.checkpoint(RootTransitionFaultPoint::AfterCandidateDescriptorLiveInstall)?;
        if root
            .read_bounded(
                crate::services::sync::identity::VAULT_DESCRIPTOR_KEY,
                crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
            )?
            .as_deref()
            != Some(expected.as_slice())
        {
            return Err(MutationError::RecoveryConflict(
                "owned candidate vault descriptor install lost its no-clobber race".into(),
            ));
        }
        if root.regular_file_identity(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)?
            != Some(witness_identity)
        {
            return Err(MutationError::RecoveryConflict(
                "owned candidate vault descriptor is not linked to its witness".into(),
            ));
        }
        self.record_owned_candidate_descriptor_live_install(transition, witness_identity)?;
        Ok(())
    }

    fn record_owned_candidate_descriptor_witness(
        &self,
        transition: &mut RootTransitionV1,
        identity: crate::services::twin_events::RegularFileIdentity,
    ) -> Result<(), MutationError> {
        self.require_exact_prepared_transition(transition)?;
        if transition.owned_candidate_vault_descriptor.is_none()
            || transition
                .owned_candidate_vault_descriptor_witness
                .is_some()
        {
            return Err(MutationError::Invalid(
                "candidate descriptor witness ownership cannot be recorded in this transition"
                    .into(),
            ));
        }
        let mut claimed = transition.clone();
        claimed.owned_candidate_vault_descriptor_witness =
            Some(OwnedCandidateDescriptorWitnessV1 {
                device: identity.device,
                inode: identity.inode,
                live_installed: false,
            });
        let bytes = self.encoded_durable_transition(&claimed)?;
        self.data_root.put_atomic(ROOT_TRANSITION_KEY, &bytes)?;
        if self.read_transition()?.as_ref() != Some(&claimed) {
            return Err(MutationError::RecoveryConflict(
                "candidate descriptor witness ownership write was not durable".into(),
            ));
        }
        *transition = claimed;
        Ok(())
    }

    fn record_owned_candidate_descriptor_live_install(
        &self,
        transition: &mut RootTransitionV1,
        identity: crate::services::twin_events::RegularFileIdentity,
    ) -> Result<(), MutationError> {
        self.require_exact_prepared_transition(transition)?;
        let Some(witness) = transition.owned_candidate_vault_descriptor_witness else {
            return Err(MutationError::Invalid(
                "candidate descriptor live install has no durable witness identity".into(),
            ));
        };
        if witness.live_installed
            || witness.device != identity.device
            || witness.inode != identity.inode
        {
            return Err(MutationError::Invalid(
                "candidate descriptor live install does not match its prepared witness".into(),
            ));
        }
        let expected = transition
            .owned_candidate_descriptor_bytes()?
            .ok_or_else(|| MutationError::Invalid("owned vault descriptor is missing".into()))?
            .to_vec();
        let root = AnchoredRoot::open(&transition.after.canonical_vault_path)?;
        if root
            .read_bounded(
                crate::services::sync::identity::VAULT_DESCRIPTOR_KEY,
                crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
            )?
            .as_deref()
            != Some(expected.as_slice())
            || root.regular_file_identity(crate::services::sync::identity::VAULT_DESCRIPTOR_KEY)?
                != Some(identity)
        {
            return Err(MutationError::RecoveryConflict(
                "candidate descriptor live install changed before ownership was recorded".into(),
            ));
        }
        let mut claimed = transition.clone();
        claimed
            .owned_candidate_vault_descriptor_witness
            .as_mut()
            .expect("witness was checked")
            .live_installed = true;
        let bytes = self.encoded_durable_transition(&claimed)?;
        self.data_root.put_atomic(ROOT_TRANSITION_KEY, &bytes)?;
        if self.read_transition()?.as_ref() != Some(&claimed) {
            return Err(MutationError::RecoveryConflict(
                "candidate descriptor live-install ownership write was not durable".into(),
            ));
        }
        *transition = claimed;
        Ok(())
    }

    fn remove_owned_candidate_vault_descriptor(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        let Some(expected) = transition.owned_candidate_descriptor_bytes()? else {
            return Ok(());
        };
        let Some(witness) = transition.owned_candidate_vault_descriptor_witness else {
            return Ok(());
        };
        let witness_identity = crate::services::twin_events::RegularFileIdentity {
            device: witness.device,
            inode: witness.inode,
        };
        let candidate = Path::new(&transition.after.canonical_vault_path);
        match std::fs::symlink_metadata(candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(MutationError::RecoveryConflict(
                    "owned candidate vault path is no longer a real directory".into(),
                ));
            }
            Ok(_) => {}
        }
        let root = AnchoredRoot::open(candidate)?;
        let witness_key = transition.candidate_descriptor_rollback_key();
        let durable_witness = root.read_bounded(
            &witness_key,
            crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
        )?;
        // The move can be durable before the follow-up WAL flag update is.
        if witness.live_installed || durable_witness.is_none() {
            let _ = root.quarantine_regular_file_if_identity(
                crate::services::sync::identity::VAULT_DESCRIPTOR_KEY,
                witness_identity,
            )?;
        }
        if let Some(durable_witness) = durable_witness {
            if durable_witness != expected
                || !root.quarantine_regular_file_if_identity(&witness_key, witness_identity)?
            {
                return Err(MutationError::RecoveryConflict(
                    "owned candidate vault descriptor witness changed during rollback".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn finalize_committed_candidate_vault_descriptor(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        if transition.decision != RootTransitionDecision::Committed
            || self.read_transition()?.as_ref() != Some(transition)
        {
            return Err(MutationError::RecoveryConflict(
                "root-transition-committed-cas-failed".into(),
            ));
        }
        let Some(expected) = transition.owned_candidate_descriptor_bytes()? else {
            return Ok(());
        };
        let witness = transition
            .owned_candidate_vault_descriptor_witness
            .ok_or_else(|| {
                MutationError::Invalid(
                    "committed owned candidate vault descriptor has no durable witness identity"
                        .into(),
                )
            })?;
        let witness_identity = crate::services::twin_events::RegularFileIdentity {
            device: witness.device,
            inode: witness.inode,
        };
        let root = AnchoredRoot::open(&transition.after.canonical_vault_path)?;
        let witness_key = transition.candidate_descriptor_rollback_key();
        let Some(durable_witness) = root.read_bounded(
            &witness_key,
            crate::services::sync::identity::VAULT_DESCRIPTOR_LIMIT,
        )?
        else {
            return Ok(());
        };
        if durable_witness != expected {
            return Err(MutationError::RecoveryConflict(
                "committed owned candidate vault descriptor witness is invalid".into(),
            ));
        }
        root.quarantine_regular_file_if_identity(&witness_key, witness_identity)?
            .then_some(())
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "committed owned candidate vault descriptor witness changed during cleanup"
                        .into(),
                )
            })
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
                            None => {
                                self.stage_secret(LEGACY_MIGRATION_KEY_VERSION, &legacy_secret)?
                            }
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
        if !matches!(
            lease.schema_version,
            LEASE_SCHEMA_VERSION | STABLE_LEASE_SCHEMA_VERSION
        ) || !is_canonical_non_nil_uuid(&lease.epoch_uuid)
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
        self.remove_owned_candidate_vault_descriptor(transition)?;
        self.checkpoint(RootTransitionFaultPoint::AfterRollbackCandidateDescriptor)?;
        self.remove_transition()
    }

    fn require_exact_prepared_transition(
        &self,
        transition: &RootTransitionV1,
    ) -> Result<(), MutationError> {
        if transition.is_forward_only_reattach()
            || transition.decision != RootTransitionDecision::Prepared
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

        if transition.is_forward_only_reattach() {
            self.write_lease(&transition.after.lease)?;
            self.checkpoint(RootTransitionFaultPoint::AfterLease)?;
            self.write_settings(&transition.after.nonsecret_settings)?;
            self.checkpoint(RootTransitionFaultPoint::AfterSettings)?;
            self.write_key_authority(
                transition.after.openrouter_key_source,
                transition.after.openrouter_key_version.as_deref(),
            )?;
            self.checkpoint(RootTransitionFaultPoint::AfterKeyRef)?;
            self.remove_transition()?;
            self.checkpoint(RootTransitionFaultPoint::AfterRecoveryWalDelete)?;
            return Ok(RecoveryWork::RolledForward);
        }

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
                self.remove_owned_candidate_vault_descriptor(&transition)?;
                self.checkpoint(RootTransitionFaultPoint::AfterRollbackCandidateDescriptor)?;
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
                self.finalize_committed_candidate_vault_descriptor(&transition)?;
                self.checkpoint(RootTransitionFaultPoint::AfterCommittedCandidateDescriptor)?;
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
        self.read_optional_lease()?
            .ok_or_else(|| MutationError::Invalid("active root lease is missing".into()))
    }

    fn read_optional_lease(&self) -> Result<Option<ActiveMarkdownRootLeaseV1>, MutationError> {
        let Some(bytes) = self
            .data_root
            .read_bounded(ACTIVE_ROOT_LEASE_KEY, KEY_REF_LIMIT)?
        else {
            return Ok(None);
        };
        parse_active_root_lease(&bytes).map(Some)
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
        SECRET_STORE_SERVICE,
        VERSIONED_KEY_PREFIX,
    ] {
        domain.extend_from_slice(&(value.len() as u64).to_be_bytes());
        domain.extend_from_slice(value.as_bytes());
    }
    Ok(crate::services::twin_events::digest_bytes(&domain))
}

fn authority_scope_for(
    canonical_vault_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<ContentDigest, MutationError> {
    if lease.is_stable() {
        Ok(crate::services::sync::identity::load_vault_identity(canonical_vault_path)?.root_scope)
    } else {
        root_identity_for_path(canonical_vault_path)
    }
}

fn same_nonsecret_settings_except_vault(
    before: &NonsecretSettingsV1,
    after: &NonsecretSettingsV1,
) -> bool {
    let mut before = before.clone();
    let mut after = after.clone();
    before.vault_path = None;
    after.vault_path = None;
    before == after
}

fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn is_canonical_non_nil_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .is_ok_and(|uuid| !uuid.is_nil() && uuid.hyphenated().to_string() == value)
}

fn parse_active_root_lease(bytes: &[u8]) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let lease: ActiveMarkdownRootLeaseV1 = serde_json::from_slice(bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid root lease: {error}")))?;
    if !matches!(
        lease.schema_version,
        LEASE_SCHEMA_VERSION | STABLE_LEASE_SCHEMA_VERSION
    ) || !is_canonical_non_nil_uuid(&lease.epoch_uuid)
    {
        return Err(MutationError::Invalid("invalid active root lease".into()));
    }
    Ok(lease)
}

#[cfg(test)]
#[path = "root_transition_tests.rs"]
mod tests;
