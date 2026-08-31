#![forbid(unsafe_code)]

mod canonical;
mod crypto;
mod types;

use std::fmt;

pub use crypto::{open_operation, seal_operation, DeviceSigningKey, TrustedDevice, VaultRootKey};
pub use types::{
    reassemble_attachment, AttachmentChunkV1, AttachmentManifestV1, DeviceId, DevicePublicKey,
    Digest32, EnvelopeV1, NoteRevisionKind, NoteRevisionV1, OperationId, OperationPayloadV1,
    OperationV1, TwinEventV1, VaultId, VerifiedOperation,
};

pub const ENVELOPE_PROTOCOL: &str = "grafyn.sync.envelope";
pub const ENVELOPE_SCHEMA_VERSION: u16 = 1;
pub const MAX_ENVELOPE_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLAINTEXT_BYTES: usize = 1_100_000;
pub const MAX_CIPHERTEXT_BYTES: usize = MAX_PLAINTEXT_BYTES + 16;
pub const MAX_CAUSAL_PARENTS: usize = 64;
pub const MAX_NOTE_ID_BYTES: usize = 256;
pub const MAX_NOTE_MARKDOWN_BYTES: usize = 1024 * 1024;
pub const MAX_TWIN_EVENT_BYTES: usize = 256 * 1024;
pub const MAX_ATTACHMENT_BYTES: usize = 24 * 1024 * 1024;
pub const ATTACHMENT_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_ATTACHMENT_CHUNKS: usize = 96;
pub const MAX_MEDIA_TYPE_BYTES: usize = 127;

pub type Result<T> = std::result::Result<T, ProtocolError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    JsonTooLarge,
    InvalidJson,
    UnsupportedProtocol,
    UnsupportedSchemaVersion,
    InvalidField(&'static str),
    LimitExceeded(&'static str),
    IdentityMismatch(&'static str),
    InvalidSignature,
    AuthenticationFailed,
    OperationIdMismatch,
    InvalidCanonicalEncoding,
    IncompleteAttachment,
    DuplicateChunk,
    AttachmentDigestMismatch,
    RandomnessUnavailable,
    KeyDerivationFailed,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JsonTooLarge => formatter.write_str("envelope JSON exceeds the protocol limit"),
            Self::InvalidJson => formatter.write_str("invalid envelope JSON"),
            Self::UnsupportedProtocol => formatter.write_str("unsupported envelope protocol"),
            Self::UnsupportedSchemaVersion => {
                formatter.write_str("unsupported envelope schema version")
            }
            Self::InvalidField(field) => write!(formatter, "invalid protocol field: {field}"),
            Self::LimitExceeded(field) => write!(formatter, "protocol limit exceeded: {field}"),
            Self::IdentityMismatch(field) => write!(formatter, "identity mismatch: {field}"),
            Self::InvalidSignature => formatter.write_str("invalid envelope signature"),
            Self::AuthenticationFailed => formatter.write_str("ciphertext authentication failed"),
            Self::OperationIdMismatch => formatter.write_str("canonical operation ID mismatch"),
            Self::InvalidCanonicalEncoding => {
                formatter.write_str("invalid canonical operation encoding")
            }
            Self::IncompleteAttachment => formatter.write_str("attachment chunks are incomplete"),
            Self::DuplicateChunk => formatter.write_str("duplicate attachment chunk"),
            Self::AttachmentDigestMismatch => {
                formatter.write_str("attachment digest does not match the manifest")
            }
            Self::RandomnessUnavailable => formatter.write_str("secure randomness is unavailable"),
            Self::KeyDerivationFailed => formatter.write_str("protocol key derivation failed"),
        }
    }
}

impl std::error::Error for ProtocolError {}
