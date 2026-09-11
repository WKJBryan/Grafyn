use std::fmt;

use chacha20poly1305::{aead::AeadInOut, KeyInit as AeadKeyInit, Tag, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{ed25519::signature::Signer as _, Signature, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{
    canonical, DeviceId, DevicePublicKey, EnvelopeV1, OperationId, OperationV1, ProtocolError,
    Result, VaultId, VerifiedOperation,
};

const AEAD_KEY_PURPOSE: &str = "xchacha20poly1305-key";
const OPERATION_ID_KEY_PURPOSE: &str = "hmac-sha256-operation-id-key";
const AEAD_TAG_BYTES: usize = 16;

pub struct VaultRootKey(Zeroizing<[u8; 32]>);

impl VaultRootKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| ProtocolError::RandomnessUnavailable)?;
        Ok(Self::from_bytes(bytes))
    }

    /// Returns a zeroizing copy for persistence in an external secret store.
    pub fn export_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.0)
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for VaultRootKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultRootKey([REDACTED])")
    }
}

pub struct DeviceSigningKey(Zeroizing<[u8; 32]>);

impl DeviceSigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(Zeroizing::new(seed))
    }

    pub fn generate() -> Result<Self> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|_| ProtocolError::RandomnessUnavailable)?;
        Ok(Self::from_seed(seed))
    }

    /// Returns a zeroizing seed copy for persistence in an external secret store.
    pub fn export_seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.0)
    }

    pub fn public_key(&self) -> DevicePublicKey {
        let signing_key = SigningKey::from_bytes(&self.0);
        DevicePublicKey::from_bytes(signing_key.verifying_key().to_bytes())
    }

    fn sign(&self, message: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(&self.0).sign(message).to_bytes()
    }
}

impl fmt::Debug for DeviceSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceSigningKey([REDACTED])")
    }
}

pub struct TrustedDevice {
    device_id: DeviceId,
    public_key: DevicePublicKey,
    verifying_key: VerifyingKey,
}

impl TrustedDevice {
    pub fn new(device_id: DeviceId, public_key: DevicePublicKey) -> Result<Self> {
        let verifying_key = VerifyingKey::from_bytes(public_key.as_bytes())
            .map_err(|_| ProtocolError::InvalidField("device_public_key"))?;
        if verifying_key.is_weak() {
            return Err(ProtocolError::InvalidField("device_public_key"));
        }
        Ok(Self {
            device_id,
            public_key,
            verifying_key,
        })
    }

    pub const fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub const fn public_key(&self) -> &DevicePublicKey {
        &self.public_key
    }
}

impl fmt::Debug for TrustedDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedDevice")
            .field("device_id", &self.device_id)
            .field("public_key", &self.public_key)
            .finish()
    }
}

pub fn seal_operation(
    root_key: &VaultRootKey,
    vault_id: &VaultId,
    device_id: &DeviceId,
    signing_key: &DeviceSigningKey,
    operation: &OperationV1,
) -> Result<EnvelopeV1> {
    let plaintext = Zeroizing::new(canonical::encode_operation(operation)?);
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| ProtocolError::RandomnessUnavailable)?;
    seal_plaintext_with_nonce(
        root_key,
        vault_id,
        device_id,
        signing_key,
        &plaintext,
        nonce,
    )
}

pub fn open_operation(
    root_key: &VaultRootKey,
    expected_vault_id: &VaultId,
    trusted_device: &TrustedDevice,
    envelope: &EnvelopeV1,
) -> Result<VerifiedOperation> {
    if envelope.vault_id() != expected_vault_id {
        return Err(ProtocolError::IdentityMismatch("vault_id"));
    }
    if envelope.device_id() != trusted_device.device_id() {
        return Err(ProtocolError::IdentityMismatch("device_id"));
    }
    if envelope.device_public_key() != trusted_device.public_key() {
        return Err(ProtocolError::IdentityMismatch("device_public_key"));
    }

    let aad = canonical::envelope_aad(
        envelope.vault_id(),
        envelope.device_id(),
        envelope.device_public_key(),
        envelope.operation_id(),
        envelope.nonce(),
    )?;
    let signature_message = canonical::signature_message(&aad, envelope.ciphertext())?;
    let signature = Signature::from_bytes(envelope.signature());
    trusted_device
        .verifying_key
        .verify_strict(&signature_message, &signature)
        .map_err(|_| ProtocolError::InvalidSignature)?;

    let aead_key = derive_subkey(
        root_key,
        AEAD_KEY_PURPOSE,
        envelope.vault_id(),
        envelope.device_id(),
        envelope.device_public_key(),
    )?;
    let cipher = <XChaCha20Poly1305 as AeadKeyInit>::new_from_slice(aead_key.as_ref())
        .map_err(|_| ProtocolError::KeyDerivationFailed)?;
    let tag_offset = envelope
        .ciphertext()
        .len()
        .checked_sub(AEAD_TAG_BYTES)
        .ok_or(ProtocolError::InvalidField("ciphertext"))?;
    let (encrypted, tag_bytes) = envelope.ciphertext().split_at(tag_offset);
    let mut plaintext = Zeroizing::new(encrypted.to_vec());
    let nonce = XNonce::from(*envelope.nonce());
    let tag = Tag::from(
        <[u8; AEAD_TAG_BYTES]>::try_from(tag_bytes)
            .map_err(|_| ProtocolError::InvalidField("ciphertext"))?,
    );
    cipher
        .decrypt_inout_detached(&nonce, &aad, plaintext.as_mut_slice().into(), &tag)
        .map_err(|_| ProtocolError::AuthenticationFailed)?;

    let operation = canonical::decode_operation(&plaintext)?;
    let operation_id_key = derive_subkey(
        root_key,
        OPERATION_ID_KEY_PURPOSE,
        envelope.vault_id(),
        envelope.device_id(),
        envelope.device_public_key(),
    )?;
    let id_message = Zeroizing::new(canonical::operation_id_message(
        envelope.vault_id(),
        envelope.device_id(),
        envelope.device_public_key(),
        &plaintext,
    )?);
    let mut id_mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(operation_id_key.as_ref())
        .map_err(|_| ProtocolError::KeyDerivationFailed)?;
    id_mac.update(&id_message);
    id_mac
        .verify_slice(envelope.operation_id().as_bytes())
        .map_err(|_| ProtocolError::OperationIdMismatch)?;

    Ok(VerifiedOperation::new(
        *envelope.vault_id(),
        *envelope.device_id(),
        *envelope.device_public_key(),
        *envelope.operation_id(),
        operation,
    ))
}

fn seal_plaintext_with_nonce(
    root_key: &VaultRootKey,
    vault_id: &VaultId,
    device_id: &DeviceId,
    signing_key: &DeviceSigningKey,
    plaintext: &[u8],
    nonce: [u8; 24],
) -> Result<EnvelopeV1> {
    let public_key = signing_key.public_key();
    let operation_id_key = derive_subkey(
        root_key,
        OPERATION_ID_KEY_PURPOSE,
        vault_id,
        device_id,
        &public_key,
    )?;
    let id_message = Zeroizing::new(canonical::operation_id_message(
        vault_id,
        device_id,
        &public_key,
        plaintext,
    )?);
    let operation_id = compute_operation_id(&operation_id_key, &id_message)?;
    seal_plaintext_with_id_and_nonce(
        root_key,
        vault_id,
        device_id,
        signing_key,
        plaintext,
        operation_id,
        nonce,
    )
}

fn seal_plaintext_with_id_and_nonce(
    root_key: &VaultRootKey,
    vault_id: &VaultId,
    device_id: &DeviceId,
    signing_key: &DeviceSigningKey,
    plaintext: &[u8],
    operation_id: OperationId,
    nonce: [u8; 24],
) -> Result<EnvelopeV1> {
    let public_key = signing_key.public_key();
    let aad = canonical::envelope_aad(vault_id, device_id, &public_key, &operation_id, &nonce)?;
    let aead_key = derive_subkey(root_key, AEAD_KEY_PURPOSE, vault_id, device_id, &public_key)?;
    let cipher = <XChaCha20Poly1305 as AeadKeyInit>::new_from_slice(aead_key.as_ref())
        .map_err(|_| ProtocolError::KeyDerivationFailed)?;
    let mut ciphertext = plaintext.to_vec();
    let nonce_value = XNonce::from(nonce);
    let tag = cipher
        .encrypt_inout_detached(&nonce_value, &aad, ciphertext.as_mut_slice().into())
        .map_err(|_| ProtocolError::AuthenticationFailed)?;
    ciphertext.extend_from_slice(&tag);

    let signature_message = canonical::signature_message(&aad, &ciphertext)?;
    let signature = signing_key.sign(&signature_message);
    EnvelopeV1::new(
        *vault_id,
        *device_id,
        public_key,
        operation_id,
        nonce,
        ciphertext,
        signature,
    )
}

fn derive_subkey(
    root_key: &VaultRootKey,
    purpose: &str,
    vault_id: &VaultId,
    device_id: &DeviceId,
    device_public_key: &DevicePublicKey,
) -> Result<Zeroizing<[u8; 32]>> {
    let info = canonical::hkdf_info(purpose, vault_id, device_id, device_public_key)?;
    let hkdf = Hkdf::<Sha256>::new(Some(canonical::HKDF_SALT), root_key.as_bytes());
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(&info, key.as_mut())
        .map_err(|_| ProtocolError::KeyDerivationFailed)?;
    Ok(key)
}

fn compute_operation_id(key: &[u8; 32], message: &[u8]) -> Result<OperationId> {
    let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(key)
        .map_err(|_| ProtocolError::KeyDerivationFailed)?;
    mac.update(message);
    let bytes: [u8; 32] = mac.finalize().into_bytes().into();
    Ok(OperationId::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NoteRevisionV1, OperationPayloadV1};

    fn fixed_context() -> (VaultRootKey, VaultId, DeviceId, DeviceSigningKey) {
        (
            VaultRootKey::from_bytes([0x11; 32]),
            VaultId::parse_str("018f1f09-7b5a-7cc4-98c0-71acb24f24d3").unwrap(),
            DeviceId::parse_str("018f1f0a-4050-7aca-aebe-16a510f897e8").unwrap(),
            DeviceSigningKey::from_seed([0x22; 32]),
        )
    }

    fn fixed_operation() -> OperationV1 {
        OperationV1::new(
            1_725_000_000_123,
            Vec::new(),
            OperationPayloadV1::NoteRevision(
                NoteRevisionV1::put(
                    "note-golden",
                    "# Golden\n\nEncrypted sync vector.\n".to_owned(),
                )
                .unwrap(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn fixed_nonce_matches_the_committed_golden_envelope() {
        let (root_key, vault_id, device_id, signing_key) = fixed_context();
        let plaintext = canonical::encode_operation(&fixed_operation()).unwrap();
        let envelope = seal_plaintext_with_nonce(
            &root_key,
            &vault_id,
            &device_id,
            &signing_key,
            &plaintext,
            [0x33; 24],
        )
        .unwrap();

        assert_eq!(
            envelope.to_json().unwrap(),
            include_str!("../testdata/envelope-v1.json").trim()
        );
    }

    #[test]
    fn authenticated_payload_type_mismatch_is_rejected() {
        let (root_key, vault_id, device_id, signing_key) = fixed_context();
        let plaintext =
            canonical::encode_operation_with_claimed_type(&fixed_operation(), "twin_event")
                .unwrap();
        let envelope = seal_plaintext_with_nonce(
            &root_key,
            &vault_id,
            &device_id,
            &signing_key,
            &plaintext,
            [0x44; 24],
        )
        .unwrap();
        let trusted = TrustedDevice::new(device_id, signing_key.public_key()).unwrap();

        assert!(matches!(
            open_operation(&root_key, &vault_id, &trusted, &envelope),
            Err(ProtocolError::InvalidCanonicalEncoding)
        ));
    }

    #[test]
    fn authenticated_wrong_operation_id_is_rejected_after_decryption() {
        let (root_key, vault_id, device_id, signing_key) = fixed_context();
        let plaintext = canonical::encode_operation(&fixed_operation()).unwrap();
        let envelope = seal_plaintext_with_id_and_nonce(
            &root_key,
            &vault_id,
            &device_id,
            &signing_key,
            &plaintext,
            OperationId::from_bytes([0x55; 32]),
            [0x66; 24],
        )
        .unwrap();
        let trusted = TrustedDevice::new(device_id, signing_key.public_key()).unwrap();

        assert!(matches!(
            open_operation(&root_key, &vault_id, &trusted, &envelope),
            Err(ProtocolError::OperationIdMismatch)
        ));
    }

    #[test]
    fn persistence_exports_are_exact_zeroizing_copies() {
        let root = VaultRootKey::from_bytes([0x71; 32]);
        let signing = DeviceSigningKey::from_seed([0x92; 32]);

        let root_bytes = root.export_bytes();
        let signing_seed = signing.export_seed();

        assert_eq!(root_bytes.as_ref(), &[0x71; 32]);
        assert_eq!(signing_seed.as_ref(), &[0x92; 32]);
        assert_eq!(format!("{root:?}"), "VaultRootKey([REDACTED])");
        assert_eq!(format!("{signing:?}"), "DeviceSigningKey([REDACTED])");
    }
}
