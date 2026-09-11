use crate::models::sync::{VaultDescriptorV1, VaultId};
use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{AnchoredRoot, MutationError};
use std::path::Path;

pub(crate) const VAULT_DESCRIPTOR_KEY: &str = "_grafyn/vault.json";
pub(crate) const VAULT_DESCRIPTOR_LIMIT: usize = 4096;
const VAULT_SCOPE_DOMAIN: &[u8] = b"grafyn.vault_scope.v1";
const VAULT_DESCRIPTOR_STAGING_DIRECTORY: &str = "_grafyn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VaultIdentity {
    pub(crate) descriptor: VaultDescriptorV1,
    pub(crate) root_scope: ContentDigest,
}

pub(crate) fn load_or_create_vault_identity(
    vault_path: &Path,
) -> Result<VaultIdentity, MutationError> {
    let root = AnchoredRoot::open(vault_path)?;
    if let Some(bytes) = root.read_bounded(VAULT_DESCRIPTOR_KEY, VAULT_DESCRIPTOR_LIMIT)? {
        return identity_from_descriptor_bytes(&bytes);
    }

    let descriptor = VaultDescriptorV1::generate();
    let mut bytes = serde_json::to_vec_pretty(&descriptor)
        .map_err(|error| MutationError::Invalid(format!("invalid vault descriptor: {error}")))?;
    bytes.push(b'\n');
    if bytes.len() > VAULT_DESCRIPTOR_LIMIT {
        return Err(MutationError::Invalid(
            "vault descriptor exceeds its size limit".into(),
        ));
    }
    root.install_no_clobber(
        VAULT_DESCRIPTOR_KEY,
        VAULT_DESCRIPTOR_STAGING_DIRECTORY,
        &bytes,
    )?;

    let durable = root
        .read_bounded(VAULT_DESCRIPTOR_KEY, VAULT_DESCRIPTOR_LIMIT)?
        .ok_or_else(|| {
            MutationError::RecoveryConflict("vault-descriptor-missing-after-install".into())
        })?;
    identity_from_descriptor_bytes(&durable)
}

pub(crate) fn load_vault_identity(vault_path: &Path) -> Result<VaultIdentity, MutationError> {
    let root = AnchoredRoot::open(vault_path)?;
    let bytes = root
        .read_bounded(VAULT_DESCRIPTOR_KEY, VAULT_DESCRIPTOR_LIMIT)?
        .ok_or_else(|| MutationError::RecoveryConflict("vault-descriptor-missing".into()))?;
    identity_from_descriptor_bytes(&bytes)
}

pub(crate) fn stable_vault_scope(vault_id: &VaultId) -> ContentDigest {
    let uuid = vault_id.as_bytes();
    let mut preimage = Vec::with_capacity(VAULT_SCOPE_DOMAIN.len() + 8 + uuid.len());
    preimage.extend_from_slice(VAULT_SCOPE_DOMAIN);
    preimage.extend_from_slice(&(uuid.len() as u64).to_be_bytes());
    preimage.extend_from_slice(uuid);
    crate::services::twin_events::digest_bytes(&preimage)
}

pub(crate) fn legacy_path_scope_for_migration(
    vault_path: &Path,
) -> Result<ContentDigest, MutationError> {
    crate::services::twin_events::root_identity_for_path(vault_path)
}

fn identity_from_descriptor_bytes(bytes: &[u8]) -> Result<VaultIdentity, MutationError> {
    let descriptor: VaultDescriptorV1 = serde_json::from_slice(bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid vault descriptor: {error}")))?;
    let root_scope = stable_vault_scope(descriptor.vault_id());
    Ok(VaultIdentity {
        descriptor,
        root_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[test]
    fn first_open_installs_and_reopens_one_identity() {
        let vault = tempfile::tempdir().unwrap();

        let first = load_or_create_vault_identity(vault.path()).unwrap();
        let reopened = load_or_create_vault_identity(vault.path()).unwrap();

        assert_eq!(reopened, first);
        let bytes = fs::read(vault.path().join(VAULT_DESCRIPTOR_KEY)).unwrap();
        assert!(bytes.len() <= VAULT_DESCRIPTOR_LIMIT);
        assert_eq!(
            serde_json::from_slice::<VaultDescriptorV1>(&bytes).unwrap(),
            first.descriptor
        );
        assert_eq!(load_vault_identity(vault.path()).unwrap(), first);
    }

    #[test]
    fn strict_load_never_creates_a_missing_descriptor() {
        let vault = tempfile::tempdir().unwrap();

        assert!(load_vault_identity(vault.path()).is_err());
        assert!(!vault.path().join(VAULT_DESCRIPTOR_KEY).exists());
    }

    #[test]
    fn identity_and_scope_survive_a_vault_move() {
        let parent = tempfile::tempdir().unwrap();
        let original = parent.path().join("original");
        let moved = parent.path().join("moved");
        fs::create_dir(&original).unwrap();
        let first = load_or_create_vault_identity(&original).unwrap();

        fs::rename(&original, &moved).unwrap();
        let reopened = load_or_create_vault_identity(&moved).unwrap();

        assert_eq!(reopened, first);
    }

    #[test]
    fn invalid_existing_descriptor_is_never_replaced() {
        for bytes in [
            br#"not json"#.as_slice(),
            br#"{"schema_version":2,"vault_id":"123e4567-e89b-42d3-a456-426614174000"}"#,
            br#"{"schema_version":1,"vault_id":"00000000-0000-0000-0000-000000000000"}"#,
            br#"{"schema_version":1,"vault_id":"123e4567-e89b-42d3-a456-426614174000","extra":true}"#,
        ] {
            let vault = tempfile::tempdir().unwrap();
            let descriptor_path = vault.path().join(VAULT_DESCRIPTOR_KEY);
            fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
            fs::write(&descriptor_path, bytes).unwrap();

            assert!(load_or_create_vault_identity(vault.path()).is_err());
            assert_eq!(fs::read(descriptor_path).unwrap(), bytes);
        }
    }

    #[test]
    fn oversized_existing_descriptor_is_never_replaced() {
        let vault = tempfile::tempdir().unwrap();
        let descriptor_path = vault.path().join(VAULT_DESCRIPTOR_KEY);
        fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let bytes = vec![b'x'; VAULT_DESCRIPTOR_LIMIT + 1];
        fs::write(&descriptor_path, &bytes).unwrap();

        assert!(load_or_create_vault_identity(vault.path()).is_err());
        assert_eq!(fs::read(descriptor_path).unwrap(), bytes);
    }

    #[test]
    fn concurrent_creators_all_adopt_the_installed_winner() {
        const CREATOR_COUNT: usize = 8;
        let vault = tempfile::tempdir().unwrap();
        let path = Arc::new(vault.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(CREATOR_COUNT));
        let handles = (0..CREATOR_COUNT)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    load_or_create_vault_identity(path.as_path()).unwrap()
                })
            })
            .collect::<Vec<_>>();

        let identities = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert!(identities.iter().all(|identity| identity == &identities[0]));
    }

    #[test]
    fn stable_scope_has_a_domain_separated_fixed_vector() {
        let vault_id = VaultId::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap();

        assert_eq!(
            stable_vault_scope(&vault_id).as_str(),
            "69645ea74cab745ce52b3e76d3c7b74307356d30d6117317d222e45db4580a46"
        );
    }

    #[test]
    fn legacy_path_scope_is_explicitly_separate_from_stable_identity() {
        let vault = tempfile::tempdir().unwrap();
        let identity = load_or_create_vault_identity(vault.path()).unwrap();

        let legacy = legacy_path_scope_for_migration(vault.path()).unwrap();

        assert_ne!(legacy, identity.root_scope);
    }

    #[test]
    fn descriptor_symlink_is_rejected_without_touching_its_target() {
        let parent = tempfile::tempdir().unwrap();
        let vault = parent.path().join("vault");
        let outside = parent.path().join("outside.json");
        let original = br#"{"schema_version":1,"vault_id":"123e4567-e89b-42d3-a456-426614174000"}"#;
        fs::create_dir(&vault).unwrap();
        fs::write(&outside, original).unwrap();
        let descriptor_path = vault.join(VAULT_DESCRIPTOR_KEY);
        fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        if let Err(error) = symlink_file(&outside, &descriptor_path) {
            // Windows may deny symlink creation unless Developer Mode or the
            // required privilege is enabled. That host capability boundary is
            // recorded here; a created link must still be rejected below.
            #[cfg(windows)]
            {
                eprintln!("skipping descriptor symlink regression: {error}");
                return;
            }
            #[cfg(not(windows))]
            panic!("test setup requires a descriptor symlink: {error}");
        }

        assert!(load_or_create_vault_identity(&vault).is_err());
        assert_eq!(fs::read(outside).unwrap(), original);
    }
}
