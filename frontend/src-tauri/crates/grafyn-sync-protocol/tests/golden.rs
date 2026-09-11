use grafyn_sync_protocol::{
    open_operation, reassemble_attachment, seal_operation, AttachmentChunkV1, AttachmentManifestV1,
    DeviceId, DeviceSigningKey, Digest32, EnvelopeV1, NoteRevisionKind, NoteRevisionV1,
    OperationId, OperationPayloadV1, OperationV1, ProtocolError, TrustedDevice, TwinEventV1,
    VaultId, VaultRootKey, ATTACHMENT_CHUNK_BYTES, MAX_ATTACHMENT_BYTES, MAX_ENVELOPE_JSON_BYTES,
};

const GOLDEN_ENVELOPE: &str = include_str!("../testdata/envelope-v1.json");
const ENVELOPE_SCHEMA: &str = include_str!("../../../../../docs/sync/envelope-v1.schema.json");

fn golden_context() -> (VaultRootKey, VaultId, TrustedDevice) {
    let root_key = VaultRootKey::from_bytes([0x11; 32]);
    let vault_id = VaultId::parse_str("018f1f09-7b5a-7cc4-98c0-71acb24f24d3").unwrap();
    let device_id = DeviceId::parse_str("018f1f0a-4050-7aca-aebe-16a510f897e8").unwrap();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let trusted_device = TrustedDevice::new(device_id, signing_key.public_key()).unwrap();
    (root_key, vault_id, trusted_device)
}

fn note_operation() -> OperationV1 {
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

fn open_with_context(
    root_key: &VaultRootKey,
    vault_id: &VaultId,
    trusted_device: &TrustedDevice,
    envelope: &EnvelopeV1,
) -> grafyn_sync_protocol::VerifiedOperation {
    open_operation(root_key, vault_id, trusted_device, envelope).unwrap()
}

#[test]
fn fixed_vector_opens_to_the_expected_note_revision() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let envelope = EnvelopeV1::from_json(GOLDEN_ENVELOPE).unwrap();

    let verified = open_operation(&root_key, &vault_id, &trusted_device, &envelope).unwrap();

    assert_eq!(
        verified.operation_id().to_string(),
        "d649fc20616de1bbe76245faa67fc7d8828a55c8a5c5a0103176134c8b2320e0"
    );
    assert_eq!(
        verified.operation().recorded_at_unix_ms(),
        1_725_000_000_123
    );
    match verified.operation().payload() {
        OperationPayloadV1::NoteRevision(revision) => {
            assert_eq!(revision.note_id(), "note-golden");
            assert_eq!(
                revision.kind(),
                &NoteRevisionKind::Put {
                    markdown: "# Golden\n\nEncrypted sync vector.\n".to_owned(),
                }
            );
        }
        other => panic!("expected note revision, got {other:?}"),
    }
}

#[test]
fn json_field_order_does_not_change_the_verified_operation() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let value: serde_json::Value = serde_json::from_str(GOLDEN_ENVELOPE).unwrap();
    let reordered = serde_json::to_string(&value).unwrap();

    assert_ne!(reordered.trim(), GOLDEN_ENVELOPE.trim());
    let envelope = EnvelopeV1::from_json(&reordered).unwrap();
    let verified = open_operation(&root_key, &vault_id, &trusted_device, &envelope).unwrap();

    assert_eq!(
        verified.operation_id().to_string(),
        "d649fc20616de1bbe76245faa67fc7d8828a55c8a5c5a0103176134c8b2320e0"
    );
}

#[test]
fn independently_sealed_same_operation_keeps_id_but_changes_nonce_and_ciphertext() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let first = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &note_operation(),
    )
    .unwrap();
    let second = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &note_operation(),
    )
    .unwrap();

    assert_eq!(first.operation_id(), second.operation_id());
    assert_ne!(first.nonce(), second.nonce());
    assert_ne!(first.ciphertext(), second.ciphertext());
    open_with_context(&root_key, &vault_id, &trusted_device, &first);
    open_with_context(&root_key, &vault_id, &trusted_device, &second);
}

#[test]
fn envelope_metadata_and_crypto_tampering_fail_closed() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let envelope = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &note_operation(),
    )
    .unwrap();
    let original: serde_json::Value = serde_json::from_str(&envelope.to_json().unwrap()).unwrap();

    let cases: [(&str, serde_json::Value); 9] = [
        ("protocol", serde_json::json!("grafyn.sync.envelope.v2")),
        ("schema_version", serde_json::json!(2)),
        (
            "vault_id",
            serde_json::json!("018f1f09-7b5a-7cc4-98c0-71acb24f24d4"),
        ),
        (
            "device_id",
            serde_json::json!("018f1f0a-4050-7aca-aebe-16a510f897e9"),
        ),
        (
            "device_public_key",
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "nonce",
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "ciphertext",
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "signature",
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "operation_id",
            serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000"),
        ),
    ];

    for (field, replacement) in cases {
        let mut tampered = original.clone();
        tampered[field] = replacement;
        let json = serde_json::to_string(&tampered).unwrap();
        let result = EnvelopeV1::from_json(&json).and_then(|candidate| {
            open_operation(&root_key, &vault_id, &trusted_device, &candidate)
        });
        assert!(result.is_err(), "tampered {field} unexpectedly opened");
    }

    let wrong_root = VaultRootKey::from_bytes([0x12; 32]);
    assert!(open_operation(&wrong_root, &vault_id, &trusted_device, &envelope).is_err());
}

#[test]
fn envelope_json_rejects_unknown_duplicate_and_noncanonical_fields() {
    let duplicate = r#"{"protocol":"grafyn.sync.envelope","protocol":"grafyn.sync.envelope","schema_version":1,"vault_id":"018f1f09-7b5a-7cc4-98c0-71acb24f24d3","device_id":"018f1f0a-4050-7aca-aebe-16a510f897e8","device_public_key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","operation_id":"0000000000000000000000000000000000000000000000000000000000000000","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","ciphertext":"AAAAAAAAAAAAAAAAAAAAAA","signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    assert!(EnvelopeV1::from_json(duplicate).is_err());

    let mut unknown: serde_json::Value = serde_json::from_str(GOLDEN_ENVELOPE).unwrap();
    unknown["relay_hint"] = serde_json::json!("plaintext is forbidden");
    assert!(EnvelopeV1::from_json(&serde_json::to_string(&unknown).unwrap()).is_err());

    let mut padded: serde_json::Value = serde_json::from_str(GOLDEN_ENVELOPE).unwrap();
    let key = padded["device_public_key"].as_str().unwrap().to_owned();
    padded["device_public_key"] = serde_json::json!(format!("{key}="));
    assert!(EnvelopeV1::from_json(&serde_json::to_string(&padded).unwrap()).is_err());
}

#[test]
fn schema_base64_patterns_match_the_strict_parser() {
    const SIGNATURE_PATTERN: &str = r"^[A-Za-z0-9_-]{85}[AQgw]$";
    const CIPHERTEXT_PATTERN: &str =
        r"^(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-][AQgw]|[A-Za-z0-9_-]{2}[AEIMQUYcgkosw048])?$";

    let schema: serde_json::Value = serde_json::from_str(ENVELOPE_SCHEMA).unwrap();
    assert_eq!(
        schema["properties"]["signature"]["pattern"],
        SIGNATURE_PATTERN
    );
    assert_eq!(
        schema["properties"]["ciphertext"]["pattern"],
        CIPHERTEXT_PATTERN
    );

    let original: serde_json::Value = serde_json::from_str(GOLDEN_ENVELOPE).unwrap();
    let mut noncanonical_signature = original.clone();
    let signature = original["signature"].as_str().unwrap();
    noncanonical_signature["signature"] =
        serde_json::json!(format!("{}E", &signature[..signature.len() - 1]));
    assert!(
        EnvelopeV1::from_json(&serde_json::to_string(&noncanonical_signature).unwrap()).is_err()
    );

    let mut impossible_ciphertext_length = original;
    impossible_ciphertext_length["ciphertext"] = serde_json::json!("A".repeat(25));
    assert!(
        EnvelopeV1::from_json(&serde_json::to_string(&impossible_ciphertext_length).unwrap())
            .is_err()
    );
}

#[test]
fn envelope_json_size_and_weak_trust_roots_fail_before_crypto() {
    let oversized = " ".repeat(MAX_ENVELOPE_JSON_BYTES + 1);
    assert_eq!(
        EnvelopeV1::from_json(&oversized),
        Err(ProtocolError::JsonTooLarge)
    );

    let device_id = DeviceId::parse_str("018f1f0a-4050-7aca-aebe-16a510f897e8").unwrap();
    assert!(TrustedDevice::new(
        device_id,
        grafyn_sync_protocol::DevicePublicKey::from_bytes([0; 32])
    )
    .is_err());
}

#[test]
fn envelope_json_bytes_enforce_the_raw_boundary_before_parsing() {
    let golden = GOLDEN_ENVELOPE.trim().as_bytes();
    let mut exactly_at_limit = vec![b' '; MAX_ENVELOPE_JSON_BYTES - golden.len()];
    exactly_at_limit.extend_from_slice(golden);

    assert!(EnvelopeV1::from_json_bytes(&exactly_at_limit).is_ok());

    exactly_at_limit.insert(0, b' ');
    assert_eq!(
        EnvelopeV1::from_json_bytes(&exactly_at_limit),
        Err(ProtocolError::JsonTooLarge)
    );
    assert_eq!(
        EnvelopeV1::from_json_bytes(&[0xff]),
        Err(ProtocolError::InvalidJson)
    );
}

#[test]
fn tombstone_and_twin_event_payloads_round_trip_without_type_confusion() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let operations = [
        OperationV1::new(
            30,
            Vec::new(),
            OperationPayloadV1::NoteRevision(NoteRevisionV1::tombstone("note-deleted").unwrap()),
        )
        .unwrap(),
        OperationV1::new(
            31,
            Vec::new(),
            OperationPayloadV1::TwinEvent(
                TwinEventV1::new(
                    Digest32::from_bytes([0x77; 32]),
                    r#"{"event_type":"observation_recorded"}"#.to_owned(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    ];

    for expected in operations {
        let envelope = seal_operation(
            &root_key,
            &vault_id,
            trusted_device.device_id(),
            &signing_key,
            &expected,
        )
        .unwrap();
        let verified = open_with_context(&root_key, &vault_id, &trusted_device, &envelope);
        assert_eq!(verified.operation(), &expected);
    }
}

#[test]
fn secret_debug_output_is_redacted() {
    assert_eq!(
        format!("{:?}", VaultRootKey::from_bytes([0x11; 32])),
        "VaultRootKey([REDACTED])"
    );
    assert_eq!(
        format!("{:?}", DeviceSigningKey::from_seed([0x22; 32])),
        "DeviceSigningKey([REDACTED])"
    );
}

#[test]
fn plaintext_debug_output_is_metadata_only() {
    let private_note_id = "private-note-title";
    let private_markdown = "NEVER_LOG_PRIVATE_MARKDOWN";
    let revision = NoteRevisionV1::put(private_note_id, private_markdown.to_owned()).unwrap();
    let payload = OperationPayloadV1::NoteRevision(revision.clone());
    let operation = OperationV1::new(42, Vec::new(), payload.clone()).unwrap();

    for debug in [
        format!("{revision:?}"),
        format!("{payload:?}"),
        format!("{operation:?}"),
    ] {
        assert!(!debug.contains(private_note_id));
        assert!(!debug.contains(private_markdown));
        assert!(debug.contains("markdown_len"));
    }

    let private_event_json = r#"{"private":"NEVER_LOG_PRIVATE_TWIN_JSON"}"#;
    let event = TwinEventV1::new(
        Digest32::from_bytes([0x71; 32]),
        private_event_json.to_owned(),
    )
    .unwrap();
    let event_debug = format!("{event:?}");
    assert!(!event_debug.contains(private_event_json));
    assert!(event_debug.contains("event_json_len"));

    let chunk = AttachmentChunkV1::new(
        OperationId::from_bytes([0x72; 32]),
        Digest32::from_bytes([0x73; 32]),
        0,
        1,
        vec![251, 252, 253, 254],
    )
    .unwrap();
    let chunk_debug = format!("{chunk:?}");
    assert!(!chunk_debug.contains("data: [251, 252, 253, 254]"));
    assert!(chunk_debug.contains("data_len: 4"));

    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let envelope = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &operation,
    )
    .unwrap();
    let verified = open_with_context(&root_key, &vault_id, &trusted_device, &envelope);
    let verified_debug = format!("{verified:?}");
    assert!(!verified_debug.contains(private_note_id));
    assert!(!verified_debug.contains(private_markdown));
    assert!(verified_debug.contains("markdown_len"));
}

#[test]
fn envelope_debug_output_omits_wire_bytes() {
    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let envelope = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &note_operation(),
    )
    .unwrap();

    let debug = format!("{envelope:?}");
    assert!(!debug.contains("nonce: ["));
    assert!(!debug.contains("ciphertext: ["));
    assert!(!debug.contains("signature: ["));
    assert!(debug.contains(&format!("ciphertext_len: {}", envelope.ciphertext().len())));
}

#[test]
fn operation_requires_sorted_unique_causal_parents() {
    let low = OperationId::from_bytes([0x01; 32]);
    let high = OperationId::from_bytes([0x02; 32]);
    let payload = || {
        OperationPayloadV1::NoteRevision(NoteRevisionV1::tombstone("note-parent-order").unwrap())
    };

    assert!(OperationV1::new(1, vec![high, low], payload()).is_err());
    assert!(OperationV1::new(1, vec![low, low], payload()).is_err());
    assert!(OperationV1::new(1, vec![low, high], payload()).is_ok());
}

#[test]
fn attachment_layout_limits_are_enforced_before_encryption() {
    let digest = Digest32::from_bytes([0x33; 32]);
    let max_manifest =
        AttachmentManifestV1::new(digest, "image/png", MAX_ATTACHMENT_BYTES).unwrap();
    assert_eq!(max_manifest.chunk_count(), 96);
    assert!(AttachmentManifestV1::new(digest, "image/png", MAX_ATTACHMENT_BYTES + 1).is_err());
    assert!(AttachmentManifestV1::new(digest, "image/png", 0).is_err());

    assert!(AttachmentChunkV1::new(
        OperationId::from_bytes([0x44; 32]),
        digest,
        0,
        2,
        vec![0x55; ATTACHMENT_CHUNK_BYTES - 1],
    )
    .is_err());
    assert!(AttachmentChunkV1::new(
        OperationId::from_bytes([0x44; 32]),
        digest,
        2,
        2,
        vec![0x55],
    )
    .is_err());
    assert!(AttachmentChunkV1::new(
        OperationId::from_bytes([0x44; 32]),
        digest,
        0,
        97,
        vec![0x55; ATTACHMENT_CHUNK_BYTES],
    )
    .is_err());
}

#[test]
fn attachment_reassembly_accepts_reordering_only_after_complete_digest_verification() {
    use sha2::{Digest as _, Sha256};

    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);
    let mut bytes = vec![0x61; ATTACHMENT_CHUNK_BYTES];
    bytes.extend_from_slice(b"tail");
    let digest = Digest32::from_bytes(Sha256::digest(&bytes).into());
    let manifest_payload = AttachmentManifestV1::new(digest, "image/png", bytes.len()).unwrap();
    let manifest_operation = OperationV1::new(
        10,
        Vec::new(),
        OperationPayloadV1::AttachmentManifest(manifest_payload),
    )
    .unwrap();
    let manifest_envelope = seal_operation(
        &root_key,
        &vault_id,
        trusted_device.device_id(),
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let verified_manifest =
        open_with_context(&root_key, &vault_id, &trusted_device, &manifest_envelope);

    let mut verified_chunks = Vec::new();
    for (index, data) in [bytes[..ATTACHMENT_CHUNK_BYTES].to_vec(), b"tail".to_vec()]
        .into_iter()
        .enumerate()
    {
        let chunk = AttachmentChunkV1::new(manifest_id, digest, index as u32, 2, data).unwrap();
        let operation = OperationV1::new(
            11 + index as u64,
            vec![manifest_id],
            OperationPayloadV1::AttachmentChunk(chunk),
        )
        .unwrap();
        let envelope = seal_operation(
            &root_key,
            &vault_id,
            trusted_device.device_id(),
            &signing_key,
            &operation,
        )
        .unwrap();
        verified_chunks.push(open_with_context(
            &root_key,
            &vault_id,
            &trusted_device,
            &envelope,
        ));
    }

    let reversed = vec![verified_chunks[1].clone(), verified_chunks[0].clone()];
    assert_eq!(
        reassemble_attachment(&verified_manifest, &reversed).unwrap(),
        bytes
    );
    assert!(reassemble_attachment(&verified_manifest, &verified_chunks[..1]).is_err());
    assert!(reassemble_attachment(
        &verified_manifest,
        &[verified_chunks[0].clone(), verified_chunks[0].clone()]
    )
    .is_err());
}

#[test]
fn attachment_reassembly_rejects_foreign_metadata_total_size_and_full_digest() {
    use sha2::{Digest as _, Sha256};

    let (root_key, vault_id, trusted_device) = golden_context();
    let signing_key = DeviceSigningKey::from_seed([0x22; 32]);

    let open_payload = |payload: OperationPayloadV1, parents: Vec<OperationId>| {
        let operation = OperationV1::new(20, parents, payload).unwrap();
        let envelope = seal_operation(
            &root_key,
            &vault_id,
            trusted_device.device_id(),
            &signing_key,
            &operation,
        )
        .unwrap();
        open_with_context(&root_key, &vault_id, &trusted_device, &envelope)
    };

    let actual = b"four".to_vec();
    let actual_digest = Digest32::from_bytes(Sha256::digest(&actual).into());
    let wrong_digest = Digest32::from_bytes([0x99; 32]);

    let bad_digest_manifest = open_payload(
        OperationPayloadV1::AttachmentManifest(
            AttachmentManifestV1::new(wrong_digest, "image/png", actual.len()).unwrap(),
        ),
        Vec::new(),
    );
    let manifest_id = *bad_digest_manifest.operation_id();
    let bad_digest_chunk = open_payload(
        OperationPayloadV1::AttachmentChunk(
            AttachmentChunkV1::new(manifest_id, wrong_digest, 0, 1, actual.clone()).unwrap(),
        ),
        vec![manifest_id],
    );
    assert!(reassemble_attachment(&bad_digest_manifest, &[bad_digest_chunk]).is_err());

    let wrong_total_manifest = open_payload(
        OperationPayloadV1::AttachmentManifest(
            AttachmentManifestV1::new(actual_digest, "image/png", actual.len() + 1).unwrap(),
        ),
        Vec::new(),
    );
    let wrong_total_id = *wrong_total_manifest.operation_id();
    let wrong_total_chunk = open_payload(
        OperationPayloadV1::AttachmentChunk(
            AttachmentChunkV1::new(wrong_total_id, actual_digest, 0, 1, actual).unwrap(),
        ),
        vec![wrong_total_id],
    );
    assert!(reassemble_attachment(&wrong_total_manifest, &[wrong_total_chunk]).is_err());

    let foreign_manifest = open_payload(
        OperationPayloadV1::AttachmentManifest(
            AttachmentManifestV1::new(actual_digest, "image/png", 4).unwrap(),
        ),
        Vec::new(),
    );
    let foreign_id = *foreign_manifest.operation_id();
    let foreign_chunk = open_payload(
        OperationPayloadV1::AttachmentChunk(
            AttachmentChunkV1::new(foreign_id, actual_digest, 0, 1, b"four".to_vec()).unwrap(),
        ),
        vec![foreign_id],
    );
    assert!(reassemble_attachment(&bad_digest_manifest, &[foreign_chunk]).is_err());

    let same_vault_manifest = open_payload(
        OperationPayloadV1::AttachmentManifest(
            AttachmentManifestV1::new(actual_digest, "image/png", 4).unwrap(),
        ),
        Vec::new(),
    );
    let same_vault_manifest_id = *same_vault_manifest.operation_id();
    let other_vault = VaultId::parse_str("018f1f09-7b5a-7cc4-98c0-71acb24f24d4").unwrap();
    let other_vault_operation = OperationV1::new(
        21,
        vec![same_vault_manifest_id],
        OperationPayloadV1::AttachmentChunk(
            AttachmentChunkV1::new(
                same_vault_manifest_id,
                actual_digest,
                0,
                1,
                b"four".to_vec(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let other_vault_envelope = seal_operation(
        &root_key,
        &other_vault,
        trusted_device.device_id(),
        &signing_key,
        &other_vault_operation,
    )
    .unwrap();
    let other_vault_chunk = open_operation(
        &root_key,
        &other_vault,
        &trusted_device,
        &other_vault_envelope,
    )
    .unwrap();
    assert!(reassemble_attachment(&same_vault_manifest, &[other_vault_chunk]).is_err());
}
