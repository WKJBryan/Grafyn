use crate::models::attachment::{
    sha256_attachment_digest, AttachmentIngestStatus, AttachmentManifestRecordV1,
    ImageAttachmentCatalogRecordV1,
};
use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{AnchoredRoot, MutationError};
use grafyn_sync_protocol::{
    AttachmentChunkV1, AttachmentManifestV1, Digest32, OperationId, ATTACHMENT_CHUNK_BYTES,
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_CHUNKS,
};
use std::path::Path;
use std::{io::BufReader, io::Cursor};

const ATTACHMENTS_ROOT: &str = "attachments/v1";
const MANIFESTS_ROOT: &str = "attachments/v1/manifests";
const CHUNKS_ROOT: &str = "attachments/v1/chunks";
const BLOBS_ROOT: &str = "attachments/v1/blobs";
const CATALOG_ROOT: &str = "attachments/v1/catalog";
const THUMBNAILS_ROOT: &str = "attachments/v1/thumbnails";
const STAGING_ROOT: &str = "attachments/v1/install-staging";
const GENERATED_STAGE_ROOT: &str = "attachments/v1/generated-staging";
const MANIFEST_RECORD_LIMIT: usize = 1024;
const IMAGE_CATALOG_RECORD_LIMIT: usize = 2048;
const GENERATED_STAGE_RECORD_LIMIT: usize = 4096;
const MAX_IMAGE_DECODE_ALLOC_BYTES: u64 = 128 * 1024 * 1024;
const MAX_GENERATED_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;
const MAX_GENERATED_THUMBNAIL_DIMENSION: u32 = 512;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GeneratedImageStageRecordV1 {
    schema_version: u16,
    vault_scope: ContentDigest,
    mutation_id: ContentDigest,
    catalog: ImageAttachmentCatalogRecordV1,
}

impl GeneratedImageStageRecordV1 {
    fn validate(
        &self,
        expected_scope: &ContentDigest,
        expected_mutation: &ContentDigest,
        expected_digest: &Digest32,
    ) -> Result<(), MutationError> {
        if self.schema_version != 1
            || &self.vault_scope != expected_scope
            || &self.mutation_id != expected_mutation
        {
            return Err(MutationError::RecoveryConflict(
                "generated image stage owner does not match its vault mutation".into(),
            ));
        }
        self.catalog.validate().map_err(MutationError::Invalid)?;
        if self
            .catalog
            .attachment_digest()
            .map_err(MutationError::Invalid)?
            != *expected_digest
        {
            return Err(MutationError::RecoveryConflict(
                "generated image stage digest does not match its mutation evidence".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedGeneratedImage {
    media_type: &'static str,
    width: u32,
    height: u32,
}

impl ValidatedGeneratedImage {
    pub(crate) const fn media_type(&self) -> &'static str {
        self.media_type
    }

    pub(crate) const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedGeneratedImage {
    bytes: Vec<u8>,
    validated: ValidatedGeneratedImage,
}

impl PreparedGeneratedImage {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) const fn media_type(&self) -> &'static str {
        self.validated.media_type()
    }

    pub(crate) const fn dimensions(&self) -> (u32, u32) {
        self.validated.dimensions()
    }
}

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
            CATALOG_ROOT,
            THUMBNAILS_ROOT,
            STAGING_ROOT,
            GENERATED_STAGE_ROOT,
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

    pub(crate) fn store_cataloged_image(
        &self,
        bytes: &[u8],
        declared_media_type: Option<&str>,
    ) -> Result<ImageAttachmentCatalogRecordV1, MutationError> {
        let validated = validate_generated_image_bytes(bytes, declared_media_type)?;
        let digest = sha256_attachment_digest(bytes);
        let (width, height) = validated.dimensions();
        let record = ImageAttachmentCatalogRecordV1::new(
            digest,
            validated.media_type().to_string(),
            bytes.len() as u64,
            width,
            height,
        )
        .map_err(MutationError::Invalid)?;
        let catalog = serde_json::to_vec(&record).map_err(|error| {
            MutationError::Invalid(format!(
                "failed to encode image attachment catalog: {error}"
            ))
        })?;
        if catalog.len() > IMAGE_CATALOG_RECORD_LIMIT {
            return Err(MutationError::Invalid(
                "image attachment catalog exceeds its storage limit".into(),
            ));
        }

        self.install_immutable(
            &blob_key(&digest),
            bytes,
            crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES,
            "cataloged image blob",
        )?;
        self.install_immutable(
            &catalog_key(&digest),
            &catalog,
            IMAGE_CATALOG_RECORD_LIMIT,
            "image attachment catalog",
        )?;
        Ok(record)
    }

    pub(crate) fn cataloged_image(
        &self,
        digest: &Digest32,
    ) -> Result<Option<ImageAttachmentCatalogRecordV1>, MutationError> {
        let Some(bytes) = self
            .root
            .read_bounded(&catalog_key(digest), IMAGE_CATALOG_RECORD_LIMIT)?
        else {
            return Ok(None);
        };
        let record: ImageAttachmentCatalogRecordV1 = serde_json::from_slice(&bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid image catalog: {error}")))?;
        record.validate().map_err(MutationError::Invalid)?;
        if record.attachment_digest().map_err(MutationError::Invalid)? != *digest {
            return Err(MutationError::RecoveryConflict(
                "image catalog digest does not match its content-addressed key".into(),
            ));
        }
        let blob = self.materialized_bytes(digest)?.ok_or_else(|| {
            MutationError::RecoveryConflict("image catalog references a missing blob".into())
        })?;
        let validated = validate_generated_image_bytes(&blob, Some(record.media_type()))?;
        if blob.len() as u64 != record.decoded_size()
            || validated.dimensions() != record.dimensions()
        {
            return Err(MutationError::RecoveryConflict(
                "image catalog metadata conflicts with its decoded blob".into(),
            ));
        }
        Ok(Some(record))
    }

    pub(crate) fn generated_image_thumbnail(
        &self,
        digest: &Digest32,
    ) -> Result<Vec<u8>, MutationError> {
        let catalog = self.cataloged_image(digest)?.ok_or_else(|| {
            MutationError::Invalid("generated image thumbnail requires a cataloged image".into())
        })?;
        let expected_dimension = catalog
            .dimensions()
            .0
            .min(MAX_GENERATED_THUMBNAIL_DIMENSION);
        let key = thumbnail_key(digest);
        if let Ok(Some(cached)) = self.root.read_bounded(&key, MAX_GENERATED_THUMBNAIL_BYTES) {
            if validate_generated_image_bytes(&cached, Some("image/png")).is_ok_and(|validated| {
                validated.dimensions() == (expected_dimension, expected_dimension)
            }) {
                return Ok(cached);
            }
        }

        let canonical = self.materialized_bytes(digest)?.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "generated image thumbnail source blob is missing".into(),
            )
        })?;
        let decoded = image::load_from_memory(&canonical).map_err(|error| {
            MutationError::Invalid(format!(
                "failed to decode generated image thumbnail: {error}"
            ))
        })?;
        let thumbnail = if decoded.width() > MAX_GENERATED_THUMBNAIL_DIMENSION {
            decoded.resize(
                MAX_GENERATED_THUMBNAIL_DIMENSION,
                MAX_GENERATED_THUMBNAIL_DIMENSION,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            decoded
        };
        let mut encoded = Cursor::new(Vec::new());
        thumbnail
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(|error| {
                MutationError::Invalid(format!(
                    "failed to encode generated image thumbnail: {error}"
                ))
            })?;
        let encoded = encoded.into_inner();
        if encoded.len() > MAX_GENERATED_THUMBNAIL_BYTES {
            return Err(MutationError::Invalid(
                "generated image thumbnail exceeds its cache limit".into(),
            ));
        }
        let validated = validate_generated_image_bytes(&encoded, Some("image/png"))?;
        if validated.dimensions() != (expected_dimension, expected_dimension) {
            return Err(MutationError::RecoveryConflict(
                "generated image thumbnail has unexpected dimensions".into(),
            ));
        }
        self.root.put_atomic(&key, &encoded)?;
        let durable = self
            .root
            .read_bounded(&key, MAX_GENERATED_THUMBNAIL_BYTES)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "generated image thumbnail disappeared after publication".into(),
                )
            })?;
        if durable != encoded {
            return Err(MutationError::RecoveryConflict(
                "generated image thumbnail cache changed during publication".into(),
            ));
        }
        Ok(durable)
    }

    pub(crate) fn stage_generated_image(
        &self,
        vault_scope: &ContentDigest,
        mutation_id: &ContentDigest,
        bytes: &[u8],
        declared_media_type: Option<&str>,
    ) -> Result<ImageAttachmentCatalogRecordV1, MutationError> {
        let validated = validate_generated_image_bytes(bytes, declared_media_type)?;
        let digest = sha256_attachment_digest(bytes);
        let (width, height) = validated.dimensions();
        let catalog = ImageAttachmentCatalogRecordV1::new(
            digest,
            validated.media_type().to_string(),
            bytes.len() as u64,
            width,
            height,
        )
        .map_err(MutationError::Invalid)?;
        if let Some(existing) = self.cataloged_image(&digest)? {
            if existing != catalog || self.materialized_bytes(&digest)?.as_deref() != Some(bytes) {
                return Err(MutationError::RecoveryConflict(
                    "existing generated image CAS conflicts with staged bytes".into(),
                ));
            }
            return Ok(existing);
        }

        let record = GeneratedImageStageRecordV1 {
            schema_version: 1,
            vault_scope: vault_scope.clone(),
            mutation_id: mutation_id.clone(),
            catalog: catalog.clone(),
        };
        let record_bytes = serde_json::to_vec(&record).map_err(|error| {
            MutationError::Invalid(format!("failed to encode generated image stage: {error}"))
        })?;
        if record_bytes.len() > GENERATED_STAGE_RECORD_LIMIT {
            return Err(MutationError::Invalid(
                "generated image stage record exceeds its storage limit".into(),
            ));
        }
        self.install_immutable(
            &generated_stage_blob_key(mutation_id),
            bytes,
            crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES,
            "staged generated image blob",
        )?;
        self.install_immutable(
            &generated_stage_record_key(mutation_id),
            &record_bytes,
            GENERATED_STAGE_RECORD_LIMIT,
            "generated image stage record",
        )?;
        let (durable_catalog, durable_bytes) = self
            .staged_generated_image(vault_scope, mutation_id, &digest)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "generated image stage disappeared after installation".into(),
                )
            })?;
        if durable_catalog != catalog || durable_bytes != bytes {
            return Err(MutationError::RecoveryConflict(
                "generated image stage changed after installation".into(),
            ));
        }
        Ok(catalog)
    }

    pub(crate) fn staged_generated_image(
        &self,
        vault_scope: &ContentDigest,
        mutation_id: &ContentDigest,
        expected_digest: &Digest32,
    ) -> Result<Option<(ImageAttachmentCatalogRecordV1, Vec<u8>)>, MutationError> {
        let record_bytes = self.root.read_bounded(
            &generated_stage_record_key(mutation_id),
            GENERATED_STAGE_RECORD_LIMIT,
        )?;
        let bytes = self.root.read_bounded(
            &generated_stage_blob_key(mutation_id),
            crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES,
        )?;
        let (record_bytes, bytes) = match (record_bytes, bytes) {
            (Some(record_bytes), Some(bytes)) => (record_bytes, bytes),
            (None, None) => return Ok(None),
            _ => {
                return Err(MutationError::RecoveryConflict(
                    "generated image stage is only partially durable".into(),
                ))
            }
        };
        let record: GeneratedImageStageRecordV1 =
            serde_json::from_slice(&record_bytes).map_err(|error| {
                MutationError::Invalid(format!("invalid generated image stage: {error}"))
            })?;
        record.validate(vault_scope, mutation_id, expected_digest)?;
        if sha256_attachment_digest(&bytes) != *expected_digest
            || bytes.len() as u64 != record.catalog.decoded_size()
        {
            return Err(MutationError::RecoveryConflict(
                "generated image stage blob conflicts with its catalog".into(),
            ));
        }
        let validated = validate_generated_image_bytes(&bytes, Some(record.catalog.media_type()))?;
        if validated.dimensions() != record.catalog.dimensions() {
            return Err(MutationError::RecoveryConflict(
                "generated image stage dimensions conflict with its catalog".into(),
            ));
        }
        Ok(Some((record.catalog, bytes)))
    }

    pub(crate) fn promote_generated_image_stage(
        &self,
        vault_scope: &ContentDigest,
        mutation_id: &ContentDigest,
        expected_digest: &Digest32,
    ) -> Result<ImageAttachmentCatalogRecordV1, MutationError> {
        if let Some(existing) = self.cataloged_image(expected_digest)? {
            self.cancel_generated_image_stage(mutation_id)?;
            return Ok(existing);
        }
        let (catalog, bytes) = self
            .staged_generated_image(vault_scope, mutation_id, expected_digest)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "committed generated image mutation lost its private stage".into(),
                )
            })?;
        let published = self.store_cataloged_image(&bytes, Some(catalog.media_type()))?;
        if published != catalog {
            return Err(MutationError::RecoveryConflict(
                "published generated image catalog changed from its private stage".into(),
            ));
        }
        self.cancel_generated_image_stage(mutation_id)?;
        Ok(published)
    }

    pub(crate) fn cancel_generated_image_stage(
        &self,
        mutation_id: &ContentDigest,
    ) -> Result<(), MutationError> {
        self.root.delete(&generated_stage_record_key(mutation_id))?;
        self.root.delete(&generated_stage_blob_key(mutation_id))
    }

    #[cfg(test)]
    pub(crate) fn generated_image_stage_file_count(&self) -> Result<usize, MutationError> {
        self.root
            .regular_file_names_bounded(GENERATED_STAGE_ROOT, 1024)
            .map(|names| names.len())
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

    pub(crate) fn catalog_generated_image_from_manifest(
        &self,
        bytes: &[u8],
        manifest: &AttachmentManifestV1,
    ) -> Result<ImageAttachmentCatalogRecordV1, MutationError> {
        let catalog = self.store_cataloged_image(bytes, Some(manifest.media_type()))?;
        if catalog
            .attachment_digest()
            .map_err(MutationError::Invalid)?
            != *manifest.attachment_digest()
            || catalog.decoded_size() != manifest.decoded_size()
        {
            return Err(MutationError::RecoveryConflict(
                "synced image catalog conflicts with its attachment manifest".into(),
            ));
        }
        Ok(catalog)
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

pub(crate) fn validate_generated_image_bytes(
    bytes: &[u8],
    declared_media_type: Option<&str>,
) -> Result<ValidatedGeneratedImage, MutationError> {
    if bytes.is_empty() || bytes.len() > crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES
    {
        return Err(MutationError::Invalid(
            "generated image must be between 1 byte and 24 MiB".into(),
        ));
    }
    let format = image::guess_format(bytes)
        .map_err(|_| MutationError::Invalid("generated image format is unknown".into()))?;
    let media_type = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => {
            return Err(MutationError::Invalid(
                "generated image must be PNG, JPEG, or WebP".into(),
            ))
        }
    };
    validate_exact_image_container(bytes, format)?;
    if let Some(declared) = declared_media_type {
        if declared != media_type {
            return Err(MutationError::Invalid(format!(
                "generated image MIME mismatch: declared {declared}, decoded {media_type}"
            )));
        }
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION);
    limits.max_image_height = Some(crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC_BYTES);
    reject_animated_raster(bytes, format, limits.clone())?;
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| MutationError::Invalid(format!("invalid image header: {error}")))?;
    if width == 0
        || height == 0
        || width > crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION
        || height > crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height)
            > crate::models::image_generation::MAX_GENERATED_IMAGE_PIXELS
        || width != height
    {
        return Err(MutationError::Invalid(
            "generated image must be square within 4096 x 4096 and 16,777,216 pixels".into(),
        ));
    }

    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| {
        MutationError::Invalid(format!("generated image failed full decode: {error}"))
    })?;
    if decoded.width() != width || decoded.height() != height {
        return Err(MutationError::Invalid(
            "generated image dimensions changed during decode".into(),
        ));
    }
    Ok(ValidatedGeneratedImage {
        media_type,
        width,
        height,
    })
}

pub(crate) fn prepare_generated_image_for_storage(
    bytes: &[u8],
    declared_media_type: Option<&str>,
    policy: crate::models::image_generation::ImageMetadataRetentionPolicy,
) -> Result<PreparedGeneratedImage, MutationError> {
    let original = validate_generated_image_bytes(bytes, declared_media_type)?;
    if policy == crate::models::image_generation::ImageMetadataRetentionPolicy::RetainOriginal {
        return Ok(PreparedGeneratedImage {
            bytes: bytes.to_vec(),
            validated: original,
        });
    }

    let format = image::guess_format(bytes)
        .map_err(|_| MutationError::Invalid("generated image format is unknown".into()))?;
    let output = strip_container_metadata(bytes, format)?;
    let validated = validate_generated_image_bytes(&output, Some(original.media_type()))?;
    Ok(PreparedGeneratedImage {
        bytes: output,
        validated,
    })
}

fn strip_container_metadata(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<Vec<u8>, MutationError> {
    match format {
        image::ImageFormat::Png => strip_png_metadata(bytes),
        image::ImageFormat::Jpeg => strip_jpeg_metadata(bytes),
        image::ImageFormat::WebP => strip_webp_metadata(bytes),
        _ => Err(MutationError::Invalid(
            "generated image metadata stripping supports only PNG, JPEG, or WebP".into(),
        )),
    }
}

fn strip_png_metadata(bytes: &[u8]) -> Result<Vec<u8>, MutationError> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut output = PNG_SIGNATURE.to_vec();
    let mut offset = PNG_SIGNATURE.len();
    loop {
        let header_end = offset
            .checked_add(8)
            .ok_or_else(|| MutationError::Invalid("PNG chunk offset overflow".into()))?;
        let header = bytes
            .get(offset..header_end)
            .ok_or_else(|| MutationError::Invalid("PNG is truncated before IEND".into()))?;
        let data_len = u32::from_be_bytes(header[..4].try_into().expect("four bytes")) as usize;
        let chunk_type = &header[4..8];
        let chunk_end = header_end
            .checked_add(data_len)
            .and_then(|value| value.checked_add(4))
            .ok_or_else(|| MutationError::Invalid("PNG chunk length overflow".into()))?;
        let chunk = bytes
            .get(offset..chunk_end)
            .ok_or_else(|| MutationError::Invalid("PNG chunk is truncated".into()))?;
        let rendering_chunk = matches!(chunk_type, b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS");
        if !rendering_chunk && chunk_type[0] & 0x20 == 0 {
            return Err(MutationError::Invalid(
                "PNG contains an unsupported critical rendering chunk".into(),
            ));
        }
        if rendering_chunk {
            output.extend_from_slice(chunk);
        }
        offset = chunk_end;
        if chunk_type == b"IEND" {
            break;
        }
    }
    Ok(output)
}

fn strip_jpeg_metadata(bytes: &[u8]) -> Result<Vec<u8>, MutationError> {
    validate_exact_jpeg_container(bytes)?;
    let mut output = bytes[..2].to_vec();
    let mut offset = 2usize;
    let mut entropy_coded = false;
    while offset < bytes.len() {
        if entropy_coded {
            let entropy_start = offset;
            loop {
                let marker_start = bytes[offset..]
                    .iter()
                    .position(|byte| *byte == 0xff)
                    .map(|relative| offset + relative)
                    .ok_or_else(|| MutationError::Invalid("JPEG is missing EOI".into()))?;
                let mut marker_offset = marker_start + 1;
                while bytes.get(marker_offset) == Some(&0xff) {
                    marker_offset += 1;
                }
                let marker = *bytes.get(marker_offset).ok_or_else(|| {
                    MutationError::Invalid("JPEG is truncated in entropy data".into())
                })?;
                if marker == 0x00 || matches!(marker, 0xd0..=0xd7) {
                    offset = marker_offset + 1;
                    continue;
                }
                output.extend_from_slice(&bytes[entropy_start..marker_start]);
                offset = marker_start;
                entropy_coded = false;
                break;
            }
            continue;
        }

        let marker_start = offset;
        if bytes.get(offset) != Some(&0xff) {
            return Err(MutationError::Invalid(
                "JPEG marker stream is malformed".into(),
            ));
        }
        offset += 1;
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| MutationError::Invalid("JPEG marker is truncated".into()))?;
        offset += 1;
        match marker {
            0xd9 => {
                output.extend_from_slice(&bytes[marker_start..offset]);
                break;
            }
            0x01 | 0xd0..=0xd7 => {
                output.extend_from_slice(&bytes[marker_start..offset]);
            }
            _ => {
                let length = u16::from_be_bytes(
                    bytes
                        .get(offset..offset + 2)
                        .ok_or_else(|| {
                            MutationError::Invalid("JPEG segment length is truncated".into())
                        })?
                        .try_into()
                        .expect("two bytes"),
                ) as usize;
                let segment_end = offset
                    .checked_add(length)
                    .ok_or_else(|| MutationError::Invalid("JPEG segment offset overflow".into()))?;
                if segment_end > bytes.len() {
                    return Err(MutationError::Invalid("JPEG segment is truncated".into()));
                }
                if matches!(marker, 0xc0..=0xc7 | 0xc9..=0xcf | 0xda..=0xdf) {
                    output.extend_from_slice(&bytes[marker_start..segment_end]);
                }
                offset = segment_end;
                entropy_coded = marker == 0xda;
            }
        }
    }
    Ok(output)
}

fn strip_webp_metadata(bytes: &[u8]) -> Result<Vec<u8>, MutationError> {
    let mut body = Vec::with_capacity(bytes.len().saturating_sub(12));
    let mut offset = 12usize;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or_else(|| MutationError::Invalid("WebP chunk header is truncated".into()))?;
        let kind = &header[..4];
        let data_len = u32::from_le_bytes(header[4..8].try_into().expect("four bytes")) as usize;
        let chunk_end = offset
            .checked_add(8)
            .and_then(|value| value.checked_add(data_len))
            .and_then(|value| value.checked_add(data_len % 2))
            .ok_or_else(|| MutationError::Invalid("WebP chunk offset overflow".into()))?;
        let chunk = bytes
            .get(offset..chunk_end)
            .ok_or_else(|| MutationError::Invalid("WebP chunk is truncated".into()))?;
        if matches!(kind, b"VP8X" | b"ALPH" | b"VP8 " | b"VP8L") {
            if kind == b"VP8X" {
                if data_len != 10 {
                    return Err(MutationError::Invalid("WebP VP8X chunk is invalid".into()));
                }
                let mut sanitized = chunk.to_vec();
                sanitized[8] &= !(0x20 | 0x08 | 0x04);
                body.extend_from_slice(&sanitized);
            } else {
                body.extend_from_slice(chunk);
            }
        }
        offset = chunk_end;
    }
    let riff_len = body
        .len()
        .checked_add(4)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| MutationError::Invalid("WebP container exceeds RIFF bounds".into()))?;
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&riff_len.to_le_bytes());
    output.extend_from_slice(b"WEBP");
    output.extend_from_slice(&body);
    Ok(output)
}

fn validate_exact_image_container(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<(), MutationError> {
    match format {
        image::ImageFormat::Png => {
            const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
            if !bytes.starts_with(PNG_SIGNATURE) {
                return Err(MutationError::Invalid("invalid PNG signature".into()));
            }
            let mut offset = PNG_SIGNATURE.len();
            let mut first = true;
            loop {
                let header_end = offset
                    .checked_add(8)
                    .ok_or_else(|| MutationError::Invalid("PNG chunk offset overflow".into()))?;
                let header = bytes
                    .get(offset..header_end)
                    .ok_or_else(|| MutationError::Invalid("PNG is truncated before IEND".into()))?;
                let data_len = u32::from_be_bytes(header[..4].try_into().expect("four bytes"));
                let chunk_type = &header[4..8];
                if first && chunk_type != b"IHDR" {
                    return Err(MutationError::Invalid("PNG must begin with IHDR".into()));
                }
                first = false;
                let chunk_end = header_end
                    .checked_add(data_len as usize)
                    .and_then(|value| value.checked_add(4))
                    .ok_or_else(|| MutationError::Invalid("PNG chunk length overflow".into()))?;
                if chunk_end > bytes.len() {
                    return Err(MutationError::Invalid("PNG chunk is truncated".into()));
                }
                offset = chunk_end;
                if chunk_type == b"IEND" {
                    if data_len != 0 || offset != bytes.len() {
                        return Err(MutationError::Invalid(
                            "PNG contains bytes after its exact IEND".into(),
                        ));
                    }
                    break;
                }
            }
        }
        image::ImageFormat::Jpeg => {
            validate_exact_jpeg_container(bytes)?;
        }
        image::ImageFormat::WebP => {
            if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
                return Err(MutationError::Invalid("invalid WebP container".into()));
            }
            let declared_len =
                u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")) as usize + 8;
            if declared_len != bytes.len() {
                return Err(MutationError::Invalid(
                    "WebP container length does not consume the exact payload".into(),
                ));
            }
        }
        _ => unreachable!("format filtered before container validation"),
    }
    Ok(())
}

fn validate_exact_jpeg_container(bytes: &[u8]) -> Result<(), MutationError> {
    if bytes.len() < 4 || bytes.get(..2) != Some(&[0xff, 0xd8]) {
        return Err(MutationError::Invalid("invalid JPEG SOI".into()));
    }
    let mut offset = 2usize;
    let mut entropy_coded = false;
    loop {
        if entropy_coded {
            let mut found_marker = false;
            while offset < bytes.len() {
                if bytes[offset] != 0xff {
                    offset += 1;
                    continue;
                }
                let marker_start = offset;
                offset += 1;
                while bytes.get(offset) == Some(&0xff) {
                    offset += 1;
                }
                let marker = *bytes.get(offset).ok_or_else(|| {
                    MutationError::Invalid("JPEG is truncated in entropy data".into())
                })?;
                offset += 1;
                match marker {
                    0x00 | 0xd0..=0xd7 => continue,
                    0xd9 => {
                        if offset != bytes.len() {
                            return Err(MutationError::Invalid(
                                "JPEG contains bytes after its first EOI".into(),
                            ));
                        }
                        return Ok(());
                    }
                    _ => {
                        offset = marker_start;
                        entropy_coded = false;
                        found_marker = true;
                        break;
                    }
                }
            }
            if !found_marker {
                return Err(MutationError::Invalid("JPEG is missing EOI".into()));
            }
            continue;
        }

        if bytes.get(offset) != Some(&0xff) {
            return Err(MutationError::Invalid(
                "JPEG marker stream is malformed".into(),
            ));
        }
        offset += 1;
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| MutationError::Invalid("JPEG marker is truncated".into()))?;
        offset += 1;
        match marker {
            0xd8 => return Err(MutationError::Invalid("JPEG contains a nested SOI".into())),
            0xd9 => {
                if offset != bytes.len() {
                    return Err(MutationError::Invalid(
                        "JPEG contains bytes after its first EOI".into(),
                    ));
                }
                return Ok(());
            }
            0x01 | 0xd0..=0xd7 => continue,
            _ => {
                let length_bytes =
                    bytes.get(offset..offset.saturating_add(2)).ok_or_else(|| {
                        MutationError::Invalid("JPEG segment length is truncated".into())
                    })?;
                let length =
                    u16::from_be_bytes(length_bytes.try_into().expect("JPEG length has two bytes"))
                        as usize;
                if length < 2 {
                    return Err(MutationError::Invalid(
                        "JPEG segment length is invalid".into(),
                    ));
                }
                offset = offset
                    .checked_add(length)
                    .ok_or_else(|| MutationError::Invalid("JPEG segment offset overflow".into()))?;
                if offset > bytes.len() {
                    return Err(MutationError::Invalid("JPEG segment is truncated".into()));
                }
                entropy_coded = marker == 0xda;
            }
        }
    }
}

fn reject_animated_raster(
    bytes: &[u8],
    format: image::ImageFormat,
    limits: image::Limits,
) -> Result<(), MutationError> {
    match format {
        image::ImageFormat::Png => {
            let decoder = image::codecs::png::PngDecoder::with_limits(
                BufReader::new(Cursor::new(bytes)),
                limits,
            )
            .map_err(|error| MutationError::Invalid(format!("invalid PNG: {error}")))?;
            if decoder
                .is_apng()
                .map_err(|error| MutationError::Invalid(format!("invalid PNG: {error}")))?
            {
                return Err(MutationError::Invalid(
                    "animated PNG is not supported".into(),
                ));
            }
        }
        image::ImageFormat::WebP => {
            let mut decoder =
                image::codecs::webp::WebPDecoder::new(BufReader::new(Cursor::new(bytes)))
                    .map_err(|error| MutationError::Invalid(format!("invalid WebP: {error}")))?;
            if decoder.has_animation() {
                return Err(MutationError::Invalid(
                    "animated WebP is not supported".into(),
                ));
            }
            image::ImageDecoder::set_limits(&mut decoder, limits)
                .map_err(|error| MutationError::Invalid(format!("invalid WebP: {error}")))?;
        }
        image::ImageFormat::Jpeg => {}
        _ => unreachable!("format filtered before animation validation"),
    }
    Ok(())
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

fn catalog_key(attachment_digest: &Digest32) -> String {
    format!("{CATALOG_ROOT}/{attachment_digest}.json")
}

fn thumbnail_key(attachment_digest: &Digest32) -> String {
    format!("{THUMBNAILS_ROOT}/{attachment_digest}.png")
}

fn generated_stage_record_key(mutation_id: &ContentDigest) -> String {
    format!("{GENERATED_STAGE_ROOT}/{}.json", mutation_id.as_str())
}

fn generated_stage_blob_key(mutation_id: &ContentDigest) -> String {
    format!("{GENERATED_STAGE_ROOT}/{}.blob", mutation_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};
    use std::io::Cursor;

    fn operation_id(seed: u8) -> OperationId {
        OperationId::from_bytes([seed; 32])
    }

    fn protocol_parts(
        manifest_operation_id: OperationId,
        bytes: &[u8],
    ) -> (AttachmentManifestV1, Vec<AttachmentChunkV1>) {
        protocol_parts_with_media_type(manifest_operation_id, bytes, "application/octet-stream")
    }

    fn protocol_parts_with_media_type(
        manifest_operation_id: OperationId,
        bytes: &[u8],
        media_type: &str,
    ) -> (AttachmentManifestV1, Vec<AttachmentChunkV1>) {
        let digest = sha256_attachment_digest(bytes);
        let manifest = AttachmentManifestV1::new(digest, media_type, bytes.len()).unwrap();
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

    fn raster_bytes(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::new_rgba8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn png_with_text_metadata() -> Vec<u8> {
        let mut bytes = raster_bytes(ImageFormat::Png, 2, 2);
        let iend_offset = bytes.len() - 12;
        let payload = b"Comment\0private-provider-metadata";
        let mut crc_input = b"tEXt".to_vec();
        crc_input.extend_from_slice(payload);
        let mut crc = 0xffff_ffffu32;
        for byte in crc_input {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        crc = !crc;
        let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(payload);
        chunk.extend_from_slice(&crc.to_be_bytes());
        bytes.splice(iend_offset..iend_offset, chunk);
        let private_payload = b"private-png-ancillary";
        let mut private_crc_input = b"vpAg".to_vec();
        private_crc_input.extend_from_slice(private_payload);
        let mut private_crc = 0xffff_ffffu32;
        for byte in private_crc_input {
            private_crc ^= u32::from(byte);
            for _ in 0..8 {
                private_crc =
                    (private_crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(private_crc & 1));
            }
        }
        let mut private_chunk = (private_payload.len() as u32).to_be_bytes().to_vec();
        private_chunk.extend_from_slice(b"vpAg");
        private_chunk.extend_from_slice(private_payload);
        private_chunk.extend_from_slice(&(!private_crc).to_be_bytes());
        bytes.splice(iend_offset..iend_offset, private_chunk);
        bytes
    }

    fn png_idat_chunks(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut offset = 8usize;
        let mut chunks = Vec::new();
        while offset < bytes.len() {
            let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let chunk_type = &bytes[offset + 4..offset + 8];
            let end = offset + 12 + length;
            if chunk_type == b"IDAT" {
                chunks.push(bytes[offset + 8..offset + 8 + length].to_vec());
            }
            offset = end;
            if chunk_type == b"IEND" {
                break;
            }
        }
        chunks
    }

    fn jpeg_with_metadata() -> Vec<u8> {
        let original = raster_bytes(ImageFormat::Jpeg, 2, 2);
        let mut metadata = vec![0xff, 0xe1, 0x00, 0x0b];
        metadata.extend_from_slice(b"Exif\0\0abc");
        metadata.extend_from_slice(&[0xff, 0xfe, 0x00, 0x08]);
        metadata.extend_from_slice(b"secret");
        let private = b"private-jpeg-segment";
        metadata.extend_from_slice(&[0xff, 0xf0]);
        metadata.extend_from_slice(&((private.len() + 2) as u16).to_be_bytes());
        metadata.extend_from_slice(private);
        let mut bytes = original[..2].to_vec();
        bytes.extend_from_slice(&metadata);
        bytes.extend_from_slice(&original[2..]);
        bytes
    }

    fn jpeg_scan_suffix(bytes: &[u8]) -> &[u8] {
        let offset = bytes
            .windows(2)
            .position(|window| window == [0xff, 0xda])
            .expect("JPEG fixture has SOS");
        &bytes[offset..]
    }

    fn webp_with_metadata() -> Vec<u8> {
        let original = raster_bytes(ImageFormat::WebP, 2, 2);
        let image_chunks = original[12..].to_vec();
        let mut body = Vec::new();
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x2c, 0, 0, 0, 1, 0, 0, 1, 0, 0]);
        for (kind, payload) in [
            (*b"ICCP", b"icc".as_slice()),
            (*b"EXIF", b"private-exif".as_slice()),
            (*b"XMP ", b"private-xmp".as_slice()),
            (*b"PRIV", b"private-webp-chunk".as_slice()),
        ] {
            body.extend_from_slice(&kind);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        body.extend_from_slice(&image_chunks);
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(&body);
        bytes
    }

    fn webp_image_chunks(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut offset = 12usize;
        let mut chunks = Vec::new();
        while offset < bytes.len() {
            let kind = &bytes[offset..offset + 4];
            let length =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let end = offset + 8 + length + (length % 2);
            if matches!(kind, b"VP8 " | b"VP8L") {
                chunks.push(bytes[offset..end].to_vec());
            }
            offset = end;
        }
        chunks
    }

    #[test]
    fn generated_raster_validation_fully_decodes_supported_exact_mime() {
        for (format, media_type) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let bytes = raster_bytes(format, 2, 2);
            let validated = validate_generated_image_bytes(&bytes, Some(media_type)).unwrap();
            assert_eq!(validated.media_type(), media_type);
            assert_eq!(validated.dimensions(), (2, 2));
        }
    }

    #[test]
    fn generated_raster_validation_infers_missing_mime_but_rejects_mismatch_svg_and_truncation() {
        let png = raster_bytes(ImageFormat::Png, 2, 2);
        assert_eq!(
            validate_generated_image_bytes(&png, None)
                .unwrap()
                .media_type(),
            "image/png"
        );
        assert!(validate_generated_image_bytes(&png, Some("image/jpeg")).is_err());
        assert!(validate_generated_image_bytes(b"<svg></svg>", Some("image/svg+xml")).is_err());
        assert!(validate_generated_image_bytes(&png[..png.len() - 4], Some("image/png")).is_err());
        assert!(validate_generated_image_bytes(
            &raster_bytes(ImageFormat::Png, 4097, 1),
            Some("image/png")
        )
        .is_err());
        assert!(validate_generated_image_bytes(
            &raster_bytes(ImageFormat::Png, 2, 1),
            Some("image/png")
        )
        .is_err());
    }

    #[test]
    fn generated_raster_rejects_trailing_polyglot_bytes_for_every_supported_format() {
        for (format, media_type) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let mut bytes = raster_bytes(format, 2, 2);
            bytes.extend_from_slice(b"<script>foreign trailer</script>");
            assert!(
                validate_generated_image_bytes(&bytes, Some(media_type)).is_err(),
                "{format:?} must reject appended bytes"
            );
        }
    }

    #[test]
    fn generated_jpeg_rejects_concatenation_and_a_forged_final_eoi() {
        let jpeg = raster_bytes(ImageFormat::Jpeg, 2, 2);
        let mut concatenated = jpeg.clone();
        concatenated.extend_from_slice(&jpeg);
        assert!(validate_generated_image_bytes(&concatenated, Some("image/jpeg")).is_err());

        let mut forged_trailer = jpeg;
        forged_trailer.extend_from_slice(b"foreign-payload");
        forged_trailer.extend_from_slice(&[0xff, 0xd9]);
        assert!(validate_generated_image_bytes(&forged_trailer, Some("image/jpeg")).is_err());
    }

    #[test]
    fn storage_preparation_strips_metadata_by_default_and_retains_only_when_explicit() {
        let original = png_with_text_metadata();
        validate_generated_image_bytes(&original, Some("image/png")).unwrap();
        let original_idat = png_idat_chunks(&original);

        let stripped = prepare_generated_image_for_storage(
            &original,
            Some("image/png"),
            crate::models::image_generation::ImageMetadataRetentionPolicy::StripMetadata,
        )
        .unwrap();
        assert_ne!(stripped.bytes(), original);
        assert!(!stripped
            .bytes()
            .windows(b"private-provider-metadata".len())
            .any(|window| window == b"private-provider-metadata"));
        assert!(!stripped
            .bytes()
            .windows(b"private-png-ancillary".len())
            .any(|window| window == b"private-png-ancillary"));
        assert_eq!(stripped.media_type(), "image/png");
        assert_eq!(png_idat_chunks(stripped.bytes()), original_idat);

        let retained = prepare_generated_image_for_storage(
            &original,
            Some("image/png"),
            crate::models::image_generation::ImageMetadataRetentionPolicy::RetainOriginal,
        )
        .unwrap();
        assert_eq!(retained.bytes(), original);
    }

    #[test]
    fn metadata_stripping_preserves_jpeg_scan_and_webp_compressed_image_payloads() {
        let jpeg = jpeg_with_metadata();
        validate_generated_image_bytes(&jpeg, Some("image/jpeg")).unwrap();
        let jpeg_scan = jpeg_scan_suffix(&jpeg).to_vec();
        let stripped_jpeg = prepare_generated_image_for_storage(
            &jpeg,
            Some("image/jpeg"),
            crate::models::image_generation::ImageMetadataRetentionPolicy::StripMetadata,
        )
        .unwrap();
        assert_eq!(jpeg_scan_suffix(stripped_jpeg.bytes()), jpeg_scan);
        assert!(!stripped_jpeg
            .bytes()
            .windows(b"private-exif".len())
            .any(|window| window == b"private-exif"));
        assert!(!stripped_jpeg
            .bytes()
            .windows(b"secret".len())
            .any(|window| window == b"secret"));
        assert!(!stripped_jpeg
            .bytes()
            .windows(b"private-jpeg-segment".len())
            .any(|window| window == b"private-jpeg-segment"));

        let webp = webp_with_metadata();
        validate_generated_image_bytes(&webp, Some("image/webp")).unwrap();
        let image_chunks = webp_image_chunks(&webp);
        let stripped_webp = prepare_generated_image_for_storage(
            &webp,
            Some("image/webp"),
            crate::models::image_generation::ImageMetadataRetentionPolicy::StripMetadata,
        )
        .unwrap();
        assert_eq!(webp_image_chunks(stripped_webp.bytes()), image_chunks);
        for marker in [b"ICCP".as_slice(), b"EXIF", b"XMP ", b"PRIV"] {
            assert!(!stripped_webp
                .bytes()
                .windows(marker.len())
                .any(|window| window == marker));
        }
    }

    #[test]
    fn cataloged_image_put_is_scoped_content_addressed_idempotent_and_collision_safe() {
        let (temp, store) = fixture();
        let bytes = raster_bytes(ImageFormat::Png, 2, 2);
        let first = store
            .store_cataloged_image(&bytes, Some("image/png"))
            .unwrap();
        let duplicate = store
            .store_cataloged_image(&bytes, Some("image/png"))
            .unwrap();
        assert_eq!(duplicate, first);
        assert_eq!(
            store
                .cataloged_image(&first.attachment_digest().unwrap())
                .unwrap()
                .unwrap(),
            first
        );
        drop(store);
        let reopened = AttachmentStore::open(temp.path()).unwrap();
        assert_eq!(
            reopened
                .materialized_bytes(&first.attachment_digest().unwrap())
                .unwrap()
                .unwrap(),
            bytes
        );

        let catalog_path = temp.path().join(format!(
            "{CATALOG_ROOT}/{}.json",
            first.attachment_digest().unwrap()
        ));
        std::fs::write(
            &catalog_path,
            format!(
                "{{\"schemaVersion\":1,\"attachmentDigest\":\"{}\",\"mediaType\":\"image/png\",\"decodedSize\":{},\"width\":1,\"height\":1}}",
                first.attachment_digest().unwrap(),
                bytes.len()
            ),
        )
        .unwrap();
        let error = reopened
            .store_cataloged_image(&bytes, Some("image/png"))
            .unwrap_err();
        assert!(error.to_string().contains("collision"));
    }

    #[test]
    fn generated_thumbnail_is_bounded_digest_keyed_and_rebuilds_corrupt_cache() {
        let (temp, store) = fixture();
        let bytes = raster_bytes(ImageFormat::Png, 1024, 1024);
        let catalog = store
            .store_cataloged_image(&bytes, Some("image/png"))
            .unwrap();
        let digest = catalog.attachment_digest().unwrap();

        let first = store.generated_image_thumbnail(&digest).unwrap();
        let validated = validate_generated_image_bytes(&first, Some("image/png")).unwrap();
        assert_eq!(validated.dimensions(), (512, 512));
        assert_eq!(store.materialized_bytes(&digest).unwrap().unwrap(), bytes);

        let cache_path = temp.path().join(thumbnail_key(&digest));
        std::fs::write(&cache_path, b"corrupt derived cache").unwrap();
        let rebuilt = store.generated_image_thumbnail(&digest).unwrap();
        assert_eq!(rebuilt, first);
        assert_eq!(store.materialized_bytes(&digest).unwrap().unwrap(), bytes);

        drop(store);
        let reopened = AttachmentStore::open(temp.path()).unwrap();
        assert_eq!(reopened.generated_image_thumbnail(&digest).unwrap(), first);
    }

    #[test]
    fn cataloged_image_read_rejects_blob_corruption() {
        let (temp, store) = fixture();
        let bytes = raster_bytes(ImageFormat::Png, 2, 2);
        let record = store
            .store_cataloged_image(&bytes, Some("image/png"))
            .unwrap();
        let digest = record.attachment_digest().unwrap();
        std::fs::write(
            temp.path().join(format!("{BLOBS_ROOT}/{digest}.blob")),
            b"foreign bytes",
        )
        .unwrap();

        let error = store.cataloged_image(&digest).unwrap_err();
        assert!(error.to_string().contains("digest"));
    }

    #[test]
    fn explicit_generated_image_intent_reconstructs_catalog_after_blob_only_crash_window() {
        let (_temp, store) = fixture();
        let bytes = raster_bytes(ImageFormat::Png, 2, 2);
        let manifest_operation_id = operation_id(0x2a);
        let (manifest, chunks) =
            protocol_parts_with_media_type(manifest_operation_id, &bytes, "image/png");
        let record = AttachmentManifestRecordV1::from_protocol(manifest_operation_id, &manifest);
        let record_bytes = serde_json::to_vec(&record).unwrap();
        store
            .install_immutable(
                &manifest_key(&manifest_operation_id),
                &record_bytes,
                MANIFEST_RECORD_LIMIT,
                "attachment manifest",
            )
            .unwrap();
        store
            .install_immutable(
                &blob_key(manifest.attachment_digest()),
                &bytes,
                MAX_ATTACHMENT_BYTES,
                "materialized attachment blob",
            )
            .unwrap();
        assert_eq!(
            store.cataloged_image(manifest.attachment_digest()).unwrap(),
            None
        );

        assert!(matches!(
            store.ingest_chunk(&chunks[0]).unwrap(),
            AttachmentIngestStatus::Complete { .. }
        ));
        assert_eq!(
            store.cataloged_image(manifest.attachment_digest()).unwrap(),
            None
        );
        store
            .catalog_generated_image_from_manifest(&bytes, &manifest)
            .unwrap();
        assert!(store
            .cataloged_image(manifest.attachment_digest())
            .unwrap()
            .is_some());
    }

    #[test]
    fn generic_non_square_png_materializes_reorders_and_reopens_without_image_catalog() {
        let (temp, store) = fixture();
        let bytes = raster_bytes(ImageFormat::Png, 2, 1);
        let manifest_operation_id = operation_id(0x2b);
        let (manifest, chunks) =
            protocol_parts_with_media_type(manifest_operation_id, &bytes, "image/png");

        store
            .ingest_manifest(manifest_operation_id, &manifest)
            .unwrap();
        for chunk in chunks.iter().rev() {
            store.ingest_chunk(chunk).unwrap();
        }
        assert_eq!(
            store
                .materialized_bytes(manifest.attachment_digest())
                .unwrap(),
            Some(bytes.clone())
        );
        assert_eq!(
            store.cataloged_image(manifest.attachment_digest()).unwrap(),
            None
        );
        drop(store);

        let reopened = AttachmentStore::open(temp.path()).unwrap();
        assert_eq!(
            reopened
                .materialized_bytes(manifest.attachment_digest())
                .unwrap(),
            Some(bytes)
        );
        assert_eq!(
            reopened
                .cataloged_image(manifest.attachment_digest())
                .unwrap(),
            None
        );
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
