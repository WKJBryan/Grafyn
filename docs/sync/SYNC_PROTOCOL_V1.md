# Grafyn Sync Protocol v1

Status: public protocol foundation. This document defines interoperable envelope bytes and
verification behavior. It does not provide a relay, pairing, recovery, revocation, billing, or
sync engine.

The reference implementation is the transport-neutral Rust crate
`frontend/src-tauri/crates/grafyn-sync-protocol`. It has no filesystem, Tauri, keyring, network,
or relay dependency.

## Cryptographic suite

- Vault root key: 32 uniformly random bytes, obtained through a secure pairing/recovery flow.
- Device signing identity: Ed25519 seed and public key, 32 bytes each.
- Encryption: XChaCha20-Poly1305 with a 32-byte derived key, random 24-byte nonce, and the full
  16-byte tag appended to the ciphertext.
- Signatures: pure Ed25519 over the domain-separated signature document. Receivers use
  `verify_strict` and reject weak public keys.
- Subkeys: HKDF-SHA-256 with salt `grafyn.sync.hkdf.extract.v1`.
- Operation ID: HMAC-SHA-256 over the canonical operation-ID document. It is stable when the
  same device reseals the same operation, while a fresh random nonce produces new ciphertext.
- Binary hashes: SHA-256.

HKDF derives independent keys with these `purpose` values:

- `xchacha20poly1305-key`
- `hmac-sha256-operation-id-key`

The HKDF info document binds the purpose, vault ID, device ID, and device public key. This also
creates a separate XChaCha nonce domain for every device identity. A root key and signing seed
are secret values: they must not be serialized, logged, exported, sent to a relay, or placed in
application settings.

## Envelope JSON

An envelope is a strict JSON object matching `docs/sync/envelope-v1.schema.json`:

```json
{
  "protocol": "grafyn.sync.envelope",
  "schema_version": 1,
  "vault_id": "018f1f09-7b5a-7cc4-98c0-71acb24f24d3",
  "device_id": "018f1f0a-4050-7aca-aebe-16a510f897e8",
  "device_public_key": "base64url-no-pad-32-bytes",
  "operation_id": "64-lowercase-hex-characters",
  "nonce": "base64url-no-pad-24-bytes",
  "ciphertext": "base64url-no-pad-ciphertext-and-16-byte-tag",
  "signature": "base64url-no-pad-64-bytes"
}
```

UUIDs are lowercase, hyphenated, non-nil 16-byte UUID values. Hex is lowercase. Base64 uses the
RFC 4648 URL-safe alphabet without padding. A decoder must decode and re-encode these values and
require byte-for-byte equality; alternate padding or alphabets are not equivalent spellings.

JSON member order and insignificant whitespace do not affect verification. Missing, duplicate,
or unknown members fail closed. Untrusted input must enter through `EnvelopeV1::from_json`, which
applies the 2 MiB raw-JSON limit before parsing.

Only routing and cryptographic material appears outside the ciphertext. Payload type, causal
parents, recorded time, note/event identifiers, media type, attachment digest, chunk index, and
user content are encrypted.

## Canonical binary encoding

Canonical documents do not use JSON. There is no implicit string normalization.

```text
Frame(x)             = U64BE(byte_length(x)) || x
Field(name, value)   = Frame(UTF8(name)) || Frame(value)
Document(domain, fs) = Frame(ASCII(domain)) || Field(f1) || ... || Field(fn)
List(values)         = U32BE(item_count) || Frame(v1) || ... || Frame(vn)
```

- Integers use a fixed-width unsigned big-endian representation inside their field frame.
- IDs and digests use their raw bytes inside canonical documents, not JSON spellings.
- Strings use their exact UTF-8 bytes. Implementations must not trim, case-fold, or Unicode-
  normalize them.
- Field order is the order declared below. A different order is non-canonical.
- Set-like causal parents are sorted by their raw 32 bytes, are unique, and are rejected rather
  than silently normalized.
- Unknown fields, missing fields, length overflow, invalid UTF-8, invalid enum values, and
  trailing bytes fail closed.
- A decoded operation is re-encoded and required to equal the authenticated plaintext bytes.

Domain strings are protocol constants:

| Document | Domain |
|---|---|
| Operation | `grafyn.sync.operation.v1` |
| Note revision | `grafyn.sync.payload.note-revision.v1` |
| Twin event | `grafyn.sync.payload.twin-event.v1` |
| Attachment manifest | `grafyn.sync.payload.attachment-manifest.v1` |
| Attachment chunk | `grafyn.sync.payload.attachment-chunk.v1` |
| Operation-ID message | `grafyn.sync.operation-id.v1` |
| Envelope AAD | `grafyn.sync.envelope.aad.v1` |
| Signature message | `grafyn.sync.envelope.signature.v1` |
| HKDF info | `grafyn.sync.hkdf.info.v1` |

The operation fields, in order, are:

1. `recorded_at_unix_ms`: `U64BE`
2. `causal_parents`: `List(raw 32-byte operation IDs)`
3. `payload_type`: one of `note_revision`, `twin_event`, `attachment_manifest`, or
   `attachment_chunk`
4. `payload`: the complete canonical payload document

The wrapper payload type and nested payload domain must agree.

### Note revision

Fields: `note_id`, `revision_kind`, `markdown`.

- `note_id` is a stable opaque identifier, not a filesystem path: 1–256 UTF-8 bytes, no leading
  or trailing whitespace, and no control characters.
- `revision_kind` is `put` or `tombstone`.
- A put contains at most 1 MiB of UTF-8 Markdown. Empty Markdown is valid.
- A tombstone has an empty `markdown` field.

Concurrent note revisions retain all causal heads. Conflict materialization belongs to the sync
engine; the protocol does not merge Markdown.

### Twin event

Fields: `event_id`, `event_json`.

- `event_id` is the raw 32-byte canonical Twin event ID.
- `event_json` is a non-empty JSON object of at most 256 KiB.

The protocol checks the transport shape. Before append, the Grafyn sync engine must parse the
existing strict Twin-event schema and recompute its semantic event ID. Receiving authenticated
bytes is not authorization to bypass Twin governance.

### Attachment manifest

Fields: `attachment_digest`, `media_type`, `decoded_size`, `chunk_size`, `chunk_count`.

- `attachment_digest` is SHA-256 over the complete decoded attachment.
- `media_type` is 1–127 visible non-space ASCII bytes containing exactly one non-edge `/`.
- `decoded_size` is 1–25,165,824 bytes (24 MiB).
- `chunk_size` is exactly 262,144 bytes (256 KiB).
- `chunk_count` is exactly `ceil(decoded_size / chunk_size)`, from 1 through 96.

### Attachment chunk

Fields: `manifest_operation_id`, `attachment_digest`, `chunk_index`, `chunk_count`, `data`.

- Indices are zero-based and less than `chunk_count`.
- `chunk_count` is 1–96 and must equal the manifest.
- Every non-final chunk is exactly 256 KiB. The final chunk is 1–256 KiB.
- The chunk operation has exactly one causal parent: its `manifest_operation_id`.

A receiver may persist verified encrypted operations in a bounded staging area, but it must not
return or render attachment bytes until all indices are present exactly once, all metadata agrees,
every chunk was verified for the manifest's vault, the summed size equals the manifest, and the
ordered SHA-256 equals the manifest digest. Delivery order is irrelevant. Missing, duplicate, or
foreign chunks fail without partial output.

## Operation ID, AAD, and signature

The operation-ID message fields are `protocol`, `schema_version`, `vault_id`, `device_id`,
`device_public_key`, and canonical `operation`. The ID is:

```text
HMAC-SHA-256(K_operation_id, canonical_operation_id_message)
```

The nonce is intentionally excluded, so safe re-delivery of the exact persisted envelope and a
fresh reseal of the same operation retain the ID. The device binding is included, so two devices
authoring equivalent plaintext do not share an operation ID.

The envelope AAD fields are `protocol`, `schema_version`, `vault_id`, `device_id`,
`device_public_key`, `operation_id`, and `nonce`. The signature message fields are the complete
canonical `aad` and the complete `ciphertext` including its authentication tag.

The only public sealing API generates the nonce with the operating system CSPRNG. Callers cannot
supply a nonce. An implementation must persist and retransmit the already sealed envelope; it must
not reconstruct an envelope with a previously used nonce. Reusing a nonce with the same derived
AEAD key breaks XChaCha20-Poly1305 security.

## Strict receiver order

1. Enforce the 2 MiB raw JSON bound.
2. Reject unknown/duplicate/missing JSON fields, unsupported protocol/version, non-canonical
   UUID/hex/base64, invalid exact lengths, and ciphertext limits.
3. Compare vault ID, device ID, and device public key with a separately trusted device descriptor.
   Never trust a public key merely because it verifies the envelope that contains it.
4. Rebuild the AAD/signature document and run Ed25519 `verify_strict`.
5. Derive the per-device AEAD key and authenticate/decrypt into a zeroizing temporary buffer.
6. Strictly decode the canonical operation, validate the payload schema, reject trailing bytes,
   and require exact canonical re-encoding.
7. Derive the operation-ID key and verify the HMAC in constant time.
8. Apply causal dependency, duplicate/collision, local-only, governance, authorization, and
   materialization checks in the sync engine.

No plaintext is returned before steps 1–7 succeed. Signature-first rejection also avoids spending
decryption work on unsigned attacker traffic.

## Limits and compatibility

| Item | v1 limit |
|---|---:|
| Raw envelope JSON | 2 MiB |
| Decoded ciphertext plus tag | 1,100,016 bytes |
| Canonical plaintext operation | 1,100,000 bytes |
| Causal parents | 64 |
| Note Markdown | 1 MiB |
| Twin event JSON | 256 KiB |
| Attachment | 24 MiB |
| Attachment chunk | 256 KiB |
| Attachment chunks | 96 |

These limits are part of v1 interoperability. A v1 implementation must not accept a larger v1
message through a configuration switch. Relaxing a limit, changing a domain, adding a field or enum
variant, changing canonical order, or changing cryptographic inputs requires a new schema/protocol
version and new golden vectors. Unknown future values fail closed; downgrade fallback is not
automatic.

The committed vector is `frontend/src-tauri/crates/grafyn-sync-protocol/testdata/envelope-v1.json`.
It uses fixed test-only keys and nonce. Production has no fixed-nonce API.

## Primary references

- [RFC 8439: ChaCha20 and Poly1305](https://www.rfc-editor.org/rfc/rfc8439.html)
- [CFRG XChaCha draft](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha)
- [RFC 8032: Edwards-Curve Digital Signature Algorithm](https://www.rfc-editor.org/rfc/rfc8032.html)
- [RFC 5869: HKDF](https://www.rfc-editor.org/rfc/rfc5869.html)
- [RFC 4648: Base-N Encodings](https://www.rfc-editor.org/rfc/rfc4648.html)
- [`chacha20poly1305` 0.11.0 documentation](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/)
- [`ed25519-dalek` 3.0.0 `verify_strict`](https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/struct.VerifyingKey.html#method.verify_strict)
- [`hkdf` 0.13.0 documentation](https://docs.rs/hkdf/0.13.0/hkdf/)
- [`getrandom::fill` 0.4.3](https://docs.rs/getrandom/0.4.3/getrandom/fn.fill.html)
- [`zeroize::Zeroizing` 1.9.0](https://docs.rs/zeroize/1.9.0/zeroize/struct.Zeroizing.html)
- [`base64` general-purpose engines 0.23.1](https://docs.rs/base64/0.23.1/base64/engine/general_purpose/)
