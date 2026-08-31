use crate::models::attachment::{
    sha256_attachment_digest, AttachmentIngestStatus, AttachmentManifestRecordV1,
};
use crate::services::twin_events::{AnchoredRoot, MutationError};
use grafyn_sync_protocol::{
    AttachmentChunkV1, AttachmentManifestV1, Digest32, OperationId, ATTACHMENT_CHUNK_BYTES,
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_CHUNKS,
};
use std::path::Path;

const ATTACHMENTS_ROOT: &str = "attachments/v1";
const MANIFESTS_ROOT: &str = "attachments/v1/manifests";
const CHUNKS_ROOT: &str = "attachments/v1/chunks";
const BLOBS_ROOT: &str = "attachments/v1/blobs";
const STAGING_ROOT: &str = "attachments/v1/install-staging";
const MANIFEST_RECORD_LIMIT: usize = 1024;

#[derive(Debug)]
pub(crate) struct AttachmentStore {
    root: AnchoredRoot,
}

impl AttachmentStore {
    pub(crate) fn open(root_path: impl AsRef<Path>) -> Result<Self, MutationError> {
        let root = AnchoredRoot::open(root_path)?;
        for directory in [
            ATTACHMENTS_ROOT,
            MANIFESTS_ROOT,
            CHUNKS_ROOT,
            BLOBS_ROOT,
            STAGING_ROOT,
        ] {
            root.open_directory(directory, true)?;
        }
        Ok(Self { root })
    }

    pub(crate) fn ingest_manifest(
        &self,
        manifest_operation_id: OperationId,
        manifest: &AttachmentManifestV1,
    ) -> Result<AttachmentIngestStatus, MutationError> {
        validate_manifest(manifest)?;
        let record = AttachmentManifestRecordV1::from_protocol(manifest_operation_id, manifest);
        let bytes = serde_json::to_vec(&record).map_err(|error| {
            MutationError::Invalid(format!("failed to encode attachment manifest: {error}"))
        })?;
        if bytes.len() > MANIFEST_RECORD_LIMIT {
            return Err(MutationError::Invalid(
                "attachment manifest record exceeds its storage limit".into(),
            ));
        }
        self.install_immutable(
            &manifest_key(&manifest_operation_id),
            &bytes,
            MANIFEST_RECORD_LIMIT,
            "attachment manifest",
        )?;
        self.try_materialize(manifest_operation_id, manifest)
    }

    pub(crate) fn ingest_chunk(
        &self,
        chunk: &AttachmentChunkV1,
    ) -> Result<AttachmentIngestStatus, MutationError> {
        let manifest_operation_id = *chunk.manifest_operation_id();
        let manifest = self.load_manifest(&manifest_operation_id)?;
        validate_chunk(chunk, &manifest)?;

        if let Some(bytes) = self.materialized_bytes(manifest.attachment_digest())? {
            validate_materialized_size(&bytes, &manifest)?;
            let (start, end) = chunk_range(chunk, &manifest)?;
            if bytes.get(start..end) != Some(chunk.data()) {
                return Err(MutationError::RecoveryConflict(
                    "attachment chunk collides with the materialized blob".into(),
                ));
            }
            return Ok(complete_status(&manifest));
        }

        self.install_immutable(
            &chunk_key(&manifest_operation_id, chunk.chunk_index()),
            chunk.data(),
            expected_chunk_len(&manifest, chunk.chunk_index())?,
            "attachment chunk",
        )?;
        self.try_materialize(manifest_operation_id, &manifest)
    }

    pub(crate) fn materialized_bytes(
        &self,
        attachment_digest: &Digest32,
    ) -> Result<Option<Vec<u8>>, MutationError> {
        let Some(bytes) = self
            .root
            .read_bounded(&blob_key(attachment_digest), MAX_ATTACHMENT_BYTES)?
        else {
            return Ok(None);
        };
        if sha256_attachment_digest(&bytes) != *attachment_digest {
            return Err(MutationError::RecoveryConflict(
                "materialized attachment digest collision or corruption".into(),
            ));
        }
        Ok(Some(bytes))
    }

    fn load_manifest(
        &self,
        manifest_operation_id: &OperationId,
    ) -> Result<AttachmentManifestV1, MutationError> {
        let bytes = self
            .root
            .read_bounded(&manifest_key(manifest_operation_id), MANIFEST_RECORD_LIMIT)?
            .ok_or_else(|| {
                MutationError::Invalid("attachment chunk references an unavailable manifest".into())
            })?;
        let record: AttachmentManifestRecordV1 =
            serde_json::from_slice(&bytes).map_err(|error| {
                MutationError::Invalid(format!("invalid attachment manifest record: {error}"))
            })?;
        let (recorded_operation_id, manifest) = record.to_protocol().map_err(|error| {
            MutationError::Invalid(format!("invalid attachment manifest record: {error}"))
        })?;
        if recorded_operation_id != *manifest_operation_id {
            return Err(MutationError::RecoveryConflict(
                "attachment manifest operation ID does not match its content-addressed key".into(),
            ));
        }
        Ok(manifest)
    }

    fn try_materialize(
        &self,
        manifest_operation_id: OperationId,
        manifest: &AttachmentManifestV1,
    ) -> Result<AttachmentIngestStatus, MutationError> {
        if let Some(bytes) = self.materialized_bytes(manifest.attachment_digest())? {
            validate_materialized_size(&bytes, manifest)?;
            return Ok(complete_status(manifest));
        }

        let chunk_count = manifest.chunk_count();
        let mut received_chunks = 0u32;
        let mut chunks = Vec::with_capacity(chunk_count as usize);
        for chunk_index in 0..chunk_count {
            let expected_len = expected_chunk_len(manifest, chunk_index)?;
            match self.root.read_bounded(
                &chunk_key(&manifest_operation_id, chunk_index),
                expected_len,
            )? {
                Some(bytes) if bytes.len() == expected_len => {
                    received_chunks += 1;
                    chunks.push(Some(bytes));
                }
                Some(_) => {
                    return Err(MutationError::Invalid(format!(
                        "attachment chunk {chunk_index} has the wrong exact size"
                    )))
                }
                None => chunks.push(None),
            }
        }
        if received_chunks != chunk_count {
            return Ok(AttachmentIngestStatus::Incomplete {
                received_chunks,
                chunk_count,
            });
        }

        let decoded_size = usize::try_from(manifest.decoded_size()).map_err(|_| {
            MutationError::Invalid("attachment decoded size cannot fit in memory".into())
        })?;
        let mut bytes = Vec::with_capacity(decoded_size);
        for chunk in chunks {
            bytes.extend_from_slice(chunk.as_deref().expect("completeness checked"));
        }
        validate_materialized_size(&bytes, manifest)?;
        if sha256_attachment_digest(&bytes) != *manifest.attachment_digest() {
            return Err(MutationError::Invalid(
                "attachment full digest does not match its manifest".into(),
            ));
        }

        self.install_immutable(
            &blob_key(manifest.attachment_digest()),
            &bytes,
            MAX_ATTACHMENT_BYTES,
            "materialized attachment blob",
        )?;
        let published = self
            .materialized_bytes(manifest.attachment_digest())?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "materialized attachment disappeared after atomic publication".into(),
                )
            })?;
        if published != bytes {
            return Err(MutationError::RecoveryConflict(
                "materialized attachment blob collision".into(),
            ));
        }
        Ok(complete_status(manifest))
    }

    fn install_immutable(
        &self,
        key: &str,
        bytes: &[u8],
        read_limit: usize,
        label: &str,
    ) -> Result<(), MutationError> {
        self.root.install_no_clobber(key, STAGING_ROOT, bytes)?;
        let stored = self.root.read_bounded(key, read_limit)?.ok_or_else(|| {
            MutationError::RecoveryConflict(format!(
                "{label} disappeared after no-clobber installation"
            ))
        })?;
        if stored != bytes {
            return Err(MutationError::RecoveryConflict(format!(
                "{label} collision at its immutable key"
            )));
        }
        Ok(())
    }
}

fn validate_manifest(manifest: &AttachmentManifestV1) -> Result<(), MutationError> {
    if manifest.chunk_size() != ATTACHMENT_CHUNK_BYTES as u32
        || manifest.chunk_count() == 0
        || manifest.chunk_count() as usize > MAX_ATTACHMENT_CHUNKS
        || manifest.decoded_size() == 0
        || manifest.decoded_size() > MAX_ATTACHMENT_BYTES as u64
    {
        return Err(MutationError::Invalid(
            "attachment manifest violates storage bounds".into(),
        ));
    }
    let expected_count = manifest
        .decoded_size()
        .div_ceil(ATTACHMENT_CHUNK_BYTES as u64);
    if expected_count != manifest.chunk_count() as u64 {
        return Err(MutationError::Invalid(
            "attachment manifest has an invalid chunk count".into(),
        ));
    }
    Ok(())
}

fn validate_chunk(
    chunk: &AttachmentChunkV1,
    manifest: &AttachmentManifestV1,
) -> Result<(), MutationError> {
    if chunk.attachment_digest() != manifest.attachment_digest()
        || chunk.chunk_count() != manifest.chunk_count()
        || chunk.chunk_index() >= manifest.chunk_count()
    {
        return Err(MutationError::Invalid(
            "attachment chunk metadata does not match its manifest".into(),
        ));
    }
    let expected_len = expected_chunk_len(manifest, chunk.chunk_index())?;
    if chunk.data().len() != expected_len {
        return Err(MutationError::Invalid(
            "attachment chunk has the wrong exact size".into(),
        ));
    }
    Ok(())
}

fn expected_chunk_len(
    manifest: &AttachmentManifestV1,
    chunk_index: u32,
) -> Result<usize, MutationError> {
    validate_manifest(manifest)?;
    if chunk_index >= manifest.chunk_count() {
        return Err(MutationError::Invalid(
            "attachment chunk index is out of bounds".into(),
        ));
    }
    if chunk_index + 1 < manifest.chunk_count() {
        return Ok(ATTACHMENT_CHUNK_BYTES);
    }
    let preceding = u64::from(chunk_index) * ATTACHMENT_CHUNK_BYTES as u64;
    usize::try_from(manifest.decoded_size() - preceding)
        .map_err(|_| MutationError::Invalid("attachment chunk size is invalid".into()))
}

fn chunk_range(
    chunk: &AttachmentChunkV1,
    manifest: &AttachmentManifestV1,
) -> Result<(usize, usize), MutationError> {
    let start = usize::try_from(chunk.chunk_index())
        .ok()
        .and_then(|index| index.checked_mul(ATTACHMENT_CHUNK_BYTES))
        .ok_or_else(|| MutationError::Invalid("attachment chunk offset overflow".into()))?;
    let end = start
        .checked_add(expected_chunk_len(manifest, chunk.chunk_index())?)
        .ok_or_else(|| MutationError::Invalid("attachment chunk range overflow".into()))?;
    Ok((start, end))
}

fn validate_materialized_size(
    bytes: &[u8],
    manifest: &AttachmentManifestV1,
) -> Result<(), MutationError> {
    if bytes.len() as u64 != manifest.decoded_size() {
        return Err(MutationError::RecoveryConflict(
            "materialized attachment size conflicts with its manifest".into(),
        ));
    }
    Ok(())
}

fn complete_status(manifest: &AttachmentManifestV1) -> AttachmentIngestStatus {
    AttachmentIngestStatus::Complete {
        attachment_digest: *manifest.attachment_digest(),
        decoded_size: manifest.decoded_size(),
    }
}

fn manifest_key(manifest_operation_id: &OperationId) -> String {
    format!("{MANIFESTS_ROOT}/{manifest_operation_id}.json")
}

fn chunk_key(manifest_operation_id: &OperationId, chunk_index: u32) -> String {
    format!("{CHUNKS_ROOT}/{manifest_operation_id}/{chunk_index:03}.chunk")
}

fn blob_key(attachment_digest: &Digest32) -> String {
    format!("{BLOBS_ROOT}/{attachment_digest}.blob")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation_id(seed: u8) -> OperationId {
        OperationId::from_bytes([seed; 32])
    }

    fn protocol_parts(
        manifest_operation_id: OperationId,
        bytes: &[u8],
    ) -> (AttachmentManifestV1, Vec<AttachmentChunkV1>) {
        let digest = sha256_attachment_digest(bytes);
        let manifest = AttachmentManifestV1::new(digest, "image/png", bytes.len()).unwrap();
        let chunks = bytes
            .chunks(ATTACHMENT_CHUNK_BYTES)
            .enumerate()
            .map(|(index, data)| {
                AttachmentChunkV1::new(
                    manifest_operation_id,
                    digest,
                    index as u32,
                    manifest.chunk_count(),
                    data.to_vec(),
                )
                .unwrap()
            })
            .collect();
        (manifest, chunks)
    }

    fn fixture() -> (tempfile::TempDir, AttachmentStore) {
        let temp = tempfile::tempdir().unwrap();
        let store = AttachmentStore::open(temp.path()).unwrap();
        (temp, store)
    }

    #[test]
    fn out_of_order_chunks_publish_only_after_the_complete_digest_verifies() {
        let (temp, store) = fixture();
        let operation_id = operation_id(1);
        let bytes = vec![0x5a; ATTACHMENT_CHUNK_BYTES * 2 + 17];
        let (manifest, chunks) = protocol_parts(operation_id, &bytes);

        assert_eq!(
            store.ingest_manifest(operation_id, &manifest).unwrap(),
            AttachmentIngestStatus::Incomplete {
                received_chunks: 0,
                chunk_count: 3,
            }
        );
        assert_eq!(
            store.ingest_chunk(&chunks[2]).unwrap(),
            AttachmentIngestStatus::Incomplete {
                received_chunks: 1,
                chunk_count: 3,
            }
        );
        assert!(store
            .materialized_bytes(manifest.attachment_digest())
            .unwrap()
            .is_none());
        drop(store);
        let store = AttachmentStore::open(temp.path()).unwrap();
        assert!(store
            .materialized_bytes(manifest.attachment_digest())
            .unwrap()
            .is_none());
        assert_eq!(
            store.ingest_chunk(&chunks[0]).unwrap(),
            AttachmentIngestStatus::Incomplete {
                received_chunks: 2,
                chunk_count: 3,
            }
        );
        assert_eq!(
            store.ingest_chunk(&chunks[1]).unwrap(),
            AttachmentIngestStatus::Complete {
                attachment_digest: *manifest.attachment_digest(),
                decoded_size: bytes.len() as u64,
            }
        );
        assert_eq!(
            store
                .materialized_bytes(manifest.attachment_digest())
                .unwrap()
                .unwrap(),
            bytes
        );
    }

    #[test]
    fn identical_chunk_duplicate_is_a_noop_but_same_index_collision_is_rejected() {
        let (_temp, store) = fixture();
        let operation_id = operation_id(2);
        let bytes = vec![0x19; ATTACHMENT_CHUNK_BYTES + 8];
        let (manifest, chunks) = protocol_parts(operation_id, &bytes);
        store.ingest_manifest(operation_id, &manifest).unwrap();

        let first = store.ingest_chunk(&chunks[0]).unwrap();
        assert_eq!(store.ingest_chunk(&chunks[0]).unwrap(), first);

        let mut collided = chunks[0].data().to_vec();
        collided[0] ^= 0xff;
        let collided = AttachmentChunkV1::new(
            operation_id,
            *manifest.attachment_digest(),
            0,
            manifest.chunk_count(),
            collided,
        )
        .unwrap();
        let error = store.ingest_chunk(&collided).unwrap_err();

        assert!(error.to_string().contains("collision"));
        assert!(store
            .materialized_bytes(manifest.attachment_digest())
            .unwrap()
            .is_none());
    }

    #[test]
    fn tampered_complete_chunk_set_never_materializes() {
        let (_temp, store) = fixture();
        let operation_id = operation_id(3);
        let bytes = vec![0x44; ATTACHMENT_CHUNK_BYTES + 11];
        let (manifest, chunks) = protocol_parts(operation_id, &bytes);
        store.ingest_manifest(operation_id, &manifest).unwrap();
        store.ingest_chunk(&chunks[0]).unwrap();

        let mut tampered = chunks[1].data().to_vec();
        tampered[0] ^= 1;
        let tampered = AttachmentChunkV1::new(
            operation_id,
            *manifest.attachment_digest(),
            1,
            manifest.chunk_count(),
            tampered,
        )
        .unwrap();
        let error = store.ingest_chunk(&tampered).unwrap_err();

        assert!(error.to_string().contains("digest"));
        assert!(store
            .materialized_bytes(manifest.attachment_digest())
            .unwrap()
            .is_none());
        assert!(store
            .ingest_chunk(&chunks[1])
            .unwrap_err()
            .to_string()
            .contains("collision"));
    }

    #[test]
    fn chunk_manifest_identity_count_digest_and_exact_size_must_match() {
        let (_temp, store) = fixture();
        let manifest_operation_id = operation_id(4);
        let bytes = vec![0x72; ATTACHMENT_CHUNK_BYTES + 9];
        let (manifest, _chunks) = protocol_parts(manifest_operation_id, &bytes);
        store
            .ingest_manifest(manifest_operation_id, &manifest)
            .unwrap();

        let wrong_manifest = AttachmentChunkV1::new(
            operation_id(5),
            *manifest.attachment_digest(),
            0,
            manifest.chunk_count(),
            vec![0x72; ATTACHMENT_CHUNK_BYTES],
        )
        .unwrap();
        assert!(store.ingest_chunk(&wrong_manifest).is_err());

        let wrong_count = AttachmentChunkV1::new(
            manifest_operation_id,
            *manifest.attachment_digest(),
            0,
            1,
            vec![0x72; ATTACHMENT_CHUNK_BYTES],
        )
        .unwrap();
        assert!(store.ingest_chunk(&wrong_count).is_err());

        let wrong_digest = AttachmentChunkV1::new(
            manifest_operation_id,
            Digest32::from_bytes([0xa5; 32]),
            0,
            manifest.chunk_count(),
            vec![0x72; ATTACHMENT_CHUNK_BYTES],
        )
        .unwrap();
        assert!(store.ingest_chunk(&wrong_digest).is_err());

        let wrong_final_size = AttachmentChunkV1::new(
            manifest_operation_id,
            *manifest.attachment_digest(),
            1,
            manifest.chunk_count(),
            vec![0x72; 8],
        )
        .unwrap();
        assert!(store.ingest_chunk(&wrong_final_size).is_err());
    }

    #[test]
    fn manifest_duplicates_are_noops_but_operation_id_collisions_are_rejected() {
        let (_temp, store) = fixture();
        let operation_id = operation_id(6);
        let (manifest, _) = protocol_parts(operation_id, b"first bytes");
        let (conflicting, _) = protocol_parts(operation_id, b"different bytes");

        let first = store.ingest_manifest(operation_id, &manifest).unwrap();
        assert_eq!(
            store.ingest_manifest(operation_id, &manifest).unwrap(),
            first
        );
        let error = store
            .ingest_manifest(operation_id, &conflicting)
            .unwrap_err();

        assert!(error.to_string().contains("collision"));
    }

    #[test]
    fn identical_digest_deduplicates_across_distinct_manifest_operations() {
        let (_temp, store) = fixture();
        let bytes = b"one content-addressed blob";
        let first_id = operation_id(7);
        let second_id = operation_id(8);
        let (manifest, first_chunks) = protocol_parts(first_id, bytes);

        store.ingest_manifest(first_id, &manifest).unwrap();
        assert!(matches!(
            store.ingest_chunk(&first_chunks[0]).unwrap(),
            AttachmentIngestStatus::Complete { .. }
        ));

        assert!(matches!(
            store.ingest_manifest(second_id, &manifest).unwrap(),
            AttachmentIngestStatus::Complete { .. }
        ));
        assert_eq!(
            store
                .materialized_bytes(manifest.attachment_digest())
                .unwrap()
                .unwrap(),
            bytes
        );
    }

    #[test]
    fn preexisting_wrong_blob_is_preserved_and_rejected() {
        let (temp, store) = fixture();
        let operation_id = operation_id(9);
        let bytes = b"expected bytes";
        let (manifest, _) = protocol_parts(operation_id, bytes);
        let blob_path = temp.path().join(format!(
            "{BLOBS_ROOT}/{}.blob",
            manifest.attachment_digest()
        ));
        std::fs::write(&blob_path, b"foreign bytes").unwrap();

        let error = store.ingest_manifest(operation_id, &manifest).unwrap_err();

        assert!(error.to_string().contains("digest"));
        assert_eq!(std::fs::read(blob_path).unwrap(), b"foreign bytes");
    }

    #[test]
    fn reopened_store_reads_only_a_verified_complete_blob() {
        let (temp, store) = fixture();
        let operation_id = operation_id(10);
        let bytes = b"durable complete bytes";
        let (manifest, chunks) = protocol_parts(operation_id, bytes);
        store.ingest_manifest(operation_id, &manifest).unwrap();
        store.ingest_chunk(&chunks[0]).unwrap();
        drop(store);

        let reopened = AttachmentStore::open(temp.path()).unwrap();

        assert_eq!(
            reopened
                .materialized_bytes(manifest.attachment_digest())
                .unwrap()
                .unwrap(),
            bytes
        );
    }
}
