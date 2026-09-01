use crate::models::runtime::{
    RuntimeCapabilitiesV1, RuntimeFeatureStatusV1, RuntimeKind, RuntimeStatusV1, RuntimeVaultKind,
    RuntimeVaultStatusV1, RUNTIME_STATUS_SCHEMA_VERSION,
};
use crate::services::sync::secrets::{SecretAccount, SecretBytes, SecretStore, SecretStoreError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const APP_DIRECTORY_NAME: &str = "Grafyn";
const SHARE_DIRECTORY_NAME: &str = "grafyn-share-v1";

#[derive(Debug, Default)]
pub(crate) struct UnavailableSecretStore;

impl SecretStore for UnavailableSecretStore {
    fn put(&self, _account: &SecretAccount, _secret: &SecretBytes) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }

    fn get(&self, _account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }

    fn delete(&self, _account: &SecretAccount) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::BackendUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimePaths {
    pub(crate) config_dir: PathBuf,
    pub(crate) data_dir: PathBuf,
    pub(crate) vault_dir: PathBuf,
    pub(crate) cache_dir: PathBuf,
    pub(crate) share_dir: PathBuf,
}

impl RuntimePaths {
    pub(crate) fn desktop(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        vault_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        let cache_dir = cache_dir.into();
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            vault_dir: vault_dir.into(),
            share_dir: cache_dir.join(SHARE_DIRECTORY_NAME),
            cache_dir,
        }
    }

    #[cfg(desktop)]
    pub(crate) fn desktop_system() -> Self {
        let config_dir = dirs::config_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIRECTORY_NAME);
        let settings = crate::models::settings::UserSettings::default();
        let data_dir = settings.effective_data_path();
        let vault_dir = settings.effective_vault_path();
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| data_dir.join("cache"))
            .join(APP_DIRECTORY_NAME);
        Self::desktop(config_dir, data_dir, vault_dir, cache_dir)
    }

    pub(crate) fn android(
        app_data_dir: impl Into<PathBuf>,
        app_cache_dir: impl Into<PathBuf>,
    ) -> Self {
        let private_root = app_data_dir.into().join(APP_DIRECTORY_NAME);
        let cache_dir = app_cache_dir.into().join(APP_DIRECTORY_NAME);
        Self {
            config_dir: private_root.join("config"),
            data_dir: private_root.join("data"),
            vault_dir: private_root.join("vault"),
            share_dir: cache_dir.join(SHARE_DIRECTORY_NAME),
            cache_dir,
        }
    }

    pub(crate) fn prepare(&self) -> Result<(), String> {
        for (label, path) in [
            ("config", &self.config_dir),
            ("data", &self.data_dir),
            ("vault", &self.vault_dir),
            ("cache", &self.cache_dir),
            ("share", &self.share_dir),
        ] {
            prepare_real_directory(path, label)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeBootstrap {
    pub(crate) kind: RuntimeKind,
    pub(crate) paths: RuntimePaths,
    pub(crate) secret_store: Arc<dyn SecretStore>,
    pub(crate) secure_secrets: RuntimeFeatureStatusV1,
    pub(crate) native_image_share: RuntimeFeatureStatusV1,
    startup_error: Option<&'static str>,
}

impl RuntimeBootstrap {
    pub(crate) fn new(
        kind: RuntimeKind,
        paths: RuntimePaths,
        secret_store: Arc<dyn SecretStore>,
        secure_secrets: RuntimeFeatureStatusV1,
        native_image_share: RuntimeFeatureStatusV1,
    ) -> Self {
        Self {
            kind,
            paths,
            secret_store,
            secure_secrets,
            native_image_share,
            startup_error: None,
        }
    }

    pub(crate) fn with_startup_error(mut self, error: &'static str) -> Self {
        self.startup_error = Some(error);
        self
    }

    pub(crate) fn startup_error(&self) -> Option<&'static str> {
        self.startup_error
    }

    #[cfg(desktop)]
    pub(crate) fn desktop() -> Self {
        Self::new(
            RuntimeKind::Desktop,
            RuntimePaths::desktop_system(),
            Arc::new(crate::services::sync::secrets::KeyringSecretStore),
            RuntimeFeatureStatusV1::ready(),
            RuntimeFeatureStatusV1::unavailable(
                "desktop_save_as",
                "Desktop exports use the governed Save As boundary.",
            ),
        )
    }

    pub(crate) fn status(&self) -> RuntimeStatusV1 {
        self.status_for_state(true, self.secure_secrets.is_ready())
    }

    pub(crate) fn status_for_state(
        &self,
        canonical_runtime_available: bool,
        sync_available: bool,
    ) -> RuntimeStatusV1 {
        let secure_secrets = if canonical_runtime_available {
            self.secure_secrets.clone()
        } else {
            RuntimeFeatureStatusV1::unavailable(
                "canonical_runtime_unavailable",
                "Secure secrets are unavailable while the canonical runtime is unavailable.",
            )
        };
        let native_image_share = if !canonical_runtime_available {
            RuntimeFeatureStatusV1::unavailable(
                "canonical_runtime_unavailable",
                "Native image sharing is unavailable while the canonical runtime is unavailable.",
            )
        } else if self.kind == RuntimeKind::Android && self.native_image_share.is_ready() {
            RuntimeFeatureStatusV1::unavailable(
                "android_image_receipt_unavailable",
                "Native image sharing is unavailable until Android can receive a generated image receipt.",
            )
        } else {
            self.native_image_share.clone()
        };
        let secure = secure_secrets.is_ready();
        let native_share = native_image_share.is_ready();
        let vault_available =
            canonical_runtime_available && is_real_directory(&self.paths.vault_dir);
        let (capabilities, vault_kind) = match self.kind {
            RuntimeKind::Desktop => (
                RuntimeCapabilitiesV1 {
                    notes_read: canonical_runtime_available,
                    notes_write: canonical_runtime_available,
                    recall: canonical_runtime_available,
                    twin_review: canonical_runtime_available,
                    twin_chat: canonical_runtime_available && secure,
                    linear_canvas: canonical_runtime_available,
                    image_generation: canonical_runtime_available && secure,
                    native_image_share: false,
                    sync: canonical_runtime_available && secure && sync_available,
                    spatial_canvas: canonical_runtime_available,
                    native_vault_picker: canonical_runtime_available,
                    import_by_path: canonical_runtime_available,
                    local_ollama: canonical_runtime_available,
                    mcp: canonical_runtime_available,
                    vault_migration: canonical_runtime_available,
                    optimizer_admin: canonical_runtime_available,
                    desktop_updater: canonical_runtime_available,
                },
                RuntimeVaultKind::UserSelected,
            ),
            RuntimeKind::Android => (
                RuntimeCapabilitiesV1 {
                    notes_read: canonical_runtime_available,
                    notes_write: canonical_runtime_available,
                    recall: canonical_runtime_available,
                    twin_review: canonical_runtime_available,
                    twin_chat: canonical_runtime_available && secure,
                    linear_canvas: canonical_runtime_available,
                    image_generation: false,
                    native_image_share: canonical_runtime_available && native_share,
                    sync: canonical_runtime_available && secure && sync_available,
                    spatial_canvas: false,
                    native_vault_picker: false,
                    import_by_path: false,
                    local_ollama: false,
                    mcp: false,
                    vault_migration: false,
                    optimizer_admin: false,
                    desktop_updater: false,
                },
                RuntimeVaultKind::AppPrivate,
            ),
        };
        let mut diagnostics = Vec::new();
        if self.kind == RuntimeKind::Android {
            if let Some(diagnostic) = secure_secrets.diagnostic() {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = native_image_share.diagnostic() {
                diagnostics.push(diagnostic);
            }
            if !vault_available {
                diagnostics.push(crate::models::runtime::RuntimeDiagnosticV1 {
                    code: "app_private_vault_unavailable".to_string(),
                    message: "The app-private vault is unavailable.".to_string(),
                });
            }
            if !canonical_runtime_available {
                diagnostics.push(crate::models::runtime::RuntimeDiagnosticV1 {
                    code: "canonical_runtime_unavailable".to_string(),
                    message: "The canonical local runtime is unavailable.".to_string(),
                });
            }
        }

        RuntimeStatusV1 {
            schema_version: RUNTIME_STATUS_SCHEMA_VERSION,
            runtime: self.kind,
            capabilities,
            vault: RuntimeVaultStatusV1 {
                kind: vault_kind,
                available: vault_available,
            },
            secure_secrets,
            native_image_share,
            diagnostics,
        }
    }
}

pub(crate) fn mask_canonical_runtime_unavailable(mut status: RuntimeStatusV1) -> RuntimeStatusV1 {
    status.capabilities = RuntimeCapabilitiesV1::default();
    status.vault.available = false;
    status.secure_secrets = RuntimeFeatureStatusV1::unavailable(
        "canonical_runtime_unavailable",
        "Secure secrets are unavailable while the canonical runtime is unavailable.",
    );
    status.native_image_share = RuntimeFeatureStatusV1::unavailable(
        "canonical_runtime_unavailable",
        "Native image sharing is unavailable while the canonical runtime is unavailable.",
    );
    status.diagnostics.clear();
    if status.runtime == RuntimeKind::Android {
        status.diagnostics.push(
            status
                .secure_secrets
                .diagnostic()
                .expect("unavailable status"),
        );
        status.diagnostics.push(
            status
                .native_image_share
                .diagnostic()
                .expect("unavailable status"),
        );
        status
            .diagnostics
            .push(crate::models::runtime::RuntimeDiagnosticV1 {
                code: "app_private_vault_unavailable".to_string(),
                message: "The app-private vault is unavailable.".to_string(),
            });
        status
            .diagnostics
            .push(crate::models::runtime::RuntimeDiagnosticV1 {
                code: "canonical_runtime_unavailable".to_string(),
                message: "The canonical local runtime is unavailable.".to_string(),
            });
    }
    status
}

fn prepare_real_directory(path: &Path, label: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!("{label} root is not a real directory"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path)
                .map_err(|error| format!("Failed to create {label} root: {error}"))?;
        }
        Err(error) => {
            return Err(format!("Failed to inspect {label} root: {error}"));
        }
    }
    if !is_real_directory(path) {
        return Err(format!("{label} root is unavailable"));
    }
    Ok(())
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}
