use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::{collections::BTreeMap, sync::Mutex};
use zeroize::Zeroizing;

pub(crate) const SECRET_STORE_SERVICE: &str = "com.grafyn.app";
pub(crate) const MAX_SECRET_BYTES: usize = 1024;

const OPENROUTER_KEY_PREFIX: &str = "openrouter_api_key/";
const SYNC_DEVICE_ED25519_ACCOUNT: &str = "sync.device.ed25519.v1";
const SYNC_VAULT_PREFIX: &str = "sync.vault.";
const SYNC_VAULT_SUFFIX: &str = ".root.v1";
const KEYRING_VALUE_PREFIX: &str = "grafyn-secret-v1:";
const SECRET_CLAIM_DIRECTORY: &str = "secret-account-claims-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecretStoreError {
    InvalidAccount,
    InvalidSecret,
    SecretTooLarge,
    AlreadyExists,
    CorruptSecret,
    BackendUnavailable,
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAccount => "invalid secret account",
            Self::InvalidSecret => "invalid secret value",
            Self::SecretTooLarge => "secret value exceeds the supported limit",
            Self::AlreadyExists => "secret account already exists",
            Self::CorruptSecret => "stored secret is invalid",
            Self::BackendUnavailable => "secret store is unavailable",
        })
    }
}

impl std::error::Error for SecretStoreError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecretAccountKind {
    OpenRouter,
    SyncDeviceEd25519,
    SyncVaultRoot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SecretAccount {
    value: String,
    kind: SecretAccountKind,
}

impl SecretAccount {
    pub(crate) fn parse(value: &str) -> Result<Self, SecretStoreError> {
        if value == SYNC_DEVICE_ED25519_ACCOUNT {
            return Ok(Self {
                value: value.to_owned(),
                kind: SecretAccountKind::SyncDeviceEd25519,
            });
        }

        if let Some(version) = value.strip_prefix(OPENROUTER_KEY_PREFIX) {
            validate_canonical_uuid(version)?;
            return Ok(Self {
                value: value.to_owned(),
                kind: SecretAccountKind::OpenRouter,
            });
        }

        if let Some(vault_id) = value
            .strip_prefix(SYNC_VAULT_PREFIX)
            .and_then(|rest| rest.strip_suffix(SYNC_VAULT_SUFFIX))
        {
            validate_canonical_uuid(vault_id)?;
            return Ok(Self {
                value: value.to_owned(),
                kind: SecretAccountKind::SyncVaultRoot,
            });
        }

        Err(SecretStoreError::InvalidAccount)
    }

    pub(crate) fn openrouter_key(version: &str) -> Result<Self, SecretStoreError> {
        validate_canonical_uuid(version)?;
        Self::parse(&format!("{OPENROUTER_KEY_PREFIX}{version}"))
    }

    pub(crate) fn sync_device_ed25519() -> Self {
        Self {
            value: SYNC_DEVICE_ED25519_ACCOUNT.to_owned(),
            kind: SecretAccountKind::SyncDeviceEd25519,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sync_vault_root(vault_id: &str) -> Result<Self, SecretStoreError> {
        validate_canonical_uuid(vault_id)?;
        Self::parse(&format!("{SYNC_VAULT_PREFIX}{vault_id}{SYNC_VAULT_SUFFIX}"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.value
    }

    fn allows_legacy_utf8(&self) -> bool {
        self.kind == SecretAccountKind::OpenRouter
    }
}

fn validate_canonical_uuid(value: &str) -> Result<(), SecretStoreError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return Err(SecretStoreError::InvalidAccount);
    }

    let mut non_nil = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return Err(SecretStoreError::InvalidAccount);
            }
            continue;
        }
        if !matches!(byte, b'0'..=b'9' | b'a'..=b'f') {
            return Err(SecretStoreError::InvalidAccount);
        }
        non_nil |= byte != b'0';
    }

    if non_nil {
        Ok(())
    } else {
        Err(SecretStoreError::InvalidAccount)
    }
}

pub(crate) struct SecretBytes {
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> Result<Self, SecretStoreError> {
        let bytes = Zeroizing::new(bytes);
        validate_secret_len(bytes.len())?;
        Ok(Self { bytes })
    }

    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, SecretStoreError> {
        Self::new(bytes.to_vec())
    }

    pub(crate) fn expose(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes([REDACTED])")
    }
}

fn validate_secret_len(len: usize) -> Result<(), SecretStoreError> {
    match len {
        0 => Err(SecretStoreError::InvalidSecret),
        1..=MAX_SECRET_BYTES => Ok(()),
        _ => Err(SecretStoreError::SecretTooLarge),
    }
}

fn encode_keyring_value(secret: &SecretBytes) -> Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let bytes = secret.expose();
    let mut encoded = Zeroizing::new(String::with_capacity(
        KEYRING_VALUE_PREFIX.len() + bytes.len() * 2,
    ));
    encoded.push_str(KEYRING_VALUE_PREFIX);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_keyring_value(
    account: &SecretAccount,
    value: &str,
) -> Result<SecretBytes, SecretStoreError> {
    let Some(encoded) = value.strip_prefix(KEYRING_VALUE_PREFIX) else {
        return if account.allows_legacy_utf8() {
            SecretBytes::from_slice(value.as_bytes()).map_err(|_| SecretStoreError::CorruptSecret)
        } else {
            Err(SecretStoreError::CorruptSecret)
        };
    };

    if encoded.is_empty()
        || encoded.len() % 2 != 0
        || encoded.len() > MAX_SECRET_BYTES.saturating_mul(2)
    {
        return Err(SecretStoreError::CorruptSecret);
    }

    let mut bytes = Zeroizing::new(Vec::with_capacity(encoded.len() / 2));
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = decode_lower_hex(pair[0]).ok_or(SecretStoreError::CorruptSecret)?;
        let low = decode_lower_hex(pair[1]).ok_or(SecretStoreError::CorruptSecret)?;
        bytes.push((high << 4) | low);
    }
    validate_secret_len(bytes.len()).map_err(|_| SecretStoreError::CorruptSecret)?;
    Ok(SecretBytes { bytes })
}

fn decode_lower_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

pub(crate) trait SecretStore: Send + Sync {
    fn put(&self, account: &SecretAccount, secret: &SecretBytes) -> Result<(), SecretStoreError>;
    fn get(&self, account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError>;
    fn delete(&self, account: &SecretAccount) -> Result<(), SecretStoreError>;
}

#[derive(Debug, Default)]
pub(crate) struct KeyringSecretStore;

impl KeyringSecretStore {
    fn entry(account: &SecretAccount) -> Result<keyring::Entry, SecretStoreError> {
        keyring::Entry::new(SECRET_STORE_SERVICE, account.as_str())
            .map_err(|_| SecretStoreError::BackendUnavailable)
    }

    fn claim_root() -> Result<PathBuf, SecretStoreError> {
        dirs::data_local_dir()
            .or_else(dirs::config_dir)
            .map(|root| root.join("Grafyn").join(SECRET_CLAIM_DIRECTORY))
            .ok_or(SecretStoreError::BackendUnavailable)
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, account: &SecretAccount, secret: &SecretBytes) -> Result<(), SecretStoreError> {
        let entry = Self::entry(account)?;
        put_keyring_with_claim(
            &Self::claim_root()?,
            account,
            secret,
            || load_raw_keyring_value(&entry),
            |value| {
                entry
                    .set_password(value)
                    .map_err(|_| SecretStoreError::BackendUnavailable)
            },
        )
    }

    fn get(&self, account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        let entry = Self::entry(account)?;
        with_account_claim(&Self::claim_root()?, account, || {
            load_raw_keyring_value(&entry)?
                .map(|value| decode_keyring_value(account, value.as_str()))
                .transpose()
        })
    }

    fn delete(&self, account: &SecretAccount) -> Result<(), SecretStoreError> {
        let entry = Self::entry(account)?;
        with_account_claim(&Self::claim_root()?, account, || {
            match entry.delete_password() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(SecretStoreError::BackendUnavailable),
            }
        })
    }
}

fn load_raw_keyring_value(
    entry: &keyring::Entry,
) -> Result<Option<Zeroizing<String>>, SecretStoreError> {
    match entry.get_password() {
        Ok(value) => Ok(Some(Zeroizing::new(value))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(SecretStoreError::BackendUnavailable),
    }
}

fn put_keyring_with_claim<Load, Store>(
    claim_root: &Path,
    account: &SecretAccount,
    secret: &SecretBytes,
    mut load: Load,
    mut store: Store,
) -> Result<(), SecretStoreError>
where
    Load: FnMut() -> Result<Option<Zeroizing<String>>, SecretStoreError>,
    Store: FnMut(&str) -> Result<(), SecretStoreError>,
{
    with_account_claim(claim_root, account, || {
        if load()?.is_some() {
            return Err(SecretStoreError::AlreadyExists);
        }

        let encoded = encode_keyring_value(secret);
        store(encoded.as_str())?;
        match load()? {
            Some(durable) if durable.as_str() == encoded.as_str() => Ok(()),
            _ => Err(SecretStoreError::BackendUnavailable),
        }
    })
}

fn with_account_claim<T>(
    claim_root: &Path,
    account: &SecretAccount,
    action: impl FnOnce() -> Result<T, SecretStoreError>,
) -> Result<T, SecretStoreError> {
    std::fs::create_dir_all(claim_root).map_err(|_| SecretStoreError::BackendUnavailable)?;
    let root = crate::services::twin_events::AnchoredRoot::open(claim_root)
        .map_err(|_| SecretStoreError::BackendUnavailable)?;
    let claim = root
        .lock_exclusive(&account_claim_filename(account))
        .map_err(|_| SecretStoreError::BackendUnavailable)?;
    let result = action();
    let unlock = claim
        .unlock()
        .map_err(|_| SecretStoreError::BackendUnavailable);
    match (result, unlock) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn account_claim_filename(account: &SecretAccount) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(account.as_str().len() * 2 + ".lock".len());
    for byte in account.as_str().bytes() {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 0x0f) as usize] as char);
    }
    name.push_str(".lock");
    name
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemorySecretStore {
    values: Mutex<BTreeMap<String, SecretBytes>>,
}

#[cfg(test)]
impl fmt::Debug for MemorySecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MemorySecretStore([REDACTED])")
    }
}

#[cfg(test)]
impl SecretStore for MemorySecretStore {
    fn put(&self, account: &SecretAccount, secret: &SecretBytes) -> Result<(), SecretStoreError> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| SecretStoreError::BackendUnavailable)?;
        if values.contains_key(account.as_str()) {
            return Err(SecretStoreError::AlreadyExists);
        }
        values.insert(
            account.as_str().to_owned(),
            SecretBytes::from_slice(secret.expose())?,
        );
        Ok(())
    }

    fn get(&self, account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        self.values
            .lock()
            .map_err(|_| SecretStoreError::BackendUnavailable)?
            .get(account.as_str())
            .map(|secret| SecretBytes::from_slice(secret.expose()))
            .transpose()
    }

    fn delete(&self, account: &SecretAccount) -> Result<(), SecretStoreError> {
        self.values
            .lock()
            .map_err(|_| SecretStoreError::BackendUnavailable)?
            .remove(account.as_str());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    const CANONICAL_UUID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const CLAIM_WORKER_ROLE: &str = "GRAFYN_SECRET_CLAIM_WORKER_ROLE";
    const CLAIM_WORKER_ROOT: &str = "GRAFYN_SECRET_CLAIM_WORKER_ROOT";
    const CRASH_WORKER_ROOT: &str = "GRAFYN_SECRET_CLAIM_CRASH_WORKER_ROOT";

    fn wait_for_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn spawn_claim_worker(role: &str, root: &Path) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("services::sync::secrets::tests::cross_process_keyring_claim_is_first_writer_wins")
            .arg("--nocapture")
            .env(CLAIM_WORKER_ROLE, role)
            .env(CLAIM_WORKER_ROOT, root)
            .spawn()
            .unwrap()
    }

    fn run_claim_worker(role: &str, root: &Path) {
        let claim_root = root.join("claims");
        let durable_path = root.join("durable-value");
        let before_path = root.join(format!("{role}-before-put"));
        let load_path = root.join(format!("{role}-load-entered"));
        let result_path = root.join(format!("{role}-result"));
        std::fs::write(&before_path, b"ready").unwrap();

        let account = SecretAccount::sync_device_ed25519();
        let secret_byte = if role == "a" { 0x11 } else { 0x22 };
        let secret = SecretBytes::new(vec![secret_byte; 32]).unwrap();
        let result = put_keyring_with_claim(
            &claim_root,
            &account,
            &secret,
            || {
                std::fs::write(&load_path, b"entered")
                    .map_err(|_| SecretStoreError::BackendUnavailable)?;
                match std::fs::read_to_string(&durable_path) {
                    Ok(value) => Ok(Some(Zeroizing::new(value))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(_) => Err(SecretStoreError::BackendUnavailable),
                }
            },
            |value| {
                if role == "a" {
                    let entered = root.join("a-store-entered");
                    std::fs::write(&entered, b"entered")
                        .map_err(|_| SecretStoreError::BackendUnavailable)?;
                    let release = root.join("release-a");
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !release.exists() {
                        if Instant::now() >= deadline {
                            return Err(SecretStoreError::BackendUnavailable);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                std::fs::write(&durable_path, value)
                    .map_err(|_| SecretStoreError::BackendUnavailable)
            },
        );
        let status = match result {
            Ok(()) => "ok".to_owned(),
            Err(SecretStoreError::AlreadyExists) => "already_exists".to_owned(),
            Err(error) => format!("error:{error}"),
        };
        std::fs::write(result_path, status).unwrap();
    }

    #[test]
    fn typed_accounts_keep_the_existing_and_sync_names_exact() {
        assert_eq!(
            SecretAccount::openrouter_key(CANONICAL_UUID)
                .unwrap()
                .as_str(),
            "openrouter_api_key/123e4567-e89b-42d3-a456-426614174000"
        );
        assert_eq!(
            SecretAccount::sync_device_ed25519().as_str(),
            "sync.device.ed25519.v1"
        );
        assert_eq!(
            SecretAccount::sync_vault_root(CANONICAL_UUID)
                .unwrap()
                .as_str(),
            "sync.vault.123e4567-e89b-42d3-a456-426614174000.root.v1"
        );
    }

    #[test]
    fn account_parser_rejects_unknown_or_noncanonical_names() {
        for value in [
            "",
            "openrouter_api_key/not-a-uuid",
            "openrouter_api_key/123E4567-E89B-42D3-A456-426614174000",
            "openrouter_api_key/00000000-0000-0000-0000-000000000000",
            "sync.device.ed25519.v2",
            "sync.vault.not-a-uuid.root.v1",
            "sync.vault.123E4567-E89B-42D3-A456-426614174000.root.v1",
            "sync.vault.00000000-0000-0000-0000-000000000000.root.v1",
            "sync.vault.123e4567-e89b-42d3-a456-426614174000.root.v2",
            "unrelated.account",
        ] {
            assert!(SecretAccount::parse(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn account_parser_accepts_only_the_three_typed_shapes() {
        for expected in [
            "openrouter_api_key/123e4567-e89b-42d3-a456-426614174000",
            "sync.device.ed25519.v1",
            "sync.vault.123e4567-e89b-42d3-a456-426614174000.root.v1",
        ] {
            assert_eq!(SecretAccount::parse(expected).unwrap().as_str(), expected);
        }
    }

    #[test]
    fn secret_values_are_nonempty_bounded_and_debug_redacted() {
        assert_eq!(
            SecretBytes::new(Vec::new()).unwrap_err(),
            SecretStoreError::InvalidSecret
        );
        assert_eq!(
            SecretBytes::new(vec![7; MAX_SECRET_BYTES + 1]).unwrap_err(),
            SecretStoreError::SecretTooLarge
        );

        let secret = SecretBytes::new(vec![0, 255, 16]).unwrap();
        assert_eq!(secret.expose(), &[0, 255, 16]);
        assert_eq!(format!("{secret:?}"), "SecretBytes([REDACTED])");
        assert!(!format!("{secret:?}").contains("255"));

        let largest = SecretBytes::new(vec![42; MAX_SECRET_BYTES]).unwrap();
        assert_eq!(largest.expose().len(), MAX_SECRET_BYTES);
    }

    #[test]
    fn keyring_representation_round_trips_arbitrary_bytes_canonically() {
        let secret = SecretBytes::new(vec![0, 255, 16]).unwrap();
        let encoded = encode_keyring_value(&secret);

        assert_eq!(encoded.as_str(), "grafyn-secret-v1:00ff10");
        assert_eq!(
            decode_keyring_value(&SecretAccount::sync_device_ed25519(), &encoded)
                .unwrap()
                .expose(),
            &[0, 255, 16]
        );
    }

    #[test]
    fn keyring_representation_rejects_noncanonical_or_malformed_values() {
        let account = SecretAccount::sync_device_ed25519();
        for value in [
            "",
            "00ff10",
            "grafyn-secret-v1:",
            "grafyn-secret-v1:0",
            "grafyn-secret-v1:00FF10",
            "grafyn-secret-v1:00fg",
        ] {
            assert_eq!(
                decode_keyring_value(&account, value).unwrap_err(),
                SecretStoreError::CorruptSecret,
                "accepted {value}"
            );
        }

        let oversized = format!("grafyn-secret-v1:{}", "00".repeat(MAX_SECRET_BYTES + 1));
        assert_eq!(
            decode_keyring_value(&account, &oversized).unwrap_err(),
            SecretStoreError::CorruptSecret
        );
    }

    #[test]
    fn only_openrouter_accounts_accept_legacy_utf8_keyring_values() {
        let openrouter = SecretAccount::openrouter_key(CANONICAL_UUID).unwrap();
        let legacy = decode_keyring_value(&openrouter, "sk-or-v1-legacy").unwrap();
        assert_eq!(legacy.expose(), b"sk-or-v1-legacy");

        assert_eq!(
            decode_keyring_value(
                &SecretAccount::sync_vault_root(CANONICAL_UUID).unwrap(),
                "legacy-value"
            )
            .unwrap_err(),
            SecretStoreError::CorruptSecret
        );
    }

    #[test]
    fn memory_store_round_trips_bytes_without_overwriting_existing_accounts() {
        let store = MemorySecretStore::default();
        let account = SecretAccount::sync_device_ed25519();
        let original = SecretBytes::new(vec![0, 255, 16]).unwrap();
        store.put(&account, &original).unwrap();

        let loaded = store.get(&account).unwrap().unwrap();
        assert_eq!(loaded.expose(), &[0, 255, 16]);
        assert_eq!(
            store
                .put(&account, &SecretBytes::new(vec![9]).unwrap())
                .unwrap_err(),
            SecretStoreError::AlreadyExists
        );
        assert_eq!(
            store.get(&account).unwrap().unwrap().expose(),
            &[0, 255, 16]
        );
    }

    #[test]
    fn memory_store_delete_is_idempotent_and_accounts_are_independent() {
        let store = MemorySecretStore::default();
        let device = SecretAccount::sync_device_ed25519();
        let vault = SecretAccount::sync_vault_root(CANONICAL_UUID).unwrap();
        store
            .put(&device, &SecretBytes::new(vec![1]).unwrap())
            .unwrap();
        store
            .put(&vault, &SecretBytes::new(vec![2]).unwrap())
            .unwrap();

        store.delete(&device).unwrap();
        store.delete(&device).unwrap();

        assert!(store.get(&device).unwrap().is_none());
        assert_eq!(store.get(&vault).unwrap().unwrap().expose(), &[2]);
    }

    #[test]
    fn store_debug_and_errors_never_include_secret_material() {
        let store = MemorySecretStore::default();
        assert_eq!(format!("{store:?}"), "MemorySecretStore([REDACTED])");

        let marker = "do-not-leak-this-secret";
        for error in [
            SecretStoreError::InvalidAccount,
            SecretStoreError::InvalidSecret,
            SecretStoreError::SecretTooLarge,
            SecretStoreError::AlreadyExists,
            SecretStoreError::CorruptSecret,
            SecretStoreError::BackendUnavailable,
        ] {
            let rendered = format!("{error:?}: {error}");
            assert!(!rendered.contains(marker));
        }
    }

    #[test]
    fn secret_store_trait_is_send_sync_and_uses_the_fixed_service() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MemorySecretStore>();
        assert_eq!(SECRET_STORE_SERVICE, "com.grafyn.app");
    }

    #[test]
    fn cross_process_keyring_claim_is_first_writer_wins() {
        if let Ok(role) = std::env::var(CLAIM_WORKER_ROLE) {
            let root = PathBuf::from(std::env::var_os(CLAIM_WORKER_ROOT).unwrap());
            run_claim_worker(&role, &root);
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let mut first = spawn_claim_worker("a", root.path());
        wait_for_path(&root.path().join("a-store-entered"));

        let mut second = spawn_claim_worker("b", root.path());
        wait_for_path(&root.path().join("b-before-put"));
        std::thread::sleep(Duration::from_millis(250));
        let second_crossed_claim = root.path().join("b-load-entered").exists();

        std::fs::write(root.path().join("release-a"), b"release").unwrap();
        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());

        assert!(
            !second_crossed_claim,
            "the competing process reached the backend before the first writer released its claim"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("a-result")).unwrap(),
            "ok"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("b-result")).unwrap(),
            "already_exists"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("durable-value")).unwrap(),
            "grafyn-secret-v1:1111111111111111111111111111111111111111111111111111111111111111"
        );
    }

    #[test]
    fn crashed_claim_holder_does_not_strand_the_account() {
        if let Some(root) = std::env::var_os(CRASH_WORKER_ROOT) {
            let root = PathBuf::from(root);
            let account = SecretAccount::sync_device_ed25519();
            let _: Result<(), SecretStoreError> =
                with_account_claim(&root.join("claims"), &account, || {
                    std::fs::write(root.join("claim-entered"), b"entered")
                        .map_err(|_| SecretStoreError::BackendUnavailable)?;
                    std::process::exit(0);
                });
            unreachable!("the crash worker exits while holding the claim");
        }

        let root = tempfile::tempdir().unwrap();
        let mut crashed = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("services::sync::secrets::tests::crashed_claim_holder_does_not_strand_the_account")
            .arg("--nocapture")
            .env(CRASH_WORKER_ROOT, root.path())
            .spawn()
            .unwrap();
        wait_for_path(&root.path().join("claim-entered"));
        assert!(crashed.wait().unwrap().success());

        let account = SecretAccount::sync_device_ed25519();
        let secret = SecretBytes::new(vec![0x5a; 32]).unwrap();
        let durable = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let read = durable.clone();
        let write = durable.clone();
        put_keyring_with_claim(
            &root.path().join("claims"),
            &account,
            &secret,
            move || {
                Ok(read
                    .lock()
                    .map_err(|_| SecretStoreError::BackendUnavailable)?
                    .clone()
                    .map(Zeroizing::new))
            },
            move |value| {
                *write
                    .lock()
                    .map_err(|_| SecretStoreError::BackendUnavailable)? = Some(value.to_owned());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            durable.lock().unwrap().as_deref(),
            Some(
                "grafyn-secret-v1:5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a"
            )
        );
    }
}
