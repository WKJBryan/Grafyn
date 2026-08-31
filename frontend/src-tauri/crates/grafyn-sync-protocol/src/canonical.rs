use std::str;

use crate::{
    AttachmentChunkV1, AttachmentManifestV1, DeviceId, DevicePublicKey, Digest32, NoteRevisionKind,
    NoteRevisionV1, OperationId, OperationPayloadV1, OperationV1, ProtocolError, Result,
    TwinEventV1, VaultId, ENVELOPE_PROTOCOL, ENVELOPE_SCHEMA_VERSION, MAX_PLAINTEXT_BYTES,
};

pub(crate) const HKDF_SALT: &[u8] = b"grafyn.sync.hkdf.extract.v1";

const OPERATION_DOMAIN: &str = "grafyn.sync.operation.v1";
const NOTE_REVISION_DOMAIN: &str = "grafyn.sync.payload.note-revision.v1";
const TWIN_EVENT_DOMAIN: &str = "grafyn.sync.payload.twin-event.v1";
const ATTACHMENT_MANIFEST_DOMAIN: &str = "grafyn.sync.payload.attachment-manifest.v1";
const ATTACHMENT_CHUNK_DOMAIN: &str = "grafyn.sync.payload.attachment-chunk.v1";
const OPERATION_ID_DOMAIN: &str = "grafyn.sync.operation-id.v1";
const ENVELOPE_AAD_DOMAIN: &str = "grafyn.sync.envelope.aad.v1";
const ENVELOPE_SIGNATURE_DOMAIN: &str = "grafyn.sync.envelope.signature.v1";
const HKDF_INFO_DOMAIN: &str = "grafyn.sync.hkdf.info.v1";

pub(crate) fn encode_operation(operation: &OperationV1) -> Result<Vec<u8>> {
    encode_operation_with_type(operation, operation.payload().kind())
}

#[cfg(test)]
pub(crate) fn encode_operation_with_claimed_type(
    operation: &OperationV1,
    claimed_type: &str,
) -> Result<Vec<u8>> {
    encode_operation_with_type(operation, claimed_type)
}

fn encode_operation_with_type(operation: &OperationV1, payload_type: &str) -> Result<Vec<u8>> {
    let payload = encode_payload(operation.payload())?;
    let bytes = document(
        OPERATION_DOMAIN,
        &[
            (
                "recorded_at_unix_ms",
                operation.recorded_at_unix_ms().to_be_bytes().to_vec(),
            ),
            ("causal_parents", encode_id_list(operation.causal_parents())),
            ("payload_type", payload_type.as_bytes().to_vec()),
            ("payload", payload),
        ],
    )?;
    if bytes.len() > MAX_PLAINTEXT_BYTES {
        return Err(ProtocolError::LimitExceeded("operation_plaintext"));
    }
    Ok(bytes)
}

pub(crate) fn decode_operation(bytes: &[u8]) -> Result<OperationV1> {
    if bytes.len() > MAX_PLAINTEXT_BYTES {
        return Err(ProtocolError::LimitExceeded("operation_plaintext"));
    }
    let mut decoder = Decoder::new(bytes);
    decoder.expect_domain(OPERATION_DOMAIN)?;
    let recorded_at_unix_ms = decode_u64(decoder.field("recorded_at_unix_ms")?)?;
    let causal_parents = decode_id_list(decoder.field("causal_parents")?)?;
    let payload_type = decode_utf8(decoder.field("payload_type")?)?.to_owned();
    let payload_bytes = decoder.field("payload")?.to_vec();
    decoder.finish()?;

    let payload = decode_payload(&payload_type, &payload_bytes)?;
    let operation = OperationV1::new(recorded_at_unix_ms, causal_parents, payload)?;
    if encode_operation(&operation)? != bytes {
        return Err(ProtocolError::InvalidCanonicalEncoding);
    }
    Ok(operation)
}

pub(crate) fn operation_id_message(
    vault_id: &VaultId,
    device_id: &DeviceId,
    device_public_key: &DevicePublicKey,
    operation_bytes: &[u8],
) -> Result<Vec<u8>> {
    document(
        OPERATION_ID_DOMAIN,
        &[
            ("protocol", ENVELOPE_PROTOCOL.as_bytes().to_vec()),
            (
                "schema_version",
                ENVELOPE_SCHEMA_VERSION.to_be_bytes().to_vec(),
            ),
            ("vault_id", vault_id.as_bytes().to_vec()),
            ("device_id", device_id.as_bytes().to_vec()),
            ("device_public_key", device_public_key.as_bytes().to_vec()),
            ("operation", operation_bytes.to_vec()),
        ],
    )
}

pub(crate) fn envelope_aad(
    vault_id: &VaultId,
    device_id: &DeviceId,
    device_public_key: &DevicePublicKey,
    operation_id: &OperationId,
    nonce: &[u8; 24],
) -> Result<Vec<u8>> {
    document(
        ENVELOPE_AAD_DOMAIN,
        &[
            ("protocol", ENVELOPE_PROTOCOL.as_bytes().to_vec()),
            (
                "schema_version",
                ENVELOPE_SCHEMA_VERSION.to_be_bytes().to_vec(),
            ),
            ("vault_id", vault_id.as_bytes().to_vec()),
            ("device_id", device_id.as_bytes().to_vec()),
            ("device_public_key", device_public_key.as_bytes().to_vec()),
            ("operation_id", operation_id.as_bytes().to_vec()),
            ("nonce", nonce.to_vec()),
        ],
    )
}

pub(crate) fn signature_message(aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    document(
        ENVELOPE_SIGNATURE_DOMAIN,
        &[("aad", aad.to_vec()), ("ciphertext", ciphertext.to_vec())],
    )
}

pub(crate) fn hkdf_info(
    purpose: &str,
    vault_id: &VaultId,
    device_id: &DeviceId,
    device_public_key: &DevicePublicKey,
) -> Result<Vec<u8>> {
    document(
        HKDF_INFO_DOMAIN,
        &[
            ("purpose", purpose.as_bytes().to_vec()),
            ("vault_id", vault_id.as_bytes().to_vec()),
            ("device_id", device_id.as_bytes().to_vec()),
            ("device_public_key", device_public_key.as_bytes().to_vec()),
        ],
    )
}

fn encode_payload(payload: &OperationPayloadV1) -> Result<Vec<u8>> {
    match payload {
        OperationPayloadV1::NoteRevision(revision) => {
            let (kind, markdown) = match revision.kind() {
                NoteRevisionKind::Put { markdown } => ("put", markdown.as_bytes().to_vec()),
                NoteRevisionKind::Tombstone => ("tombstone", Vec::new()),
            };
            document(
                NOTE_REVISION_DOMAIN,
                &[
                    ("note_id", revision.note_id().as_bytes().to_vec()),
                    ("revision_kind", kind.as_bytes().to_vec()),
                    ("markdown", markdown),
                ],
            )
        }
        OperationPayloadV1::TwinEvent(event) => document(
            TWIN_EVENT_DOMAIN,
            &[
                ("event_id", event.event_id().as_bytes().to_vec()),
                ("event_json", event.event_json().as_bytes().to_vec()),
            ],
        ),
        OperationPayloadV1::AttachmentManifest(manifest) => document(
            ATTACHMENT_MANIFEST_DOMAIN,
            &[
                (
                    "attachment_digest",
                    manifest.attachment_digest().as_bytes().to_vec(),
                ),
                ("media_type", manifest.media_type().as_bytes().to_vec()),
                (
                    "decoded_size",
                    manifest.decoded_size().to_be_bytes().to_vec(),
                ),
                ("chunk_size", manifest.chunk_size().to_be_bytes().to_vec()),
                ("chunk_count", manifest.chunk_count().to_be_bytes().to_vec()),
            ],
        ),
        OperationPayloadV1::AttachmentChunk(chunk) => document(
            ATTACHMENT_CHUNK_DOMAIN,
            &[
                (
                    "manifest_operation_id",
                    chunk.manifest_operation_id().as_bytes().to_vec(),
                ),
                (
                    "attachment_digest",
                    chunk.attachment_digest().as_bytes().to_vec(),
                ),
                ("chunk_index", chunk.chunk_index().to_be_bytes().to_vec()),
                ("chunk_count", chunk.chunk_count().to_be_bytes().to_vec()),
                ("data", chunk.data().to_vec()),
            ],
        ),
    }
}

fn decode_payload(payload_type: &str, bytes: &[u8]) -> Result<OperationPayloadV1> {
    match payload_type {
        "note_revision" => decode_note_revision(bytes).map(OperationPayloadV1::NoteRevision),
        "twin_event" => decode_twin_event(bytes).map(OperationPayloadV1::TwinEvent),
        "attachment_manifest" => {
            decode_attachment_manifest(bytes).map(OperationPayloadV1::AttachmentManifest)
        }
        "attachment_chunk" => {
            decode_attachment_chunk(bytes).map(OperationPayloadV1::AttachmentChunk)
        }
        _ => Err(ProtocolError::InvalidField("payload_type")),
    }
}

fn decode_note_revision(bytes: &[u8]) -> Result<NoteRevisionV1> {
    let mut decoder = Decoder::new(bytes);
    decoder.expect_domain(NOTE_REVISION_DOMAIN)?;
    let note_id = decode_utf8(decoder.field("note_id")?)?.to_owned();
    let revision_kind = decode_utf8(decoder.field("revision_kind")?)?.to_owned();
    let markdown_bytes = decoder.field("markdown")?.to_vec();
    decoder.finish()?;
    match revision_kind.as_str() {
        "put" => NoteRevisionV1::put(
            note_id,
            String::from_utf8(markdown_bytes)
                .map_err(|_| ProtocolError::InvalidField("note_markdown"))?,
        ),
        "tombstone" if markdown_bytes.is_empty() => NoteRevisionV1::tombstone(note_id),
        "tombstone" => Err(ProtocolError::InvalidField("note_markdown")),
        _ => Err(ProtocolError::InvalidField("note_revision_kind")),
    }
}

fn decode_twin_event(bytes: &[u8]) -> Result<TwinEventV1> {
    let mut decoder = Decoder::new(bytes);
    decoder.expect_domain(TWIN_EVENT_DOMAIN)?;
    let event_id = Digest32::from_bytes(decode_array::<32>(decoder.field("event_id")?)?);
    let event_json = decode_utf8(decoder.field("event_json")?)?.to_owned();
    decoder.finish()?;
    TwinEventV1::new(event_id, event_json)
}

fn decode_attachment_manifest(bytes: &[u8]) -> Result<AttachmentManifestV1> {
    let mut decoder = Decoder::new(bytes);
    decoder.expect_domain(ATTACHMENT_MANIFEST_DOMAIN)?;
    let digest = Digest32::from_bytes(decode_array::<32>(decoder.field("attachment_digest")?)?);
    let media_type = decode_utf8(decoder.field("media_type")?)?.to_owned();
    let decoded_size = decode_u64(decoder.field("decoded_size")?)?;
    let chunk_size = decode_u32(decoder.field("chunk_size")?)?;
    let chunk_count = decode_u32(decoder.field("chunk_count")?)?;
    decoder.finish()?;
    AttachmentManifestV1::from_wire(digest, media_type, decoded_size, chunk_size, chunk_count)
}

fn decode_attachment_chunk(bytes: &[u8]) -> Result<AttachmentChunkV1> {
    let mut decoder = Decoder::new(bytes);
    decoder.expect_domain(ATTACHMENT_CHUNK_DOMAIN)?;
    let manifest_operation_id =
        OperationId::from_bytes(decode_array::<32>(decoder.field("manifest_operation_id")?)?);
    let digest = Digest32::from_bytes(decode_array::<32>(decoder.field("attachment_digest")?)?);
    let chunk_index = decode_u32(decoder.field("chunk_index")?)?;
    let chunk_count = decode_u32(decoder.field("chunk_count")?)?;
    let data = decoder.field("data")?.to_vec();
    decoder.finish()?;
    AttachmentChunkV1::new(
        manifest_operation_id,
        digest,
        chunk_index,
        chunk_count,
        data,
    )
}

fn document(domain: &str, fields: &[(&str, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_frame(&mut bytes, domain.as_bytes())?;
    for (name, value) in fields {
        encode_frame(&mut bytes, name.as_bytes())?;
        encode_frame(&mut bytes, value)?;
    }
    Ok(bytes)
}

fn encode_frame(target: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length =
        u64::try_from(value.len()).map_err(|_| ProtocolError::LimitExceeded("canonical_frame"))?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(value);
    Ok(())
}

fn encode_id_list(ids: &[OperationId]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + ids.len() * 40);
    bytes.extend_from_slice(&(ids.len() as u32).to_be_bytes());
    for id in ids {
        bytes.extend_from_slice(&32u64.to_be_bytes());
        bytes.extend_from_slice(id.as_bytes());
    }
    bytes
}

fn decode_id_list(bytes: &[u8]) -> Result<Vec<OperationId>> {
    if bytes.len() < 4 {
        return Err(ProtocolError::InvalidCanonicalEncoding);
    }
    let count = u32::from_be_bytes(
        bytes[..4]
            .try_into()
            .map_err(|_| ProtocolError::InvalidCanonicalEncoding)?,
    ) as usize;
    if count > crate::MAX_CAUSAL_PARENTS {
        return Err(ProtocolError::LimitExceeded("causal_parents"));
    }
    let mut decoder = Decoder::new(&bytes[4..]);
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(OperationId::from_bytes(decode_array::<32>(
            decoder.frame()?,
        )?));
    }
    decoder.finish()?;
    Ok(ids)
}

fn decode_u64(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(decode_array(bytes)?))
}

fn decode_u32(bytes: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(decode_array(bytes)?))
}

fn decode_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidCanonicalEncoding)
}

fn decode_utf8(bytes: &[u8]) -> Result<&str> {
    str::from_utf8(bytes).map_err(|_| ProtocolError::InvalidCanonicalEncoding)
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn expect_domain(&mut self, expected: &str) -> Result<()> {
        if self.frame()? != expected.as_bytes() {
            return Err(ProtocolError::InvalidCanonicalEncoding);
        }
        Ok(())
    }

    fn field(&mut self, expected_name: &str) -> Result<&'a [u8]> {
        if self.frame()? != expected_name.as_bytes() {
            return Err(ProtocolError::InvalidCanonicalEncoding);
        }
        self.frame()
    }

    fn frame(&mut self) -> Result<&'a [u8]> {
        let length_end = self
            .offset
            .checked_add(8)
            .ok_or(ProtocolError::InvalidCanonicalEncoding)?;
        let length_bytes = self
            .bytes
            .get(self.offset..length_end)
            .ok_or(ProtocolError::InvalidCanonicalEncoding)?;
        let length = usize::try_from(u64::from_be_bytes(
            length_bytes
                .try_into()
                .map_err(|_| ProtocolError::InvalidCanonicalEncoding)?,
        ))
        .map_err(|_| ProtocolError::InvalidCanonicalEncoding)?;
        let value_end = length_end
            .checked_add(length)
            .ok_or(ProtocolError::InvalidCanonicalEncoding)?;
        let value = self
            .bytes
            .get(length_end..value_end)
            .ok_or(ProtocolError::InvalidCanonicalEncoding)?;
        self.offset = value_end;
        Ok(value)
    }

    fn finish(self) -> Result<()> {
        if self.offset != self.bytes.len() {
            return Err(ProtocolError::InvalidCanonicalEncoding);
        }
        Ok(())
    }
}
