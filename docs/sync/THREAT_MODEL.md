# Grafyn Sync Foundation Threat Model

## Scope

This threat model covers the public v1 envelope crate and the boundary a future local sync engine
must enforce around it. No production relay, account service, pairing/recovery flow, key rotation,
device revocation, subscription system, or remote deletion service is implemented by this task.

The security goal is confidentiality and authenticated authorship of eligible immutable Grafyn
operations between devices that already share a vault root key and have an independently trusted
device-ID/public-key binding.

## Assets

- Vault root key and derived encryption/operation-ID keys.
- Device Ed25519 signing seed.
- Note revisions, Twin events, attachment manifests, and attachment chunks.
- The integrity of vault/device identity, operation IDs, causal history, and attachment bytes.
- Governance: local-only and disallowed material must never reach sealing or an outbox.

Secret owners must store root/signing keys in the platform secure-secret adapter. Secret wrappers
are non-serializable, redact `Debug`, and zeroize owned key bytes on drop. Application logs, crash
reports, exports, settings, envelope JSON, and relay records must not contain them.

## Adversaries

The design assumes an attacker may:

- operate or compromise the relay and store, drop, delay, reorder, duplicate, or replay envelopes;
- edit any envelope JSON member or ciphertext byte;
- send malformed, oversized, non-canonical, or cryptographically invalid traffic;
- know public device keys and routing IDs;
- obtain a valid envelope and deliver it to a different vault/device context;
- try to exhaust receiver CPU, memory, staging space, or attachment storage.

The protocol does not protect a device after its vault root key and signing seed are both
compromised. Revocation and forward secrecy require future protocol work. A compromised authorized
device can author valid operations; semantic governance and user review remain necessary.

## Relay-visible metadata

The relay and a network observer can learn:

- protocol and schema version;
- vault and device routing UUIDs;
- device public key;
- stable per-device operation ID;
- nonce, ciphertext length, and signature;
- message counts, upload/download timing, retry patterns, IP/account/network metadata, and likely
  attachment chunk groupings from fixed-size traffic.

The relay cannot read the encrypted payload type, causal parent list, recorded time, note/event
identifier, media type, attachment digest/index, or content. This is end-to-end encryption, not
metadata privacy, anonymity, traffic-flow confidentiality, or deniability. Padding, batching,
private information retrieval, and anonymous routing are out of v1 scope.

## Defenses

- Strict bounded parsing runs before public-key or symmetric crypto.
- Vault/device/public-key equality uses a separately trusted descriptor before signature work.
- `verify_strict` rejects Ed25519 malleability/weak-key cases before decryption.
- AEAD AAD binds protocol, version, vault, device, public key, operation ID, and nonce.
- The signature covers the complete AAD and ciphertext/tag.
- HKDF creates domain- and device-separated AEAD and operation-ID subkeys.
- Canonical, domain-separated length-prefixed bytes prevent JSON member order and concatenation
  ambiguity.
- HMAC operation-ID verification is constant-time and prevents an untrusted relay from computing
  plaintext guesses without the vault key.
- The production sealing API obtains a fresh 192-bit nonce from the operating system CSPRNG and
  exposes no caller-controlled nonce.
- Attachment layout is bounded before allocation; bytes are materialized only after completeness,
  size, and full SHA-256 verification.

## Replay, causality, and authorization

Cryptographic verification proves that bytes match a trusted author and vault. It does not prove
that an operation is new, causally ready, authorized by current policy, or safe to materialize.

A sync engine must, in this order after `open_operation` succeeds:

1. Look up the operation ID in an immutable operation store.
2. Treat an exact duplicate as a no-op and quarantine an ID/content collision.
3. Require every causal dependency to exist and remain in the same eligible causal lane.
4. Re-run note-local-only, Twin visibility/sensitivity/allowed-use, and local authorization rules.
5. Apply remote/recovery origin without producing a new outbox operation.
6. Materialize through Grafyn's atomic storage mutation boundary.

The engine must cap pending dependency count, per-peer envelope count/bytes, attachment staging
bytes, retry rate, and retention time. The v1 crate caps each object but cannot cap an unbounded
stream of individually valid objects. Relay quotas are an availability control, not a
confidentiality guarantee.

The current manual-transport foundation enforces hard operation, byte, per-device pending,
attachment, staging, and quarantine bounds and fails closed when one is reached. It has no relay,
acknowledgement protocol, unattended retry scheduler, or time-based ciphertext retention yet, so it
does not claim retry-rate control or automatic expiry. Those controls belong to the later hosted
relay/ack design; until then, bounded manual export/import is the only supported transport.

## Non-goals and residual risks

- No protection from endpoint malware, unlocked-device access, screenshots, or user-authorized
  exports.
- No forward secrecy or post-compromise security in v1.
- No device revocation, root-key rotation, recovery escrow, or key-loss recovery in this crate.
- No automatic conflict resolution for Markdown and no semantic validation of Grafyn Twin events;
  those belong to the local engine and existing strict event model.
- No guarantee that a relay deletes stored ciphertext.
- SHA-256 attachment digests allow a key-holding peer to recognize equal plaintext attachments;
  the relay cannot see those encrypted digests.
- Random 192-bit nonces make accidental collision negligible, but implementations must still
  persist the exact sealed envelope and must never intentionally reuse a nonce with the same key.

Security claims must not expand when later hosted services are added. A relay remains plaintext-
blind, local access remains available without a subscription, and local-only/governance gates run
before envelope creation.
