use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use zeroize::Zeroizing;

use crate::services::sync::secrets::{
    SecretAccount, SecretBytes, SecretStore, SecretStoreError, MAX_SECRET_BYTES,
};

#[cfg(target_os = "android")]
pub(crate) const ANDROID_PLUGIN_IDENTIFIER: &str = "com.grafyn.app";
pub(crate) const SHARE_DIRECTORY_NAME: &str = "grafyn-share-v1";
pub(crate) const SHARE_DIRECTORY_PARENT: &str = "Grafyn";
pub(crate) const MAX_SHARE_FILENAME_BYTES: usize = 128;
pub(crate) const MAX_SHARE_IMAGE_BYTES: usize = 24 * 1024 * 1024;
const STALE_SHARE_FILE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidBridgeError {
    InvalidFilename,
    InvalidMime,
    InvalidImage,
    FileUnavailable,
    BackendUnavailable,
    InvalidHealth,
}

impl fmt::Display for AndroidBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFilename => "android_share_invalid_filename",
            Self::InvalidMime => "android_share_invalid_mime",
            Self::InvalidImage => "android_share_invalid_image",
            Self::FileUnavailable => "android_share_file_unavailable",
            Self::BackendUnavailable => "android_share_backend_unavailable",
            Self::InvalidHealth => "android_native_health_invalid",
        })
    }
}

impl std::error::Error for AndroidBridgeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShareDescriptor {
    file_name: String,
    mime: String,
}

impl ShareDescriptor {
    pub(crate) fn parse(file_name: &str, mime: &str) -> Result<Self, AndroidBridgeError> {
        if file_name.is_empty()
            || file_name.len() > MAX_SHARE_FILENAME_BYTES
            || !file_name.is_ascii()
        {
            return Err(AndroidBridgeError::InvalidFilename);
        }

        let extension = match mime {
            "image/png" => ".png",
            "image/jpeg" => ".jpg",
            "image/webp" => ".webp",
            _ => return Err(AndroidBridgeError::InvalidMime),
        };
        if !file_name.ends_with(extension) {
            return Err(AndroidBridgeError::InvalidMime);
        }
        let Some(generated_id) = file_name
            .strip_suffix(extension)
            .and_then(|stem| stem.strip_prefix("grafyn-"))
        else {
            return Err(AndroidBridgeError::InvalidFilename);
        };
        if !is_canonical_v4_uuid(generated_id) {
            return Err(AndroidBridgeError::InvalidFilename);
        }

        Ok(Self {
            file_name: file_name.to_owned(),
            mime: mime.to_owned(),
        })
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }

    pub(crate) fn mime(&self) -> &str {
        &self.mime
    }
}

fn is_canonical_v4_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
        && bytes
            .iter()
            .all(|byte| !byte.is_ascii_alphabetic() || byte.is_ascii_lowercase())
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

#[derive(Debug)]
pub(crate) struct StagedShareFile {
    path: PathBuf,
    descriptor: ShareDescriptor,
}

impl StagedShareFile {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn file_name(&self) -> &str {
        self.descriptor.file_name()
    }

    pub(crate) fn mime(&self) -> &str {
        self.descriptor.mime()
    }
}

pub(crate) fn stage_share_file(
    share_root: &Path,
    descriptor: &ShareDescriptor,
    image: &[u8],
) -> Result<StagedShareFile, AndroidBridgeError> {
    if image.is_empty() || image.len() > MAX_SHARE_IMAGE_BYTES {
        return Err(AndroidBridgeError::InvalidImage);
    }

    std::fs::create_dir_all(share_root).map_err(|_| AndroidBridgeError::FileUnavailable)?;
    let path = share_root.join(descriptor.file_name());
    if path.parent() != Some(share_root) {
        return Err(AndroidBridgeError::InvalidFilename);
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| AndroidBridgeError::FileUnavailable)?;
    if file
        .write_all(image)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(AndroidBridgeError::FileUnavailable);
    }

    Ok(StagedShareFile {
        path,
        descriptor: descriptor.clone(),
    })
}

fn share_mime_for_file_name(file_name: &str) -> Option<&'static str> {
    if file_name.ends_with(".png") {
        Some("image/png")
    } else if file_name.ends_with(".jpg") {
        Some("image/jpeg")
    } else if file_name.ends_with(".webp") {
        Some("image/webp")
    } else {
        None
    }
}

fn sweep_stale_share_files_at(
    share_root: &Path,
    now: SystemTime,
) -> Result<usize, AndroidBridgeError> {
    let entries = std::fs::read_dir(share_root).map_err(|_| AndroidBridgeError::FileUnavailable)?;
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(mime) = share_mime_for_file_name(&file_name) else {
            continue;
        };
        if ShareDescriptor::parse(&file_name, mime).is_err() {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let is_stale = now
            .duration_since(modified)
            .is_ok_and(|age| age >= STALE_SHARE_FILE_AGE);
        if is_stale && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativePutStatus {
    Stored,
    AlreadyExists,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeBridgeFailure {
    InvalidAccount,
    InvalidSecret,
    SecretTooLarge,
    AlreadyExists,
    CorruptSecret,
    BackendUnavailable,
}

impl NativeBridgeFailure {
    pub(crate) fn from_code(code: &str) -> Self {
        match code {
            "invalid_account" => Self::InvalidAccount,
            "invalid_secret" => Self::InvalidSecret,
            "secret_too_large" => Self::SecretTooLarge,
            "already_exists" => Self::AlreadyExists,
            "corrupt_secret" => Self::CorruptSecret,
            _ => Self::BackendUnavailable,
        }
    }

    pub(crate) fn secret_error(self) -> SecretStoreError {
        match self {
            Self::InvalidAccount => SecretStoreError::InvalidAccount,
            Self::InvalidSecret => SecretStoreError::InvalidSecret,
            Self::SecretTooLarge => SecretStoreError::SecretTooLarge,
            Self::AlreadyExists => SecretStoreError::AlreadyExists,
            Self::CorruptSecret => SecretStoreError::CorruptSecret,
            Self::BackendUnavailable => SecretStoreError::BackendUnavailable,
        }
    }
}

pub(crate) trait NativeSecretBackend: Send + Sync {
    fn put_secret(
        &self,
        account: &str,
        secret_base64: &str,
    ) -> Result<NativePutStatus, NativeBridgeFailure>;
    fn get_secret(&self, account: &str) -> Result<Option<Zeroizing<String>>, NativeBridgeFailure>;
    fn delete_secret(&self, account: &str) -> Result<(), NativeBridgeFailure>;
}

pub(crate) struct AndroidSecretStore<B: NativeSecretBackend> {
    backend: Arc<B>,
}

impl<B: NativeSecretBackend> AndroidSecretStore<B> {
    pub(crate) fn new(backend: Arc<B>) -> Self {
        Self { backend }
    }
}

impl<B: NativeSecretBackend> fmt::Debug for AndroidSecretStore<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AndroidSecretStore([REDACTED])")
    }
}

impl<B: NativeSecretBackend> SecretStore for AndroidSecretStore<B> {
    fn put(&self, account: &SecretAccount, secret: &SecretBytes) -> Result<(), SecretStoreError> {
        let encoded = Zeroizing::new(STANDARD.encode(secret.expose()));
        match self
            .backend
            .put_secret(account.as_str(), encoded.as_str())
            .map_err(NativeBridgeFailure::secret_error)?
        {
            NativePutStatus::Stored => Ok(()),
            NativePutStatus::AlreadyExists => Err(SecretStoreError::AlreadyExists),
        }
    }

    fn get(&self, account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        let Some(encoded) = self
            .backend
            .get_secret(account.as_str())
            .map_err(NativeBridgeFailure::secret_error)?
        else {
            return Ok(None);
        };
        if encoded.is_empty() || encoded.len() > ((MAX_SECRET_BYTES + 2) / 3) * 4 {
            return Err(SecretStoreError::CorruptSecret);
        }
        let decoded = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(|_| SecretStoreError::CorruptSecret)?,
        );
        if STANDARD.encode(decoded.as_slice()) != encoded.as_str() {
            return Err(SecretStoreError::CorruptSecret);
        }
        SecretBytes::from_slice(decoded.as_slice())
            .map(Some)
            .map_err(|_| SecretStoreError::CorruptSecret)
    }

    fn delete(&self, account: &SecretAccount) -> Result<(), SecretStoreError> {
        self.backend
            .delete_secret(account.as_str())
            .map_err(NativeBridgeFailure::secret_error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeHealth {
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AndroidBridgeHealth {
    pub(crate) secure_secrets: NativeHealth,
    pub(crate) native_image_share: NativeHealth,
}

fn parse_native_health(
    overall: &str,
    secure_secrets: &str,
    native_image_share: &str,
) -> Result<AndroidBridgeHealth, AndroidBridgeError> {
    if secure_secrets == "corrupt" {
        if overall != "fatal" || !matches!(native_image_share, "ready" | "unavailable") {
            return Err(AndroidBridgeError::InvalidHealth);
        }
        return Err(AndroidBridgeError::InvalidHealth);
    }

    fn capability(value: &str) -> Result<NativeHealth, AndroidBridgeError> {
        match value {
            "ready" => Ok(NativeHealth::Ready),
            "unavailable" => Ok(NativeHealth::Unavailable),
            _ => Err(AndroidBridgeError::InvalidHealth),
        }
    }

    let health = AndroidBridgeHealth {
        secure_secrets: capability(secure_secrets)?,
        native_image_share: capability(native_image_share)?,
    };
    let expected_overall = if health.secure_secrets == NativeHealth::Ready
        && health.native_image_share == NativeHealth::Ready
    {
        "ready"
    } else {
        "degraded"
    };
    if overall != expected_overall {
        return Err(AndroidBridgeError::InvalidHealth);
    }
    Ok(health)
}

fn fail_closed_bridge_health(
    native_health: Result<AndroidBridgeHealth, AndroidBridgeError>,
    share_root_available: bool,
) -> Result<AndroidBridgeHealth, AndroidBridgeError> {
    let mut health = native_health?;
    if !share_root_available {
        health.native_image_share = NativeHealth::Unavailable;
    }
    Ok(health)
}

fn unavailable_native_bridge_health() -> AndroidBridgeHealth {
    AndroidBridgeHealth {
        secure_secrets: NativeHealth::Unavailable,
        native_image_share: NativeHealth::Unavailable,
    }
}

fn share_error_from_status(status: &str) -> AndroidBridgeError {
    match status {
        "invalid_filename" => AndroidBridgeError::InvalidFilename,
        "invalid_mime" => AndroidBridgeError::InvalidMime,
        "invalid_image" => AndroidBridgeError::InvalidImage,
        "file_unavailable" => AndroidBridgeError::FileUnavailable,
        _ => AndroidBridgeError::BackendUnavailable,
    }
}

fn parse_share_launch_status(status: &str) -> Result<(), AndroidBridgeError> {
    if status == "share_sheet_opened" {
        Ok(())
    } else {
        Err(share_error_from_status(status))
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
    use tauri::{Manager, Runtime};

    const PLUGIN_CLASS: &str = "GrafynAndroidPlugin";

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SecretPutPayload<'a> {
        account: &'a str,
        secret_base64: &'a str,
    }

    #[derive(Serialize)]
    struct SecretAccountPayload<'a> {
        account: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SharePayload<'a> {
        file_name: &'a str,
        mime: &'a str,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct NativeStatusReply {
        status: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct NativeGetReply {
        status: String,
        #[serde(default)]
        secret_base64: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct NativeHealthReply {
        status: String,
        secure_secrets: String,
        native_image_share: String,
    }

    struct TauriNativeBackend<R: Runtime> {
        handle: PluginHandle<R>,
    }

    enum AndroidBackend<R: Runtime> {
        Ready(TauriNativeBackend<R>),
        Unavailable,
    }

    impl<R: Runtime> TauriNativeBackend<R> {
        fn invoke_status(
            &self,
            command: &str,
            payload: impl Serialize,
        ) -> Result<String, NativeBridgeFailure> {
            self.handle
                .run_mobile_plugin::<NativeStatusReply>(command, payload)
                .map(|reply| reply.status)
                .map_err(|_| NativeBridgeFailure::BackendUnavailable)
        }

        fn share(&self, staged: &StagedShareFile) -> Result<(), AndroidBridgeError> {
            let status = self
                .invoke_status(
                    "shareImage",
                    SharePayload {
                        file_name: staged.file_name(),
                        mime: staged.mime(),
                    },
                )
                .map_err(|_| AndroidBridgeError::BackendUnavailable)?;
            parse_share_launch_status(&status)
        }

        fn health(&self) -> Result<AndroidBridgeHealth, AndroidBridgeError> {
            let reply = self
                .handle
                .run_mobile_plugin::<NativeHealthReply>("health", serde_json::json!({}))
                .map_err(|_| AndroidBridgeError::InvalidHealth)?;
            parse_native_health(
                &reply.status,
                &reply.secure_secrets,
                &reply.native_image_share,
            )
        }
    }

    impl<R: Runtime> NativeSecretBackend for TauriNativeBackend<R> {
        fn put_secret(
            &self,
            account: &str,
            secret_base64: &str,
        ) -> Result<NativePutStatus, NativeBridgeFailure> {
            match self.invoke_status(
                "putSecret",
                SecretPutPayload {
                    account,
                    secret_base64,
                },
            )? {
                status if status == "stored" => Ok(NativePutStatus::Stored),
                status if status == "already_exists" => Ok(NativePutStatus::AlreadyExists),
                status => Err(NativeBridgeFailure::from_code(&status)),
            }
        }

        fn get_secret(
            &self,
            account: &str,
        ) -> Result<Option<Zeroizing<String>>, NativeBridgeFailure> {
            let reply = self
                .handle
                .run_mobile_plugin::<NativeGetReply>("getSecret", SecretAccountPayload { account })
                .map_err(|_| NativeBridgeFailure::BackendUnavailable)?;
            match (reply.status.as_str(), reply.secret_base64) {
                ("found", Some(secret)) => Ok(Some(Zeroizing::new(secret))),
                ("missing", None) => Ok(None),
                ("found" | "missing", _) => Err(NativeBridgeFailure::CorruptSecret),
                (status, _) => Err(NativeBridgeFailure::from_code(status)),
            }
        }

        fn delete_secret(&self, account: &str) -> Result<(), NativeBridgeFailure> {
            match self.invoke_status("deleteSecret", SecretAccountPayload { account })? {
                status if status == "deleted" => Ok(()),
                status => Err(NativeBridgeFailure::from_code(&status)),
            }
        }
    }

    impl<R: Runtime> AndroidBackend<R> {
        fn share(&self, staged: &StagedShareFile) -> Result<(), AndroidBridgeError> {
            match self {
                Self::Ready(backend) => backend.share(staged),
                Self::Unavailable => Err(AndroidBridgeError::BackendUnavailable),
            }
        }

        fn health(&self) -> Result<AndroidBridgeHealth, AndroidBridgeError> {
            match self {
                Self::Ready(backend) => backend.health(),
                Self::Unavailable => Ok(unavailable_native_bridge_health()),
            }
        }
    }

    impl<R: Runtime> NativeSecretBackend for AndroidBackend<R> {
        fn put_secret(
            &self,
            account: &str,
            secret_base64: &str,
        ) -> Result<NativePutStatus, NativeBridgeFailure> {
            match self {
                Self::Ready(backend) => backend.put_secret(account, secret_base64),
                Self::Unavailable => Err(NativeBridgeFailure::BackendUnavailable),
            }
        }

        fn get_secret(
            &self,
            account: &str,
        ) -> Result<Option<Zeroizing<String>>, NativeBridgeFailure> {
            match self {
                Self::Ready(backend) => backend.get_secret(account),
                Self::Unavailable => Err(NativeBridgeFailure::BackendUnavailable),
            }
        }

        fn delete_secret(&self, account: &str) -> Result<(), NativeBridgeFailure> {
            match self {
                Self::Ready(backend) => backend.delete_secret(account),
                Self::Unavailable => Err(NativeBridgeFailure::BackendUnavailable),
            }
        }
    }

    pub(crate) struct AndroidBridge<R: Runtime> {
        backend: Arc<AndroidBackend<R>>,
        share_root: Option<PathBuf>,
    }

    impl<R: Runtime> fmt::Debug for AndroidBridge<R> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("AndroidBridge([NATIVE])")
        }
    }

    impl<R: Runtime> AndroidBridge<R> {
        pub(crate) fn secret_store(&self) -> Arc<dyn SecretStore> {
            Arc::new(AndroidSecretStore::new(self.backend.clone()))
        }

        pub(crate) fn share_root(&self) -> Option<&Path> {
            self.share_root.as_deref()
        }

        pub(crate) fn health(&self) -> Result<AndroidBridgeHealth, AndroidBridgeError> {
            fail_closed_bridge_health(self.backend.health(), self.share_root.is_some())
        }

        pub(crate) fn stage_and_share_generated_image(
            &self,
            file_name: &str,
            mime: &str,
            image: &[u8],
        ) -> Result<(), AndroidBridgeError> {
            let descriptor = ShareDescriptor::parse(file_name, mime)?;
            let share_root = self
                .share_root
                .as_deref()
                .ok_or(AndroidBridgeError::FileUnavailable)?;
            sweep_stale_share_files_at(share_root, SystemTime::now())?;
            let staged = stage_share_file(share_root, &descriptor, image)?;
            if let Err(error) = self.backend.share(&staged) {
                let _ = std::fs::remove_file(staged.path());
                return Err(error);
            }
            Ok(())
        }
    }

    fn prepare_share_root(cache_root: &Path) -> Result<PathBuf, AndroidBridgeError> {
        std::fs::create_dir_all(cache_root).map_err(|_| AndroidBridgeError::FileUnavailable)?;
        let canonical_cache =
            std::fs::canonicalize(cache_root).map_err(|_| AndroidBridgeError::FileUnavailable)?;
        let requested = canonical_cache
            .join(SHARE_DIRECTORY_PARENT)
            .join(SHARE_DIRECTORY_NAME);
        std::fs::create_dir_all(&requested).map_err(|_| AndroidBridgeError::FileUnavailable)?;
        let canonical_share =
            std::fs::canonicalize(&requested).map_err(|_| AndroidBridgeError::FileUnavailable)?;
        if canonical_share != requested {
            return Err(AndroidBridgeError::FileUnavailable);
        }
        sweep_stale_share_files_at(&canonical_share, SystemTime::now())?;
        Ok(canonical_share)
    }

    pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("grafyn-android")
            .setup(|app, api| {
                let backend =
                    match api.register_android_plugin(ANDROID_PLUGIN_IDENTIFIER, PLUGIN_CLASS) {
                        Ok(handle) => AndroidBackend::Ready(TauriNativeBackend { handle }),
                        Err(_) => AndroidBackend::Unavailable,
                    };
                let share_root = app
                    .path()
                    .app_cache_dir()
                    .ok()
                    .and_then(|cache_root| prepare_share_root(&cache_root).ok());
                let _ = app.manage(AndroidBridge {
                    backend: Arc::new(backend),
                    share_root,
                });
                Ok(())
            })
            .build()
    }
}

#[cfg(target_os = "android")]
pub(crate) use android::{init, AndroidBridge};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sync::secrets::{
        SecretAccount, SecretBytes, SecretStore, SecretStoreError,
    };
    use std::sync::{Arc, Mutex};

    const OPENROUTER_ACCOUNT: &str = "openrouter_api_key/123e4567-e89b-42d3-a456-426614174000";
    const DEVICE_ACCOUNT: &str = "sync.device.ed25519.v1";
    const VAULT_ACCOUNT: &str = "sync.vault.123e4567-e89b-42d3-a456-426614174000.root.v1";

    #[derive(Default)]
    struct FakeNativeSecrets(Mutex<Option<(String, String)>>);

    struct UnavailableNativeSecrets;

    impl NativeSecretBackend for UnavailableNativeSecrets {
        fn put_secret(
            &self,
            _account: &str,
            _secret_base64: &str,
        ) -> Result<NativePutStatus, NativeBridgeFailure> {
            Err(NativeBridgeFailure::BackendUnavailable)
        }

        fn get_secret(
            &self,
            _account: &str,
        ) -> Result<Option<Zeroizing<String>>, NativeBridgeFailure> {
            Err(NativeBridgeFailure::BackendUnavailable)
        }

        fn delete_secret(&self, _account: &str) -> Result<(), NativeBridgeFailure> {
            Err(NativeBridgeFailure::BackendUnavailable)
        }
    }

    impl NativeSecretBackend for FakeNativeSecrets {
        fn put_secret(
            &self,
            account: &str,
            secret_base64: &str,
        ) -> Result<NativePutStatus, NativeBridgeFailure> {
            let mut value = self.0.lock().unwrap();
            if value.is_some() {
                return Ok(NativePutStatus::AlreadyExists);
            }
            *value = Some((account.to_owned(), secret_base64.to_owned()));
            Ok(NativePutStatus::Stored)
        }

        fn get_secret(
            &self,
            account: &str,
        ) -> Result<Option<Zeroizing<String>>, NativeBridgeFailure> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(stored_account, _)| stored_account == account)
                .map(|(_, secret)| Zeroizing::new(secret.clone())))
        }

        fn delete_secret(&self, account: &str) -> Result<(), NativeBridgeFailure> {
            let mut value = self.0.lock().unwrap();
            if value
                .as_ref()
                .is_some_and(|(stored_account, _)| stored_account == account)
            {
                *value = None;
            }
            Ok(())
        }
    }

    #[test]
    fn android_secret_adapter_round_trips_openrouter_and_sync_accounts_without_overwrite() {
        for account_name in [OPENROUTER_ACCOUNT, DEVICE_ACCOUNT, VAULT_ACCOUNT] {
            let backend = Arc::new(FakeNativeSecrets::default());
            let store = AndroidSecretStore::new(backend.clone());
            let account = SecretAccount::parse(account_name).unwrap();
            let secret = SecretBytes::new(vec![0, 255, 16, 42]).unwrap();

            store.put(&account, &secret).unwrap();
            assert_eq!(
                backend.0.lock().unwrap().as_ref(),
                Some(&(account_name.to_owned(), "AP8QKg==".to_owned()))
            );
            assert_eq!(
                store.get(&account).unwrap().unwrap().expose(),
                &[0, 255, 16, 42]
            );
            assert_eq!(
                store
                    .put(&account, &SecretBytes::new(vec![9]).unwrap())
                    .unwrap_err(),
                SecretStoreError::AlreadyExists
            );
            assert_eq!(
                store.get(&account).unwrap().unwrap().expose(),
                &[0, 255, 16, 42]
            );
            store.delete(&account).unwrap();
            assert!(store.get(&account).unwrap().is_none());
        }
    }

    #[test]
    fn share_descriptor_accepts_only_bounded_grafyn_image_names_with_matching_mime() {
        for (file_name, mime) in [
            (
                "grafyn-123e4567-e89b-42d3-a456-426614174000.png",
                "image/png",
            ),
            (
                "grafyn-123e4567-e89b-42d3-a456-426614174000.jpg",
                "image/jpeg",
            ),
            (
                "grafyn-123e4567-e89b-42d3-a456-426614174000.webp",
                "image/webp",
            ),
        ] {
            let descriptor = ShareDescriptor::parse(file_name, mime).unwrap();
            assert_eq!(descriptor.file_name(), file_name);
            assert_eq!(descriptor.mime(), mime);
        }
    }

    #[test]
    fn share_descriptor_rejects_paths_traversal_wrong_extensions_and_unbounded_names() {
        for (file_name, mime) in [
            ("../grafyn-x.png", "image/png"),
            ("grafyn/secret.png", "image/png"),
            ("grafyn\\secret.png", "image/png"),
            ("C:grafyn.png", "image/png"),
            ("grafyn-..png", "image/png"),
            ("other.png", "image/png"),
            ("grafyn-x.jpg", "image/png"),
            ("grafyn-x.png", "image/jpeg"),
            ("grafyn-x.gif", "image/gif"),
            ("grafyn-é.png", "image/png"),
            ("grafyn-x.png\0tail", "image/png"),
            (
                "grafyn-123e4567-e89b-42d3-a456-426614174000.jpeg",
                "image/jpeg",
            ),
            (
                "grafyn-123E4567-E89B-42D3-A456-426614174000.png",
                "image/png",
            ),
            (
                "grafyn-123e4567-e89b-12d3-a456-426614174000.png",
                "image/png",
            ),
        ] {
            assert!(ShareDescriptor::parse(file_name, mime).is_err());
        }
        let too_long = format!("grafyn-{}.png", "a".repeat(MAX_SHARE_FILENAME_BYTES));
        assert!(ShareDescriptor::parse(&too_long, "image/png").is_err());
    }

    #[test]
    fn staging_creates_a_new_nonempty_file_only_below_the_private_share_root() {
        let temp = tempfile::tempdir().unwrap();
        let share_root = temp
            .path()
            .join(SHARE_DIRECTORY_PARENT)
            .join(SHARE_DIRECTORY_NAME);
        let descriptor = ShareDescriptor::parse(
            "grafyn-123e4567-e89b-42d3-a456-426614174000.png",
            "image/png",
        )
        .unwrap();

        assert_eq!(
            stage_share_file(&share_root, &descriptor, b"").unwrap_err(),
            AndroidBridgeError::InvalidImage
        );
        let staged = stage_share_file(&share_root, &descriptor, b"png-bytes").unwrap();
        assert_eq!(
            staged.path(),
            share_root.join("grafyn-123e4567-e89b-42d3-a456-426614174000.png")
        );
        assert_eq!(staged.file_name(), descriptor.file_name());
        assert_eq!(staged.mime(), descriptor.mime());
        assert_eq!(std::fs::read(staged.path()).unwrap(), b"png-bytes");
        assert_eq!(
            stage_share_file(&share_root, &descriptor, b"replacement").unwrap_err(),
            AndroidBridgeError::FileUnavailable
        );
        assert_eq!(std::fs::read(staged.path()).unwrap(), b"png-bytes");
    }

    #[test]
    fn stale_share_sweep_removes_only_old_valid_regular_generated_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(SHARE_DIRECTORY_NAME);
        std::fs::create_dir_all(&root).unwrap();
        let stale = root.join("grafyn-123e4567-e89b-42d3-a456-426614174000.png");
        let unrelated = root.join("do-not-delete.txt");
        std::fs::write(&stale, b"old").unwrap();
        std::fs::write(&unrelated, b"user").unwrap();

        let stale_modified = std::fs::metadata(&stale).unwrap().modified().unwrap();
        assert_eq!(
            sweep_stale_share_files_at(&root, stale_modified).unwrap(),
            0
        );
        let sweep_time = stale_modified + STALE_SHARE_FILE_AGE + Duration::from_secs(1);
        assert_eq!(sweep_stale_share_files_at(&root, sweep_time).unwrap(), 1);
        assert!(!stale.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn android_secret_adapter_rejects_malformed_or_noncanonical_native_base64() {
        for encoded in ["", "AP8QKg", "AP8QKg===", "AP8QKg==\n", "AB=="] {
            let backend = Arc::new(FakeNativeSecrets(Mutex::new(Some((
                DEVICE_ACCOUNT.to_owned(),
                encoded.to_owned(),
            )))));
            let store = AndroidSecretStore::new(backend);
            assert_eq!(
                store
                    .get(&SecretAccount::sync_device_ed25519())
                    .unwrap_err(),
                SecretStoreError::CorruptSecret,
                "accepted {encoded:?}"
            );
        }
    }

    #[test]
    fn unavailable_native_registration_keeps_secret_operations_fail_closed() {
        let store = AndroidSecretStore::new(Arc::new(UnavailableNativeSecrets));
        let account = SecretAccount::sync_device_ed25519();
        let secret = SecretBytes::new(vec![1]).unwrap();

        assert_eq!(
            store.put(&account, &secret).unwrap_err(),
            SecretStoreError::BackendUnavailable
        );
        assert_eq!(
            store.get(&account).unwrap_err(),
            SecretStoreError::BackendUnavailable
        );
        assert_eq!(
            store.delete(&account).unwrap_err(),
            SecretStoreError::BackendUnavailable
        );
    }

    #[test]
    fn fixed_native_failure_codes_map_without_reflecting_backend_text() {
        assert_eq!(
            NativeBridgeFailure::from_code("invalid_account").secret_error(),
            SecretStoreError::InvalidAccount
        );
        assert_eq!(
            NativeBridgeFailure::from_code("invalid_secret").secret_error(),
            SecretStoreError::InvalidSecret
        );
        assert_eq!(
            NativeBridgeFailure::from_code("secret_too_large").secret_error(),
            SecretStoreError::SecretTooLarge
        );
        assert_eq!(
            NativeBridgeFailure::from_code("already_exists").secret_error(),
            SecretStoreError::AlreadyExists
        );
        assert_eq!(
            NativeBridgeFailure::from_code("corrupt_secret").secret_error(),
            SecretStoreError::CorruptSecret
        );
        assert_eq!(
            NativeBridgeFailure::from_code("do-not-reflect-native-details").secret_error(),
            SecretStoreError::BackendUnavailable
        );
        assert_eq!(
            share_error_from_status("invalid_filename"),
            AndroidBridgeError::InvalidFilename
        );
        assert_eq!(
            share_error_from_status("invalid_mime"),
            AndroidBridgeError::InvalidMime
        );
        assert_eq!(
            share_error_from_status("file_unavailable"),
            AndroidBridgeError::FileUnavailable
        );
        assert_eq!(
            share_error_from_status("do-not-reflect-native-details"),
            AndroidBridgeError::BackendUnavailable
        );
        assert_eq!(parse_share_launch_status("share_sheet_opened"), Ok(()));
        for status in ["shared", "canceled", "unknown"] {
            assert_eq!(
                parse_share_launch_status(status),
                Err(AndroidBridgeError::BackendUnavailable)
            );
        }
    }

    #[test]
    fn android_release_source_gate_forbids_plaintext_or_fallback_secret_persistence() {
        let plugin = include_str!(
            "../../gen/android/app/src/main/java/com/grafyn/app/GrafynAndroidPlugin.kt"
        );
        let contracts = include_str!(
            "../../gen/android/app/src/main/java/com/grafyn/app/GrafynAndroidContracts.kt"
        );
        let secret_store = plugin
            .split("private class AndroidKeystoreSecretStore")
            .nth(1)
            .and_then(|source| source.split("@TauriPlugin").next())
            .expect("Android Keystore secret store source");
        let write_record = secret_store
            .split("private fun writeRecord")
            .nth(1)
            .and_then(|source| source.split("private fun readRecord").next())
            .expect("encrypted-record persistence source");
        let health = secret_store
            .split("fun health")
            .nth(1)
            .and_then(|source| source.split("fun put").next())
            .expect("health source");
        let read_record = secret_store
            .split("private fun readRecordByPrefix")
            .nth(1)
            .and_then(|source| source.split("private fun validateStoredRecords").next())
            .expect("encrypted-record read source");
        let validate_records = secret_store
            .split("private fun validateStoredRecords")
            .nth(1)
            .and_then(|source| source.split("private fun recordParts").next())
            .expect("encrypted-record authentication source");
        assert!(contracts.contains("const val KEY_ALIAS = \"com.grafyn.app.secrets.aes-gcm.v1\""));
        assert!(contracts.contains("const val GCM_NONCE_BYTES = 12"));
        assert!(contracts.contains("const val GCM_TAG_BYTES = 16"));
        assert!(contracts.contains("private const val AAD_SCHEMA = \"grafyn-secret-aad-v1\""));
        assert!(contracts.contains("const val SECRET_SERVICE = \"com.grafyn.app\""));
        assert!(contracts.contains(
            "\"$AAD_SCHEMA\\u0000$SECRET_SERVICE\\u0000$account\".toByteArray(StandardCharsets.UTF_8)"
        ));
        for required in [
            "Cipher.DECRYPT_MODE",
            "cipher.updateAAD(aadFor(account))",
            "cipher.doFinal(sealed)",
        ] {
            assert!(
                contracts.contains(required),
                "missing authentication {required}"
            );
        }
        for required in [
            "KeyStore.getInstance(\"AndroidKeyStore\")",
            "Cipher.getInstance(\"AES/GCM/NoPadding\")",
            ".setKeySize(256)",
            ".setRandomizedEncryptionRequired(true)",
            "if (key.encoded != null)",
            "val nonce = cipher.iv",
            "nonce.size != GrafynAndroidContracts.GCM_NONCE_BYTES",
            "cipher.updateAAD(GrafynAndroidContracts.aadFor(account))",
            "if (readRecord(account) != null) return@synchronized \"already_exists\"",
            "MessageDigest.isEqual(secret, readback)",
        ] {
            assert!(secret_store.contains(required), "missing {required}");
        }
        for forbidden in [
            "FileOutputStream",
            "openFileOutput",
            "getExternalFilesDir",
            "Environment.",
            "System.getenv",
            "System.getProperty",
            "SharedPreferences.getDefaultSharedPreferences",
            "printStackTrace",
            "error.message",
            "Log.",
        ] {
            assert!(
                !secret_store.contains(forbidden),
                "found fallback {forbidden}"
            );
        }
        assert_eq!(secret_store.matches(".putString(").count(), 4);
        for persisted_suffix in [
            "$prefix.account",
            "$prefix.nonce",
            "$prefix.ciphertext",
            "$prefix.tag",
        ] {
            assert!(write_record.contains(persisted_suffix));
        }
        for forbidden in ["encodedSecret", "secretBase64", "plaintext"] {
            assert!(
                !write_record.contains(forbidden),
                "persisted submitted or plaintext secret via {forbidden}"
            );
        }
        assert!(health.contains("preferences.all.keys.toSet()"));
        assert!(health.contains("val hasDurableRecords = recordKeys.isNotEmpty()"));
        assert!(health.contains("validateStoredRecords(recordKeys, key)"));
        assert!(health
            .contains("val key = if (hasDurableRecords) loadExistingKey() else loadOrCreateKey()"));
        assert!(health
            .contains("if (key.encoded != null) throw SecretFailure(\"backend_unavailable\")"));
        assert!(health.contains(
            "GrafynAndroidContracts.secureSecretHealth(hasDurableRecords, keystoreReady = false)"
        ));
        assert!(!health.contains(".edit()"));
        assert!(!health.contains(".putString("));
        assert!(!health.contains("File("));
        assert!(read_record.contains("preferences.getString(\"$prefix.account\", null)"));
        assert!(read_record.contains("recordPrefix(account) != prefix"));
        assert!(validate_records
            .contains("for (prefix in recordKeys.map { it.substringBeforeLast('.') }.toSet())"));
        assert!(validate_records.contains("readRecordByPrefix(prefix)"));
        assert!(validate_records.contains("GrafynAndroidContracts.authenticatesSecret("));
        assert!(validate_records.contains("record.account"));
    }

    #[test]
    fn android_release_source_gate_keeps_native_share_private_and_read_only() {
        let plugin = include_str!(
            "../../gen/android/app/src/main/java/com/grafyn/app/GrafynAndroidPlugin.kt"
        );
        let contracts = include_str!(
            "../../gen/android/app/src/main/java/com/grafyn/app/GrafynAndroidContracts.kt"
        );
        let manifest = include_str!("../../gen/android/app/src/main/AndroidManifest.xml");
        let paths = include_str!("../../gen/android/app/src/main/res/xml/file_paths.xml");

        assert!(contracts.contains("const val SHARE_DIRECTORY = \"Grafyn/grafyn-share-v1\""));
        assert!(contracts.contains("const val SHARE_AUTHORITY_SUFFIX = \".grafyn.share\""));
        assert!(contracts.contains("const val SHARE_SHEET_OPENED_STATUS = \"share_sheet_opened\""));
        assert!(!plugin.contains("\"shared\""));
        assert!(plugin.contains("Intent.ACTION_SEND"));
        assert!(plugin.contains("Intent.FLAG_GRANT_READ_URI_PERMISSION"));
        assert!(!plugin.contains("Intent.FLAG_GRANT_WRITE_URI_PERMISSION"));
        assert!(plugin.contains("file != requested || file.parentFile != root || !file.isFile"));
        assert!(manifest.contains("android:authorities=\"${applicationId}.grafyn.share\""));
        assert!(manifest.contains("android:exported=\"false\""));
        assert!(manifest.contains("android:grantUriPermissions=\"true\""));
        assert_eq!(paths.matches("<cache-path").count(), 1);
        assert!(!paths.contains("external"));
        assert!(paths.contains("path=\"Grafyn/grafyn-share-v1/\""));
    }

    #[test]
    fn native_health_accepts_only_consistent_fixed_bounded_statuses() {
        assert_eq!(
            parse_native_health("ready", "ready", "ready").unwrap(),
            AndroidBridgeHealth {
                secure_secrets: NativeHealth::Ready,
                native_image_share: NativeHealth::Ready,
            }
        );
        assert_eq!(
            parse_native_health("degraded", "unavailable", "ready").unwrap(),
            AndroidBridgeHealth {
                secure_secrets: NativeHealth::Unavailable,
                native_image_share: NativeHealth::Ready,
            }
        );
        for (overall, secrets, share) in [
            ("ready", "unavailable", "ready"),
            ("degraded", "ready", "ready"),
            ("unknown", "ready", "ready"),
            ("ready", "do-not-reflect", "ready"),
            ("ready", "ready", "do-not-reflect"),
        ] {
            assert!(parse_native_health(overall, secrets, share).is_err());
        }

        assert_eq!(
            parse_native_health("fatal", "corrupt", "ready"),
            Err(AndroidBridgeError::InvalidHealth)
        );
        assert_eq!(
            fail_closed_bridge_health(Err(AndroidBridgeError::InvalidHealth), true),
            Err(AndroidBridgeError::InvalidHealth)
        );
        assert_eq!(
            unavailable_native_bridge_health(),
            AndroidBridgeHealth {
                secure_secrets: NativeHealth::Unavailable,
                native_image_share: NativeHealth::Unavailable,
            }
        );
        assert_eq!(
            fail_closed_bridge_health(
                Ok(AndroidBridgeHealth {
                    secure_secrets: NativeHealth::Ready,
                    native_image_share: NativeHealth::Ready,
                }),
                false,
            ),
            Ok(AndroidBridgeHealth {
                secure_secrets: NativeHealth::Ready,
                native_image_share: NativeHealth::Unavailable,
            })
        );
    }

    #[test]
    fn android_plugin_setup_manages_fail_closed_state_instead_of_aborting_boot() {
        let source = include_str!("android_bridge.rs");
        let setup = source
            .split("pub(crate) fn init")
            .nth(1)
            .and_then(|source| {
                source
                    .split("pub(crate) use android::{init, AndroidBridge};")
                    .next()
            })
            .expect("Android bridge plugin setup source");

        assert!(setup.contains("Err(_) => AndroidBackend::Unavailable"));
        assert!(setup.contains("prepare_share_root(&cache_root).ok()"));
        assert!(setup.contains("let _ = app.manage(AndroidBridge"));
        assert!(setup.contains("Ok(())"));
        assert!(
            !setup.contains("register_android_plugin(ANDROID_PLUGIN_IDENTIFIER, PLUGIN_CLASS)?")
        );
        assert!(!setup.contains("app_cache_dir()?"));
    }
}
