use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::{
    ProtocolError, Result, ATTACHMENT_CHUNK_BYTES, ENVELOPE_PROTOCOL, ENVELOPE_SCHEMA_VERSION,
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_CHUNKS, MAX_CAUSAL_PARENTS, MAX_CIPHERTEXT_BYTES,
    MAX_ENVELOPE_JSON_BYTES, MAX_MEDIA_TYPE_BYTES, MAX_NOTE_ID_BYTES, MAX_NOTE_MARKDOWN_BYTES,
    MAX_TWIN_EVENT_BYTES,
};

macro_rules! uuid_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn parse_str(value: &str) -> Result<Self> {
                parse_uuid(value)
                    .map(Self)
                    .ok_or(ProtocolError::InvalidField($field))
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_uuid(formatter, &self.0)
            }
        }

        impl FromStr for $name {
            type Err = ProtocolError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse_str(value)
            }
        }
    };
}

uuid_id!(VaultId, "vault_id");
uuid_id!(DeviceId, "device_id");

macro_rules! bytes32_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            pub fn parse_hex(value: &str) -> Result<Self> {
                parse_hex_32(value)
                    .map(Self)
                    .ok_or(ProtocolError::InvalidField($field))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_lower_hex(formatter, &self.0)
            }
        }

        impl FromStr for $name {
            type Err = ProtocolError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse_hex(value)
            }
        }
    };
}

bytes32_id!(OperationId, "operation_id");
bytes32_id!(Digest32, "digest");

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DevicePublicKey([u8; 32]);

impl DevicePublicKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum NoteRevisionKind {
    Put { markdown: String },
    Tombstone,
}

impl fmt::Debug for NoteRevisionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Put { markdown } => formatter
                .debug_struct("Put")
                .field("markdown_len", &markdown.len())
                .finish(),
            Self::Tombstone => formatter.write_str("Tombstone"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct NoteRevisionV1 {
    note_id: String,
    kind: NoteRevisionKind,
}

impl fmt::Debug for NoteRevisionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoteRevisionV1")
            .field("note_id_len", &self.note_id.len())
            .field("kind", &self.kind)
            .finish()
    }
}

impl NoteRevisionV1 {
    pub fn put(note_id: impl Into<String>, markdown: String) -> Result<Self> {
        Self::new(note_id.into(), NoteRevisionKind::Put { markdown })
    }

    pub fn tombstone(note_id: impl Into<String>) -> Result<Self> {
        Self::new(note_id.into(), NoteRevisionKind::Tombstone)
    }

    pub(crate) fn new(note_id: String, kind: NoteRevisionKind) -> Result<Self> {
        validate_identifier(&note_id, MAX_NOTE_ID_BYTES, "note_id")?;
        if let NoteRevisionKind::Put { markdown } = &kind {
            if markdown.len() > MAX_NOTE_MARKDOWN_BYTES {
                return Err(ProtocolError::LimitExceeded("note_markdown"));
            }
        }
        Ok(Self { note_id, kind })
    }

    pub fn note_id(&self) -> &str {
        &self.note_id
    }

    pub fn kind(&self) -> &NoteRevisionKind {
        &self.kind
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TwinEventV1 {
    event_id: Digest32,
    event_json: String,
}

impl fmt::Debug for TwinEventV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TwinEventV1")
            .field("event_id", &self.event_id)
            .field("event_json_len", &self.event_json.len())
            .finish()
    }
}

impl TwinEventV1 {
    pub fn new(event_id: Digest32, event_json: String) -> Result<Self> {
        if event_json.is_empty() {
            return Err(ProtocolError::InvalidField("twin_event_json"));
        }
        if event_json.len() > MAX_TWIN_EVENT_BYTES {
            return Err(ProtocolError::LimitExceeded("twin_event_json"));
        }
        let value: serde_json::Value = serde_json::from_str(&event_json)
            .map_err(|_| ProtocolError::InvalidField("twin_event_json"))?;
        if !value.is_object() {
            return Err(ProtocolError::InvalidField("twin_event_json"));
        }
        Ok(Self {
            event_id,
            event_json,
        })
    }

    pub const fn event_id(&self) -> &Digest32 {
        &self.event_id
    }

    pub fn event_json(&self) -> &str {
        &self.event_json
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AttachmentManifestV1 {
    attachment_digest: Digest32,
    media_type: String,
    decoded_size: u64,
    chunk_size: u32,
    chunk_count: u32,
}

impl fmt::Debug for AttachmentManifestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentManifestV1")
            .field("attachment_digest", &self.attachment_digest)
            .field("media_type_len", &self.media_type.len())
            .field("decoded_size", &self.decoded_size)
            .field("chunk_size", &self.chunk_size)
            .field("chunk_count", &self.chunk_count)
            .finish()
    }
}

impl AttachmentManifestV1 {
    pub fn new(
        attachment_digest: Digest32,
        media_type: impl Into<String>,
        decoded_size: usize,
    ) -> Result<Self> {
        let decoded_size = u64::try_from(decoded_size)
            .map_err(|_| ProtocolError::LimitExceeded("attachment_size"))?;
        Self::from_wire(
            attachment_digest,
            media_type.into(),
            decoded_size,
            ATTACHMENT_CHUNK_BYTES as u32,
            expected_chunk_count(decoded_size)?,
        )
    }

    pub(crate) fn from_wire(
        attachment_digest: Digest32,
        media_type: String,
        decoded_size: u64,
        chunk_size: u32,
        chunk_count: u32,
    ) -> Result<Self> {
        validate_media_type(&media_type)?;
        if decoded_size == 0 {
            return Err(ProtocolError::InvalidField("attachment_size"));
        }
        if decoded_size > MAX_ATTACHMENT_BYTES as u64 {
            return Err(ProtocolError::LimitExceeded("attachment_size"));
        }
        if chunk_size != ATTACHMENT_CHUNK_BYTES as u32 {
            return Err(ProtocolError::InvalidField("attachment_chunk_size"));
        }
        if chunk_count == 0 || chunk_count as usize > MAX_ATTACHMENT_CHUNKS {
            return Err(ProtocolError::LimitExceeded("attachment_chunk_count"));
        }
        if chunk_count != expected_chunk_count(decoded_size)? {
            return Err(ProtocolError::InvalidField("attachment_chunk_count"));
        }
        Ok(Self {
            attachment_digest,
            media_type,
            decoded_size,
            chunk_size,
            chunk_count,
        })
    }

    pub const fn attachment_digest(&self) -> &Digest32 {
        &self.attachment_digest
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub const fn decoded_size(&self) -> u64 {
        self.decoded_size
    }

    pub const fn chunk_size(&self) -> u32 {
        self.chunk_size
    }

    pub const fn chunk_count(&self) -> u32 {
        self.chunk_count
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AttachmentChunkV1 {
    manifest_operation_id: OperationId,
    attachment_digest: Digest32,
    chunk_index: u32,
    chunk_count: u32,
    data: Vec<u8>,
}

impl fmt::Debug for AttachmentChunkV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentChunkV1")
            .field("manifest_operation_id", &self.manifest_operation_id)
            .field("attachment_digest", &self.attachment_digest)
            .field("chunk_index", &self.chunk_index)
            .field("chunk_count", &self.chunk_count)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl AttachmentChunkV1 {
    pub fn new(
        manifest_operation_id: OperationId,
        attachment_digest: Digest32,
        chunk_index: u32,
        chunk_count: u32,
        data: Vec<u8>,
    ) -> Result<Self> {
        if chunk_count == 0 || chunk_count as usize > MAX_ATTACHMENT_CHUNKS {
            return Err(ProtocolError::LimitExceeded("attachment_chunk_count"));
        }
        if chunk_index >= chunk_count {
            return Err(ProtocolError::InvalidField("attachment_chunk_index"));
        }
        let is_final = chunk_index + 1 == chunk_count;
        if data.is_empty()
            || data.len() > ATTACHMENT_CHUNK_BYTES
            || (!is_final && data.len() != ATTACHMENT_CHUNK_BYTES)
        {
            return Err(ProtocolError::InvalidField("attachment_chunk_data"));
        }
        Ok(Self {
            manifest_operation_id,
            attachment_digest,
            chunk_index,
            chunk_count,
            data,
        })
    }

    pub const fn manifest_operation_id(&self) -> &OperationId {
        &self.manifest_operation_id
    }

    pub const fn attachment_digest(&self) -> &Digest32 {
        &self.attachment_digest
    }

    pub const fn chunk_index(&self) -> u32 {
        self.chunk_index
    }

    pub const fn chunk_count(&self) -> u32 {
        self.chunk_count
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum OperationPayloadV1 {
    NoteRevision(NoteRevisionV1),
    TwinEvent(TwinEventV1),
    AttachmentManifest(AttachmentManifestV1),
    AttachmentChunk(AttachmentChunkV1),
}

impl fmt::Debug for OperationPayloadV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoteRevision(revision) => formatter
                .debug_tuple("NoteRevision")
                .field(revision)
                .finish(),
            Self::TwinEvent(event) => formatter.debug_tuple("TwinEvent").field(event).finish(),
            Self::AttachmentManifest(manifest) => formatter
                .debug_tuple("AttachmentManifest")
                .field(manifest)
                .finish(),
            Self::AttachmentChunk(chunk) => formatter
                .debug_tuple("AttachmentChunk")
                .field(chunk)
                .finish(),
        }
    }
}

impl OperationPayloadV1 {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NoteRevision(_) => "note_revision",
            Self::TwinEvent(_) => "twin_event",
            Self::AttachmentManifest(_) => "attachment_manifest",
            Self::AttachmentChunk(_) => "attachment_chunk",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct OperationV1 {
    recorded_at_unix_ms: u64,
    causal_parents: Vec<OperationId>,
    payload: OperationPayloadV1,
}

impl fmt::Debug for OperationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationV1")
            .field("recorded_at_unix_ms", &self.recorded_at_unix_ms)
            .field("causal_parents", &self.causal_parents)
            .field("payload", &self.payload)
            .finish()
    }
}

impl OperationV1 {
    pub fn new(
        recorded_at_unix_ms: u64,
        causal_parents: Vec<OperationId>,
        payload: OperationPayloadV1,
    ) -> Result<Self> {
        if causal_parents.len() > MAX_CAUSAL_PARENTS {
            return Err(ProtocolError::LimitExceeded("causal_parents"));
        }
        if causal_parents.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ProtocolError::InvalidField("causal_parents"));
        }
        if let OperationPayloadV1::AttachmentChunk(chunk) = &payload {
            if causal_parents.as_slice() != [*chunk.manifest_operation_id()] {
                return Err(ProtocolError::InvalidField("attachment_manifest_parent"));
            }
        }
        Ok(Self {
            recorded_at_unix_ms,
            causal_parents,
            payload,
        })
    }

    pub const fn recorded_at_unix_ms(&self) -> u64 {
        self.recorded_at_unix_ms
    }

    pub fn causal_parents(&self) -> &[OperationId] {
        &self.causal_parents
    }

    pub const fn payload(&self) -> &OperationPayloadV1 {
        &self.payload
    }
}

/// A validated version-1 encrypted envelope.
///
/// Untrusted JSON must enter through [`EnvelopeV1::from_json`] or
/// [`EnvelopeV1::from_json_bytes`] so the raw input bound is enforced before
/// parsing. Generic Serde deserialization is intentionally unavailable:
///
/// ```compile_fail
/// use grafyn_sync_protocol::EnvelopeV1;
///
/// let _: EnvelopeV1 = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Eq, PartialEq)]
pub struct EnvelopeV1 {
    vault_id: VaultId,
    device_id: DeviceId,
    device_public_key: DevicePublicKey,
    operation_id: OperationId,
    nonce: [u8; 24],
    ciphertext: Vec<u8>,
    signature: [u8; 64],
}

impl fmt::Debug for EnvelopeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvelopeV1")
            .field("vault_id", &self.vault_id)
            .field("device_id", &self.device_id)
            .field("device_public_key", &self.device_public_key)
            .field("operation_id", &self.operation_id)
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

impl EnvelopeV1 {
    pub fn from_json(json: &str) -> Result<Self> {
        Self::from_json_bytes(json.as_bytes())
    }

    pub fn from_json_bytes(json: &[u8]) -> Result<Self> {
        if json.len() > MAX_ENVELOPE_JSON_BYTES {
            return Err(ProtocolError::JsonTooLarge);
        }
        let wire: EnvelopeWire =
            serde_json::from_slice(json).map_err(|_| ProtocolError::InvalidJson)?;
        wire.try_into()
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|_| ProtocolError::InvalidJson)
    }

    pub const fn protocol(&self) -> &'static str {
        ENVELOPE_PROTOCOL
    }

    pub const fn schema_version(&self) -> u16 {
        ENVELOPE_SCHEMA_VERSION
    }

    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    pub const fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub const fn device_public_key(&self) -> &DevicePublicKey {
        &self.device_public_key
    }

    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub const fn nonce(&self) -> &[u8; 24] {
        &self.nonce
    }

    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    pub const fn signature(&self) -> &[u8; 64] {
        &self.signature
    }

    pub(crate) fn new(
        vault_id: VaultId,
        device_id: DeviceId,
        device_public_key: DevicePublicKey,
        operation_id: OperationId,
        nonce: [u8; 24],
        ciphertext: Vec<u8>,
        signature: [u8; 64],
    ) -> Result<Self> {
        if ciphertext.len() <= 16 {
            return Err(ProtocolError::InvalidField("ciphertext"));
        }
        if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
            return Err(ProtocolError::LimitExceeded("ciphertext"));
        }
        Ok(Self {
            vault_id,
            device_id,
            device_public_key,
            operation_id,
            nonce,
            ciphertext,
            signature,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeWire {
    protocol: String,
    schema_version: u16,
    vault_id: String,
    device_id: String,
    device_public_key: String,
    operation_id: String,
    nonce: String,
    ciphertext: String,
    signature: String,
}

impl TryFrom<EnvelopeWire> for EnvelopeV1 {
    type Error = ProtocolError;

    fn try_from(wire: EnvelopeWire) -> Result<Self> {
        if wire.protocol != ENVELOPE_PROTOCOL {
            return Err(ProtocolError::UnsupportedProtocol);
        }
        if wire.schema_version != ENVELOPE_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedSchemaVersion);
        }
        let vault_id = VaultId::parse_str(&wire.vault_id)?;
        let device_id = DeviceId::parse_str(&wire.device_id)?;
        let device_public_key = DevicePublicKey::from_bytes(decode_base64_exact::<32>(
            &wire.device_public_key,
            "device_public_key",
        )?);
        let operation_id = OperationId::parse_hex(&wire.operation_id)?;
        let nonce = decode_base64_exact::<24>(&wire.nonce, "nonce")?;
        let ciphertext =
            decode_base64_bounded(&wire.ciphertext, "ciphertext", 17, MAX_CIPHERTEXT_BYTES)?;
        let signature = decode_base64_exact::<64>(&wire.signature, "signature")?;
        Self::new(
            vault_id,
            device_id,
            device_public_key,
            operation_id,
            nonce,
            ciphertext,
            signature,
        )
    }
}

impl From<&EnvelopeV1> for EnvelopeWire {
    fn from(envelope: &EnvelopeV1) -> Self {
        Self {
            protocol: ENVELOPE_PROTOCOL.to_owned(),
            schema_version: ENVELOPE_SCHEMA_VERSION,
            vault_id: envelope.vault_id.to_string(),
            device_id: envelope.device_id.to_string(),
            device_public_key: URL_SAFE_NO_PAD.encode(envelope.device_public_key.as_bytes()),
            operation_id: envelope.operation_id.to_string(),
            nonce: URL_SAFE_NO_PAD.encode(envelope.nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(&envelope.ciphertext),
            signature: URL_SAFE_NO_PAD.encode(envelope.signature),
        }
    }
}

impl Serialize for EnvelopeV1 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        EnvelopeWire::from(self).serialize(serializer)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct VerifiedOperation {
    vault_id: VaultId,
    device_id: DeviceId,
    device_public_key: DevicePublicKey,
    operation_id: OperationId,
    operation: OperationV1,
}

impl fmt::Debug for VerifiedOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedOperation")
            .field("vault_id", &self.vault_id)
            .field("device_id", &self.device_id)
            .field("device_public_key", &self.device_public_key)
            .field("operation_id", &self.operation_id)
            .field("operation", &self.operation)
            .finish()
    }
}

impl VerifiedOperation {
    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    pub const fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub const fn device_public_key(&self) -> &DevicePublicKey {
        &self.device_public_key
    }

    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub const fn operation(&self) -> &OperationV1 {
        &self.operation
    }

    pub(crate) const fn new(
        vault_id: VaultId,
        device_id: DeviceId,
        device_public_key: DevicePublicKey,
        operation_id: OperationId,
        operation: OperationV1,
    ) -> Self {
        Self {
            vault_id,
            device_id,
            device_public_key,
            operation_id,
            operation,
        }
    }
}

pub fn reassemble_attachment(
    manifest_operation: &VerifiedOperation,
    chunk_operations: &[VerifiedOperation],
) -> Result<Vec<u8>> {
    let OperationPayloadV1::AttachmentManifest(manifest) = manifest_operation.operation().payload()
    else {
        return Err(ProtocolError::InvalidField("attachment_manifest"));
    };
    let expected_count = manifest.chunk_count() as usize;
    if chunk_operations.len() != expected_count {
        return Err(ProtocolError::IncompleteAttachment);
    }

    let mut chunks: Vec<Option<&AttachmentChunkV1>> = vec![None; expected_count];
    let mut decoded_size = 0usize;
    for operation in chunk_operations {
        let OperationPayloadV1::AttachmentChunk(chunk) = operation.operation().payload() else {
            return Err(ProtocolError::InvalidField("attachment_chunk"));
        };
        if operation.vault_id() != manifest_operation.vault_id()
            || chunk.manifest_operation_id() != manifest_operation.operation_id()
            || chunk.attachment_digest() != manifest.attachment_digest()
            || chunk.chunk_count() != manifest.chunk_count()
            || operation.operation().causal_parents() != [*manifest_operation.operation_id()]
        {
            return Err(ProtocolError::InvalidField("attachment_chunk_manifest"));
        }
        let index = chunk.chunk_index() as usize;
        let slot = chunks
            .get_mut(index)
            .ok_or(ProtocolError::InvalidField("attachment_chunk_index"))?;
        if slot.is_some() {
            return Err(ProtocolError::DuplicateChunk);
        }
        decoded_size = decoded_size
            .checked_add(chunk.data().len())
            .ok_or(ProtocolError::LimitExceeded("attachment_size"))?;
        *slot = Some(chunk);
    }
    if decoded_size as u64 != manifest.decoded_size() || chunks.iter().any(Option::is_none) {
        return Err(ProtocolError::IncompleteAttachment);
    }

    let mut hasher = Sha256::new();
    for chunk in &chunks {
        hasher.update(chunk.expect("completeness checked").data());
    }
    let actual_digest = Digest32::from_bytes(hasher.finalize().into());
    if &actual_digest != manifest.attachment_digest() {
        return Err(ProtocolError::AttachmentDigestMismatch);
    }

    let mut bytes = Vec::with_capacity(decoded_size);
    for chunk in chunks {
        bytes.extend_from_slice(chunk.expect("completeness checked").data());
    }
    Ok(bytes)
}

fn validate_identifier(value: &str, max_bytes: usize, field: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(())
}

fn validate_media_type(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_MEDIA_TYPE_BYTES
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
        || value.matches('/').count() != 1
        || value.starts_with('/')
        || value.ends_with('/')
    {
        return Err(ProtocolError::InvalidField("media_type"));
    }
    Ok(())
}

fn expected_chunk_count(decoded_size: u64) -> Result<u32> {
    if decoded_size == 0 {
        return Err(ProtocolError::InvalidField("attachment_size"));
    }
    let chunk_size = ATTACHMENT_CHUNK_BYTES as u64;
    let count = decoded_size
        .checked_add(chunk_size - 1)
        .ok_or(ProtocolError::LimitExceeded("attachment_size"))?
        / chunk_size;
    u32::try_from(count).map_err(|_| ProtocolError::LimitExceeded("attachment_chunk_count"))
}

fn decode_base64_exact<const N: usize>(value: &str, field: &'static str) -> Result<[u8; N]> {
    let bytes = decode_base64_bounded(value, field, N, N)?;
    bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidField(field))
}

fn decode_base64_bounded(
    value: &str,
    field: &'static str,
    min_len: usize,
    max_len: usize,
) -> Result<Vec<u8>> {
    let encoded_max = max_len
        .checked_add(2)
        .and_then(|length| length.checked_mul(4))
        .map(|length| length / 3)
        .ok_or(ProtocolError::LimitExceeded(field))?;
    if value.len() > encoded_max {
        return Err(ProtocolError::LimitExceeded(field));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::InvalidField(field))?;
    if bytes.len() < min_len || bytes.len() > max_len || URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(bytes)
}

fn parse_uuid(value: &str) -> Option<[u8; 16]> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
    {
        return None;
    }
    let mut decoded = [0u8; 16];
    let mut source = 0usize;
    for target in &mut decoded {
        while matches!(source, 8 | 13 | 18 | 23) {
            source += 1;
        }
        let high = lower_hex_value(*bytes.get(source)?)?;
        let low = lower_hex_value(*bytes.get(source + 1)?)?;
        *target = high << 4 | low;
        source += 2;
    }
    if decoded == [0; 16] {
        return None;
    }
    Some(decoded)
}

fn parse_hex_32(value: &str) -> Option<[u8; 32]> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut decoded = [0u8; 32];
    for (index, target) in decoded.iter_mut().enumerate() {
        let high = lower_hex_value(bytes[index * 2])?;
        let low = lower_hex_value(bytes[index * 2 + 1])?;
        *target = high << 4 | low;
    }
    Some(decoded)
}

const fn lower_hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn write_uuid(formatter: &mut fmt::Formatter<'_>, bytes: &[u8; 16]) -> fmt::Result {
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            formatter.write_str("-")?;
        }
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

fn write_lower_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}
