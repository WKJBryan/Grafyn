#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;

use grafyn_sync_protocol::VaultRootKey;
use zeroize::Zeroizing;

use super::secrets::{SecretAccount, SecretBytes, SecretStore, SecretStoreError};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum VaultKeyError {
    InvalidVaultId,
    AlreadyProvisioned,
    CorruptStoredKey,
    StoreUnavailable,
}

impl fmt::Display for VaultKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidVaultId => "invalid vault identity",
            Self::AlreadyProvisioned => "vault root key is already provisioned",
            Self::CorruptStoredKey => "stored vault root key is invalid",
            Self::StoreUnavailable => "vault root key store is unavailable",
        })
    }
}

impl std::error::Error for VaultKeyError {}

pub(crate) fn load_vault_root_key(
    store: &dyn SecretStore,
    vault_id: &str,
) -> Result<Option<VaultRootKey>, VaultKeyError> {
    let account = vault_account(vault_id)?;
    let Some(secret) = store.get(&account).map_err(map_store_error)? else {
        return Ok(None);
    };
    if secret.expose().len() != 32 {
        return Err(VaultKeyError::CorruptStoredKey);
    }

    let mut bytes = Zeroizing::new([0u8; 32]);
    bytes.copy_from_slice(secret.expose());
    Ok(Some(VaultRootKey::from_bytes(*bytes)))
}

pub(crate) fn provision_vault_root_key(
    store: &dyn SecretStore,
    vault_id: &str,
    root_key: &VaultRootKey,
) -> Result<(), VaultKeyError> {
    let account = vault_account(vault_id)?;
    let exported = root_key.export_bytes();
    let secret = SecretBytes::from_slice(exported.as_ref()).map_err(map_store_error)?;
    store.put(&account, &secret).map_err(map_store_error)
}

fn vault_account(vault_id: &str) -> Result<SecretAccount, VaultKeyError> {
    SecretAccount::sync_vault_root(vault_id).map_err(|_| VaultKeyError::InvalidVaultId)
}

fn map_store_error(error: SecretStoreError) -> VaultKeyError {
    match error {
        SecretStoreError::InvalidAccount => VaultKeyError::InvalidVaultId,
        SecretStoreError::AlreadyExists => VaultKeyError::AlreadyProvisioned,
        SecretStoreError::CorruptSecret => VaultKeyError::CorruptStoredKey,
        SecretStoreError::InvalidSecret
        | SecretStoreError::SecretTooLarge
        | SecretStoreError::BackendUnavailable => VaultKeyError::StoreUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sync::secrets::{
        MemorySecretStore, SecretAccount, SecretBytes, SecretStore,
    };
    use std::sync::{Arc, Barrier};

    const VAULT_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

    #[test]
    fn missing_vault_root_key_is_not_provisioned_and_stays_absent() {
        let store = MemorySecretStore::default();

        assert!(load_vault_root_key(&store, VAULT_ID).unwrap().is_none());
        assert!(load_vault_root_key(&store, VAULT_ID).unwrap().is_none());
        assert!(store
            .get(&SecretAccount::sync_vault_root(VAULT_ID).unwrap())
            .unwrap()
            .is_none());
    }

    #[test]
    fn explicit_provision_round_trips_the_exact_root_key() {
        let store = MemorySecretStore::default();
        let expected = VaultRootKey::from_bytes([7; 32]);

        provision_vault_root_key(&store, VAULT_ID, &expected).unwrap();

        let loaded = load_vault_root_key(&store, VAULT_ID).unwrap().unwrap();
        assert_eq!(loaded.export_bytes().as_ref(), &[7; 32]);
    }

    #[test]
    fn second_provision_is_rejected_and_preserves_the_first_key() {
        let store = MemorySecretStore::default();
        provision_vault_root_key(&store, VAULT_ID, &VaultRootKey::from_bytes([1; 32])).unwrap();

        assert_eq!(
            provision_vault_root_key(&store, VAULT_ID, &VaultRootKey::from_bytes([2; 32]))
                .unwrap_err(),
            VaultKeyError::AlreadyProvisioned
        );
        assert_eq!(
            load_vault_root_key(&store, VAULT_ID)
                .unwrap()
                .unwrap()
                .export_bytes()
                .as_ref(),
            &[1; 32]
        );
    }

    #[test]
    fn corrupt_stored_root_key_length_is_rejected() {
        let store = MemorySecretStore::default();
        store
            .put(
                &SecretAccount::sync_vault_root(VAULT_ID).unwrap(),
                &SecretBytes::new(vec![5; 31]).unwrap(),
            )
            .unwrap();

        assert_eq!(
            load_vault_root_key(&store, VAULT_ID).unwrap_err(),
            VaultKeyError::CorruptStoredKey
        );
    }

    #[test]
    fn only_deliberate_export_copy_shares_a_vault_key_between_stores() {
        let first_store = MemorySecretStore::default();
        let second_store = MemorySecretStore::default();
        let source = VaultRootKey::from_bytes([9; 32]);
        let exported = source.export_bytes();
        let first_copy = VaultRootKey::from_bytes(*exported);
        let second_copy = VaultRootKey::from_bytes(*exported);

        provision_vault_root_key(&first_store, VAULT_ID, &first_copy).unwrap();
        provision_vault_root_key(&second_store, VAULT_ID, &second_copy).unwrap();
        first_store
            .put(
                &SecretAccount::sync_device_ed25519(),
                &SecretBytes::new(vec![3; 32]).unwrap(),
            )
            .unwrap();

        let first = load_vault_root_key(&first_store, VAULT_ID)
            .unwrap()
            .unwrap()
            .export_bytes();
        let second = load_vault_root_key(&second_store, VAULT_ID)
            .unwrap()
            .unwrap()
            .export_bytes();
        assert_eq!(first.as_ref(), exported.as_ref());
        assert_eq!(second.as_ref(), exported.as_ref());
        assert!(second_store
            .get(&SecretAccount::sync_device_ed25519())
            .unwrap()
            .is_none());
    }

    #[test]
    fn concurrent_vault_root_provisioning_keeps_exactly_one_key() {
        let store = Arc::new(MemorySecretStore::default());
        let start = Arc::new(Barrier::new(3));
        let contenders = [0x31, 0x72].map(|byte| {
            let store = store.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                (
                    byte,
                    provision_vault_root_key(
                        store.as_ref(),
                        VAULT_ID,
                        &VaultRootKey::from_bytes([byte; 32]),
                    ),
                )
            })
        });
        start.wait();
        let results = contenders.map(|contender| contender.join().unwrap());

        let winners = results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .map(|(byte, _)| *byte)
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|(_, result)| matches!(result, Err(VaultKeyError::AlreadyProvisioned)))
                .count(),
            1
        );
        assert_eq!(
            load_vault_root_key(store.as_ref(), VAULT_ID)
                .unwrap()
                .unwrap()
                .export_bytes()
                .as_ref(),
            &[winners[0]; 32]
        );
    }
}
