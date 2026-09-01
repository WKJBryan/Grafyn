use grafyn_sync_protocol::{AttachmentManifestV1, Digest32, OperationId, ProtocolError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub(crate) const ATTACHMENT_MANIFEST_RECORD_SCHEMA_VERSION: u16 = 1;
pub(crate) const IMAGE_ATTACHMENT_CATALOG_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImageAttachmentCatalogRecordV1 {
    schema_version: u16,
    attachment_digest: String,
    media_type: String,
    decoded_size: u64,
    width: u32,
    height: u32,
}

impl ImageAttachmentCatalogRecordV1 {
    pub(crate) fn new(
        attachment_digest: Digest32,
        media_type: String,
        decoded_size: u64,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let record = Self {
            schema_version: IMAGE_ATTACHMENT_CATALOG_SCHEMA_VERSION,
            attachment_digest: attachment_digest.to_string(),
            media_type,
            decoded_size,
            width,
            height,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != IMAGE_ATTACHMENT_CATALOG_SCHEMA_VERSION {
            return Err("unsupported image attachment catalog schema".into());
        }
        Digest32::parse_hex(&self.attachment_digest)
            .map_err(|_| "invalid image attachment digest".to_string())?;
        if !matches!(
            self.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        ) {
            return Err("image attachment MIME must be PNG, JPEG, or WebP".into());
        }
        if self.decoded_size == 0
            || self.decoded_size > crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES as u64
        {
            return Err("image attachment must be between 1 byte and 24 MiB".into());
        }
        if self.width == 0
            || self.height == 0
            || self.width > crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION
            || self.height > crate::models::image_generation::MAX_GENERATED_IMAGE_DIMENSION
            || u64::from(self.width) * u64::from(self.height)
                > crate::models::image_generation::MAX_GENERATED_IMAGE_PIXELS
        {
            return Err("image attachment dimensions exceed Grafyn bounds".into());
        }
        Ok(())
    }

    pub(crate) fn attachment_digest(&self) -> Result<Digest32, String> {
        Digest32::parse_hex(&self.attachment_digest)
            .map_err(|_| "invalid image attachment digest".to_string())
    }

    pub(crate) fn media_type(&self) -> &str {
        &self.media_type
    }

    pub(crate) const fn decoded_size(&self) -> u64 {
        self.decoded_size
    }

    pub(crate) const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentManifestRecordV1 {
    schema_version: u16,
    manifest_operation_id: String,
    attachment_digest: String,
    media_type: String,
    decoded_size: u64,
    chunk_size: u32,
    chunk_count: u32,
}

impl AttachmentManifestRecordV1 {
    pub(crate) fn from_protocol(
        manifest_operation_id: OperationId,
        manifest: &AttachmentManifestV1,
    ) -> Self {
        Self {
            schema_version: ATTACHMENT_MANIFEST_RECORD_SCHEMA_VERSION,
            manifest_operation_id: manifest_operation_id.to_string(),
            attachment_digest: manifest.attachment_digest().to_string(),
            media_type: manifest.media_type().to_string(),
            decoded_size: manifest.decoded_size(),
            chunk_size: manifest.chunk_size(),
            chunk_count: manifest.chunk_count(),
        }
    }

    pub(crate) fn to_protocol(&self) -> Result<(OperationId, AttachmentManifestV1), ProtocolError> {
        if self.schema_version != ATTACHMENT_MANIFEST_RECORD_SCHEMA_VERSION {
            return Err(ProtocolError::InvalidField(
                "attachment_manifest_record_schema",
            ));
        }
        let manifest_operation_id = OperationId::parse_hex(&self.manifest_operation_id)?;
        let attachment_digest = Digest32::parse_hex(&self.attachment_digest)?;
        let decoded_size = usize::try_from(self.decoded_size)
            .map_err(|_| ProtocolError::LimitExceeded("attachment_size"))?;
        let manifest =
            AttachmentManifestV1::new(attachment_digest, self.media_type.clone(), decoded_size)?;
        if manifest.chunk_size() != self.chunk_size || manifest.chunk_count() != self.chunk_count {
            return Err(ProtocolError::InvalidField(
                "attachment_manifest_chunk_layout",
            ));
        }
        Ok((manifest_operation_id, manifest))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttachmentIngestStatus {
    Incomplete {
        received_chunks: u32,
        chunk_count: u32,
    },
    Complete {
        attachment_digest: Digest32,
        decoded_size: u64,
    },
}

pub(crate) fn sha256_attachment_digest(bytes: &[u8]) -> Digest32 {
    Digest32::from_bytes(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grafyn_sync_protocol::{
        ATTACHMENT_CHUNK_BYTES, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_CHUNKS,
    };

    fn operation_id(seed: u8) -> OperationId {
        OperationId::from_bytes([seed; 32])
    }

    #[test]
    fn manifest_record_round_trips_protocol_metadata_exactly() {
        let digest = sha256_attachment_digest(b"complete attachment");
        let manifest =
            AttachmentManifestV1::new(digest, "image/png", ATTACHMENT_CHUNK_BYTES + 7).unwrap();
        let record = AttachmentManifestRecordV1::from_protocol(operation_id(7), &manifest);

        let json = serde_json::to_vec(&record).unwrap();
        let decoded: AttachmentManifestRecordV1 = serde_json::from_slice(&json).unwrap();
        let (decoded_operation_id, decoded_manifest) = decoded.to_protocol().unwrap();

        assert_eq!(decoded_operation_id, operation_id(7));
        assert_eq!(decoded_manifest, manifest);
    }

    #[test]
    fn durable_manifest_rejects_unknown_fields_and_unsupported_schema() {
        let unknown = br#"{"schema_version":1,"manifest_operation_id":"0707070707070707070707070707070707070707070707070707070707070707","attachment_digest":"0808080808080808080808080808080808080808080808080808080808080808","media_type":"image/png","decoded_size":1,"chunk_size":262144,"chunk_count":1,"extra":true}"#;
        assert!(serde_json::from_slice::<AttachmentManifestRecordV1>(unknown).is_err());

        let manifest = AttachmentManifestV1::new(
            sha256_attachment_digest(b"x"),
            "application/octet-stream",
            1,
        )
        .unwrap();
        let mut record = AttachmentManifestRecordV1::from_protocol(operation_id(9), &manifest);
        record.schema_version = 2;

        assert_eq!(
            record.to_protocol().unwrap_err(),
            ProtocolError::InvalidField("attachment_manifest_record_schema")
        );
    }

    #[test]
    fn durable_manifest_revalidates_all_protocol_bounds() {
        let manifest = AttachmentManifestV1::new(
            sha256_attachment_digest(b"x"),
            "application/octet-stream",
            1,
        )
        .unwrap();
        let base = AttachmentManifestRecordV1::from_protocol(operation_id(3), &manifest);

        let mut oversized = base.clone();
        oversized.decoded_size = MAX_ATTACHMENT_BYTES as u64 + 1;
        assert!(oversized.to_protocol().is_err());

        let mut wrong_chunk_size = base.clone();
        wrong_chunk_size.chunk_size = ATTACHMENT_CHUNK_BYTES as u32 - 1;
        assert!(wrong_chunk_size.to_protocol().is_err());

        let mut too_many_chunks = base.clone();
        too_many_chunks.chunk_count = MAX_ATTACHMENT_CHUNKS as u32 + 1;
        assert!(too_many_chunks.to_protocol().is_err());

        let mut malformed_digest = base;
        malformed_digest.attachment_digest = "../blob".into();
        assert!(malformed_digest.to_protocol().is_err());
    }

    #[test]
    fn attachment_digest_is_full_lowercase_sha256() {
        assert_eq!(
            sha256_attachment_digest(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn image_catalog_record_round_trips_only_intrinsic_cas_metadata() {
        let digest = sha256_attachment_digest(b"stored raster bytes");
        let record =
            ImageAttachmentCatalogRecordV1::new(digest, "image/png".into(), 19, 1024, 1024)
                .unwrap();

        let encoded = serde_json::to_value(&record).unwrap();
        assert!(encoded.get("source").is_none());
        assert!(encoded.get("annotation").is_none());
        assert!(encoded.get("createdAt").is_none());
        assert!(encoded.get("retentionPolicy").is_none());
        let decoded: ImageAttachmentCatalogRecordV1 = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, record);
        assert_eq!(decoded.attachment_digest().unwrap(), digest);
        assert_eq!(decoded.media_type(), "image/png");
        assert_eq!(decoded.decoded_size(), 19);
        assert_eq!(decoded.dimensions(), (1024, 1024));
    }

    #[test]
    fn image_catalog_record_is_strict_and_bounded() {
        let unknown = br#"{"schemaVersion":1,"attachmentDigest":"0808080808080808080808080808080808080808080808080808080808080808","mediaType":"image/png","decodedSize":1,"width":1,"height":1,"extra":true}"#;
        assert!(serde_json::from_slice::<ImageAttachmentCatalogRecordV1>(unknown).is_err());

        let digest = sha256_attachment_digest(b"x");
        for (media_type, decoded_size, width, height) in [
            ("image/svg+xml", 1, 1, 1),
            ("image/png", 0, 1, 1),
            ("image/png", 1, 0, 1),
            ("image/png", 1, 4097, 1),
            ("image/png", 1, 4096, 4097),
        ] {
            assert!(ImageAttachmentCatalogRecordV1::new(
                digest,
                media_type.into(),
                decoded_size,
                width,
                height,
            )
            .is_err());
        }
        assert!(
            ImageAttachmentCatalogRecordV1::new(digest, "image/png".into(), 1, 4096, 4096,).is_ok()
        );
    }
}
