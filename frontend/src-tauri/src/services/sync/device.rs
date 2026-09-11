use crate::models::twin_event::DeviceId as TwinDeviceId;
use crate::services::sync::secrets::{SecretAccount, SecretBytes, SecretStore, SecretStoreError};
use crate::services::twin_events::{AnchoredRoot, CoordinatorProcessLock, MutationError};
use grafyn_sync_protocol::{DeviceId as ProtocolDeviceId, DevicePublicKey, DeviceSigningKey};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) const DEVICE_SIGNING_BINDING_KEY: &str = "twin/events/device-signing-v1.json";
pub(crate) const DEVICE_SIGNING_BINDING_LIMIT: usize = 4096;
const DEVICE_SIGNING_BINDING_SCHEMA_VERSION: u16 = 1;
const DEVICE_SIGNING_BINDING_STAGING_DIRECTORY: &str = "twin/events";

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DeviceSigningIdentity {
    device_id: ProtocolDeviceId,
    signing_key: DeviceSigningKey,
    public_key: DevicePublicKey,
}

#[cfg_attr(not(test), allow(dead_code))]
impl DeviceSigningIdentity {
    pub(crate) const fn device_id(&self) -> &ProtocolDeviceId {
        &self.device_id
    }

    pub(crate) const fn signing_key(&self) -> &DeviceSigningKey {
        &self.signing_key
    }

    pub(crate) const fn public_key(&self) -> &DevicePublicKey {
        &self.public_key
    }
}

impl fmt::Debug for DeviceSigningIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceSigningIdentity")
            .field("device_id", &self.device_id.to_string())
            .field("public_key_hex", &lower_hex(self.public_key.as_bytes()))
            .field("signing_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceSigningBindingWire {
    schema_version: u16,
    device_id: String,
    public_key_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceSigningBinding {
    device_id: ProtocolDeviceId,
    public_key: DevicePublicKey,
}

pub(crate) fn load_or_create_device_signing_identity(
    data_path: &Path,
    process_lock: &CoordinatorProcessLock,
    secret_store: Arc<dyn SecretStore>,
    writer_device_id: &TwinDeviceId,
) -> Result<DeviceSigningIdentity, MutationError> {
    if !process_lock.covers_data_path(data_path)? {
        return Err(MutationError::Invalid(
            "coordinator process lock does not cover the device identity data root".into(),
        ));
    }
    let writer_device_id = ProtocolDeviceId::parse_str(writer_device_id.as_str())
        .map_err(|_| MutationError::Invalid("Twin writer device ID is not canonical".into()))?;
    let root = AnchoredRoot::open(data_path)?;
    let account = SecretAccount::sync_device_ed25519();

    if let Some(bytes) =
        root.read_bounded(DEVICE_SIGNING_BINDING_KEY, DEVICE_SIGNING_BINDING_LIMIT)?
    {
        return validate_bound_identity(
            parse_binding(&bytes)?,
            writer_device_id,
            secret_store.as_ref(),
            &account,
        );
    }

    let signing_key = load_or_create_unbound_secret(secret_store.as_ref(), &account)?;
    let binding = DeviceSigningBinding {
        device_id: writer_device_id,
        public_key: signing_key.public_key(),
    };
    let bytes = serialize_binding(&binding)?;
    root.install_no_clobber(
        DEVICE_SIGNING_BINDING_KEY,
        DEVICE_SIGNING_BINDING_STAGING_DIRECTORY,
        &bytes,
    )?;

    let durable = root
        .read_bounded(DEVICE_SIGNING_BINDING_KEY, DEVICE_SIGNING_BINDING_LIMIT)?
        .ok_or_else(|| {
            MutationError::RecoveryConflict("device-signing-binding-missing-after-install".into())
        })?;
    validate_bound_identity(
        parse_binding(&durable)?,
        writer_device_id,
        secret_store.as_ref(),
        &account,
    )
}

fn load_or_create_unbound_secret(
    secret_store: &dyn SecretStore,
    account: &SecretAccount,
) -> Result<DeviceSigningKey, MutationError> {
    if let Some(signing_key) = load_signing_key(secret_store, account)? {
        return Ok(signing_key);
    }

    let generated = DeviceSigningKey::generate()
        .map_err(|_| MutationError::Invalid("device signing key generation failed".into()))?;
    let seed = generated.export_seed();
    let secret = SecretBytes::from_slice(seed.as_ref()).map_err(secret_store_error)?;
    match secret_store.put(account, &secret) {
        Ok(()) => Ok(generated),
        Err(SecretStoreError::AlreadyExists) => load_signing_key(secret_store, account)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict("device-signing-secret-winner-missing".into())
            }),
        Err(error) => Err(secret_store_error(error)),
    }
}

fn validate_bound_identity(
    binding: DeviceSigningBinding,
    writer_device_id: ProtocolDeviceId,
    secret_store: &dyn SecretStore,
    account: &SecretAccount,
) -> Result<DeviceSigningIdentity, MutationError> {
    if binding.device_id != writer_device_id {
        return Err(MutationError::Invalid(
            "device signing binding does not match the Twin writer".into(),
        ));
    }
    let signing_key = load_signing_key(secret_store, account)?.ok_or_else(|| {
        MutationError::Invalid("bound device signing secret is unavailable".into())
    })?;
    let public_key = signing_key.public_key();
    if public_key != binding.public_key {
        return Err(MutationError::Invalid(
            "device signing secret does not match its public binding".into(),
        ));
    }
    Ok(DeviceSigningIdentity {
        device_id: writer_device_id,
        signing_key,
        public_key,
    })
}

fn load_signing_key(
    secret_store: &dyn SecretStore,
    account: &SecretAccount,
) -> Result<Option<DeviceSigningKey>, MutationError> {
    secret_store
        .get(account)
        .map_err(secret_store_error)?
        .map(signing_key_from_secret)
        .transpose()
}

fn signing_key_from_secret(secret: SecretBytes) -> Result<DeviceSigningKey, MutationError> {
    let seed: [u8; 32] = secret.expose().try_into().map_err(|_| {
        MutationError::Invalid("stored device signing secret has an invalid length".into())
    })?;
    let seed = Zeroizing::new(seed);
    Ok(DeviceSigningKey::from_seed(*seed))
}

fn parse_binding(bytes: &[u8]) -> Result<DeviceSigningBinding, MutationError> {
    let wire: DeviceSigningBindingWire = serde_json::from_slice(bytes).map_err(|error| {
        MutationError::Invalid(format!("invalid device signing binding: {error}"))
    })?;
    if wire.schema_version != DEVICE_SIGNING_BINDING_SCHEMA_VERSION {
        return Err(MutationError::Invalid(
            "unsupported device signing binding schema".into(),
        ));
    }
    let device_id = ProtocolDeviceId::parse_str(&wire.device_id)
        .map_err(|_| MutationError::Invalid("invalid device signing binding device ID".into()))?;
    let public_key = parse_public_key_hex(&wire.public_key_hex)?;
    Ok(DeviceSigningBinding {
        device_id,
        public_key,
    })
}

fn serialize_binding(binding: &DeviceSigningBinding) -> Result<Vec<u8>, MutationError> {
    let wire = DeviceSigningBindingWire {
        schema_version: DEVICE_SIGNING_BINDING_SCHEMA_VERSION,
        device_id: binding.device_id.to_string(),
        public_key_hex: lower_hex(binding.public_key.as_bytes()),
    };
    let mut bytes = serde_json::to_vec_pretty(&wire).map_err(|error| {
        MutationError::Invalid(format!("invalid device signing binding: {error}"))
    })?;
    bytes.push(b'\n');
    if bytes.len() > DEVICE_SIGNING_BINDING_LIMIT {
        return Err(MutationError::Invalid(
            "device signing binding exceeds its size limit".into(),
        ));
    }
    Ok(bytes)
}

fn parse_public_key_hex(value: &str) -> Result<DevicePublicKey, MutationError> {
    if value.len() != 64 {
        return Err(MutationError::Invalid(
            "device signing public key must be lowercase hex".into(),
        ));
    }
    let mut bytes = [0_u8; 32];
    for (target, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = lower_hex_value(pair[0]).ok_or_else(|| {
            MutationError::Invalid("device signing public key must be lowercase hex".into())
        })?;
        let low = lower_hex_value(pair[1]).ok_or_else(|| {
            MutationError::Invalid("device signing public key must be lowercase hex".into())
        })?;
        *target = high << 4 | low;
    }
    Ok(DevicePublicKey::from_bytes(bytes))
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

fn lower_hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn secret_store_error(error: SecretStoreError) -> MutationError {
    MutationError::Invalid(format!("device signing secret store failure: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sync::secrets::{MemorySecretStore, SecretAccount};
    use crate::services::twin_events::acquire_shared_coordinator_process_lock;
    use serde_json::Value;
    use std::fs;
    use std::sync::Barrier;

    const WRITER_A: &str = "123e4567-e89b-42d3-a456-426614174000";
    const WRITER_B: &str = "123e4567-e89b-42d3-a456-426614174001";

    fn prepared_data_root() -> (tempfile::TempDir, CoordinatorProcessLock) {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir_all(data.path().join("twin/events")).unwrap();
        let lock = acquire_shared_coordinator_process_lock(data.path()).unwrap();
        (data, lock)
    }

    fn writer(value: &str) -> TwinDeviceId {
        TwinDeviceId::parse(value).unwrap()
    }

    fn device_account() -> SecretAccount {
        SecretAccount::sync_device_ed25519()
    }

    fn seed_hex(seed: &[u8]) -> String {
        seed.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn first_creation_is_stable_on_reopen() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let writer = writer(WRITER_A);

        let first =
            load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer)
                .unwrap();
        let reopened =
            load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer)
                .unwrap();

        assert_eq!(first.device_id(), reopened.device_id());
        assert_eq!(first.public_key(), reopened.public_key());
        assert_eq!(
            first.signing_key().export_seed().as_ref(),
            reopened.signing_key().export_seed().as_ref()
        );
        assert_eq!(
            store
                .get(&device_account())
                .unwrap()
                .unwrap()
                .expose()
                .len(),
            32
        );
    }

    #[test]
    fn concurrent_unbound_device_creation_converges_on_the_first_secret() {
        let store = Arc::new(MemorySecretStore::default());
        let start = Arc::new(Barrier::new(3));
        let contenders = [(), ()].map(|()| {
            let store = store.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let key = load_or_create_unbound_secret(
                    store.as_ref(),
                    &SecretAccount::sync_device_ed25519(),
                )
                .unwrap();
                let seed = key.export_seed();
                *seed
            })
        });
        start.wait();
        let seeds = contenders.map(|contender| contender.join().unwrap());

        assert_eq!(seeds[0], seeds[1]);
        assert_eq!(
            store
                .get(&SecretAccount::sync_device_ed25519())
                .unwrap()
                .unwrap()
                .expose(),
            seeds[0]
        );
    }

    #[test]
    fn binding_reuses_the_existing_twin_writer_device_id() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let writer = writer(WRITER_A);

        let identity =
            load_or_create_device_signing_identity(data.path(), &lock, store, &writer).unwrap();
        let artifact: Value = serde_json::from_slice(
            &fs::read(data.path().join(DEVICE_SIGNING_BINDING_KEY)).unwrap(),
        )
        .unwrap();

        assert_eq!(identity.device_id().to_string(), WRITER_A);
        assert_eq!(artifact["device_id"], WRITER_A);
    }

    #[test]
    fn existing_binding_with_missing_secret_fails_without_rotation() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let writer = writer(WRITER_A);
        load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer).unwrap();
        let binding_path = data.path().join(DEVICE_SIGNING_BINDING_KEY);
        let before = fs::read(&binding_path).unwrap();
        store.delete(&device_account()).unwrap();

        assert!(
            load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer)
                .is_err()
        );

        assert_eq!(fs::read(binding_path).unwrap(), before);
        assert!(store.get(&device_account()).unwrap().is_none());
    }

    #[test]
    fn corrupt_or_unsupported_binding_fails_unchanged() {
        let invalid_bindings = [
            br#"not json"#.as_slice(),
            br#"{"schema_version":2,"device_id":"123e4567-e89b-42d3-a456-426614174000","public_key_hex":"1111111111111111111111111111111111111111111111111111111111111111"}"#,
            br#"{"schema_version":1,"device_id":"123E4567-E89B-42D3-A456-426614174000","public_key_hex":"1111111111111111111111111111111111111111111111111111111111111111"}"#,
            br#"{"schema_version":1,"device_id":"123e4567-e89b-42d3-a456-426614174000","public_key_hex":"111111111111111111111111111111111111111111111111111111111111111A"}"#,
            br#"{"schema_version":1,"device_id":"123e4567-e89b-42d3-a456-426614174000","public_key_hex":"11","extra":true}"#,
        ];

        for bytes in invalid_bindings {
            let (data, lock) = prepared_data_root();
            let store = Arc::new(MemorySecretStore::default());
            let binding_path = data.path().join(DEVICE_SIGNING_BINDING_KEY);
            fs::write(&binding_path, bytes).unwrap();

            assert!(load_or_create_device_signing_identity(
                data.path(),
                &lock,
                store.clone(),
                &writer(WRITER_A)
            )
            .is_err());
            assert_eq!(fs::read(binding_path).unwrap(), bytes);
            assert!(store.get(&device_account()).unwrap().is_none());
        }
    }

    #[test]
    fn oversized_binding_fails_unchanged() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let binding_path = data.path().join(DEVICE_SIGNING_BINDING_KEY);
        let bytes = vec![b'x'; DEVICE_SIGNING_BINDING_LIMIT + 1];
        fs::write(&binding_path, &bytes).unwrap();

        assert!(load_or_create_device_signing_identity(
            data.path(),
            &lock,
            store.clone(),
            &writer(WRITER_A)
        )
        .is_err());
        assert_eq!(fs::read(binding_path).unwrap(), bytes);
        assert!(store.get(&device_account()).unwrap().is_none());
    }

    #[test]
    fn binding_for_a_different_writer_fails_without_changing_key_or_artifact() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        load_or_create_device_signing_identity(
            data.path(),
            &lock,
            store.clone(),
            &writer(WRITER_A),
        )
        .unwrap();
        let binding_path = data.path().join(DEVICE_SIGNING_BINDING_KEY);
        let before_binding = fs::read(&binding_path).unwrap();
        let before_secret = store.get(&device_account()).unwrap().unwrap();

        assert!(load_or_create_device_signing_identity(
            data.path(),
            &lock,
            store.clone(),
            &writer(WRITER_B)
        )
        .is_err());

        assert_eq!(fs::read(binding_path).unwrap(), before_binding);
        assert_eq!(
            store.get(&device_account()).unwrap().unwrap().expose(),
            before_secret.expose()
        );
    }

    #[test]
    fn bound_public_key_mismatch_fails_without_rotating_the_replacement_secret() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let writer = writer(WRITER_A);
        load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer).unwrap();
        let binding_path = data.path().join(DEVICE_SIGNING_BINDING_KEY);
        let before_binding = fs::read(&binding_path).unwrap();
        let replacement_seed = [0x71; 32];
        store.delete(&device_account()).unwrap();
        store
            .put(
                &device_account(),
                &SecretBytes::from_slice(&replacement_seed).unwrap(),
            )
            .unwrap();

        assert!(
            load_or_create_device_signing_identity(data.path(), &lock, store.clone(), &writer)
                .is_err()
        );

        assert_eq!(fs::read(binding_path).unwrap(), before_binding);
        assert_eq!(
            store.get(&device_account()).unwrap().unwrap().expose(),
            replacement_seed
        );
    }

    #[test]
    fn existing_unbound_secret_is_adopted_without_replacement() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let seed = [0x42; 32];
        store
            .put(&device_account(), &SecretBytes::from_slice(&seed).unwrap())
            .unwrap();

        let identity = load_or_create_device_signing_identity(
            data.path(),
            &lock,
            store.clone(),
            &writer(WRITER_A),
        )
        .unwrap();

        assert_eq!(identity.signing_key().export_seed().as_ref(), &seed);
        assert_eq!(
            store.get(&device_account()).unwrap().unwrap().expose(),
            seed
        );
    }

    #[test]
    fn foreign_process_lock_is_rejected_before_writing_secret_or_binding() {
        let (data, _data_lock) = prepared_data_root();
        let (_foreign, foreign_lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());

        assert!(load_or_create_device_signing_identity(
            data.path(),
            &foreign_lock,
            store.clone(),
            &writer(WRITER_A)
        )
        .is_err());

        assert!(!data.path().join(DEVICE_SIGNING_BINDING_KEY).exists());
        assert!(store.get(&device_account()).unwrap().is_none());
    }

    #[test]
    fn public_artifact_and_debug_never_include_the_private_seed() {
        let (data, lock) = prepared_data_root();
        let store = Arc::new(MemorySecretStore::default());
        let seed = [0x5a; 32];
        store
            .put(&device_account(), &SecretBytes::from_slice(&seed).unwrap())
            .unwrap();
        let identity =
            load_or_create_device_signing_identity(data.path(), &lock, store, &writer(WRITER_A))
                .unwrap();

        let artifact = fs::read_to_string(data.path().join(DEVICE_SIGNING_BINDING_KEY)).unwrap();
        let artifact_json: Value = serde_json::from_str(&artifact).unwrap();
        let debug = format!("{identity:?}");
        let private_seed_hex = seed_hex(&seed);

        assert_eq!(artifact_json.as_object().unwrap().len(), 3);
        assert_eq!(artifact_json["schema_version"], 1);
        assert_eq!(artifact_json["device_id"], WRITER_A);
        assert_eq!(
            artifact_json["public_key_hex"],
            lower_hex(identity.public_key().as_bytes())
        );
        assert!(!artifact.contains(&private_seed_hex));
        assert!(!artifact.contains("seed"));
        assert!(!debug.contains(&private_seed_hex));
        assert!(debug.contains("[REDACTED]"));
    }
}
