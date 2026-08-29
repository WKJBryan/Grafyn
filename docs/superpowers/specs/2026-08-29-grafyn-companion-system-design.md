# Grafyn Companion System Design

**Date:** 2026-08-29
**Status:** Approved for implementation by the owner on 2026-08-29
**Decision owner:** Bryan
**Scope:** Open local client/core, Tauri 2 desktop and Android companion, Twin event spine, and an optional E2EE sync foundation

## Product thesis

Grafyn is the local-first data-collection and state layer for a person's digital twin. It is not a chat wrapper and it is not a model that claims to *be* the person. It observes what the person records and does, retains evidence with context and time, proposes patterns, lets the person review durable memory, and exposes deterministic state for recall, advice, simulation, and future model training.

The companion is the everyday sensor and feedback surface. Desktop and Android are peers with local vaults. Android uses an app-private vault; desktop preserves the user's selected Markdown vault, anchored by a stable vault descriptor rather than its path. Each remains useful offline. The implemented foundation will create and consume end-to-end encrypted operations locally. A future optional paid relay may move those opaque operations between devices, but no production relay or subscription is part of this delivery. The public repository contains the client, local core, protocol, and deterministic relay test harness. Production relay, billing, operations, and service abuse controls remain a separate proprietary service boundary.

## The simple biological picture

The biological language names responsibilities; it does not claim the software is biologically faithful.

- **Observation cells** preserve what happened. A capture, note edit, Canvas exchange, review action, decision outcome, or imported conversation becomes immutable evidence with time and context.
- **Dendritic cells** connect observations and propose meaning. They may identify a recurring preference, contradiction, relationship-specific behavior, or possible memory. They are proposal engines, never truth authorities.
- **B-memory cells** are durable, reviewed patterns. A proposal enters durable Twin memory only after an explicit review or another policy that is itself auditable. Repetition can raise attention but cannot silently make a claim authoritative.
- **Homeostatic attention** decides what matters for the current job. Recall, decision support, simulation, reflection, and capture review use different weight profiles. There is no universal importance score.
- **The organism state** is a deterministic projection of the event history. Different projections answer different questions without rewriting the evidence.

This architecture makes the current Twin RAG path more useful without turning Grafyn into an unverified foundational model. Learned models can propose and summarize; deterministic storage, review status, provenance, and projections remain authoritative.

## System boundaries

```text
Android companion ─┐                         ┌─ Optional opaque relay
                   ├─ local Grafyn core ─────┤  stores encrypted envelopes only
Desktop app ───────┘          │              └─ billing/operations (separate)
                              │
                              ├─ Markdown vault
                              ├─ append-only Twin events
                              ├─ deterministic projections
                              ├─ local search/graph/Canvas
                              └─ encrypted sync journal/outbox/inbox
```

### Open public boundary

- Vue/Tauri clients and shared Rust local core
- `grafyn-sync-protocol` transport-neutral protocol crate and schemas
- local storage, event spine, projections, attention profiles, export
- E2EE envelope creation/verification and deterministic in-memory relay harness
- interoperability and threat-model documentation

### Proprietary hosted boundary

- production relay deployment and scaling
- account, subscription, billing, quotas, abuse protection, support tooling
- push notification infrastructure and operational telemetry

A future conforming hosted service may receive only opaque, signed ciphertext plus the minimum routing metadata specified by the public protocol. The contract forbids it from receiving vault keys, plaintext notes, Twin state, OpenRouter credentials, or recovery phrases. This is a boundary for later implementation, not a claim that such a service is deployed.

## Licensing and governance

- The existing public client and local core are relicensed under **MPL-2.0** with the owner-authorized change recorded in the repository.
- New source-file modifications remain shareable at file scope while permitting a separately implemented hosted service.
- The sync protocol crate/spec is dual-noted as **Apache-2.0 OR MPL-2.0** where package tooling permits, to maximize independent implementations without changing the rest of the application's MPL terms.
- `CONTRIBUTING.md` uses inbound-equals-outbound and a Developer Certificate of Origin sign-off; no broad CLA is introduced.
- `TRADEMARKS.md` makes clear that code rights do not grant use of the Grafyn name or marks.
- Existing third-party notices remain governed by their original licenses. Relicensing is an owner representation about code they control, not a warranty about external contributions.

## Runtime architecture

### Tauri 2 shape

The Rust application becomes a library-backed Tauri 2 app:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() { /* builder and commands */ }
```

The desktop `main.rs` calls `grafyn_lib::run()`. Android invokes the mobile entry point. Platform-specific functionality is registered through capabilities and `cfg` gates rather than frontend user-agent checks.

Desktop retains:

- full spatial Canvas and debates
- native path-based import
- bundled MCP binary
- local Ollama
- desktop updater and release packaging
- vault migration and optimizer administration

Android v1 provides:

- app-owned offline vault
- quick capture and note editing
- full-screen recall and readable note detail
- Twin state/review/chat
- linear single-model Canvas
- quick image generation through OpenRouter when online and configured
- local sync queue and status

Android v1 does not expose path imports, MCP, Ollama, migration administration, the desktop updater, D3 spatial Canvas, or multi-model debates.

### Frontend platform contract

All direct Tauri access moves behind injected platform services:

```js
{
  runtime: 'desktop' | 'android' | 'web',
  formFactor: 'wide' | 'compact',
  capabilities: {
    notesRead, notesWrite, recall, twinReview, twinChat,
    linearCanvas, imageGeneration, sync,
    spatialCanvas, nativeVaultPicker, importByPath,
    localOllama, mcp, vaultMigration, optimizerAdmin, desktopUpdater
  }
}
```

`transport.invoke`, `transport.listen`, `transport.openExternal`, and `transport.showMainWindow` are the only frontend runtime bridges. Tauri on Android is still local IPC; it is not a remote desktop controller.

Responsive layouts use `100dvh`, safe-area insets, visible focus, minimum 44px touch targets, no hover-only actions, and a composer that remains usable above the software keyboard. Wide desktop routes preserve current behavior.

## Twin event spine

### Event envelope

Every governed event has a stable identifier, actor/device provenance, multiple time fields, context, evidence references, and a typed payload.

```rust
pub struct TwinEvent {
    pub schema_version: u16,
    pub event_id: EventId,
    pub event_type: TwinEventType,
    pub actor_id: ActorId,
    pub device_id: DeviceId,
    pub causal_stream: CausalStream,
    pub device_sequence: u64,
    pub causal_parents: Vec<EventId>,
    pub recorded_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub supersedes: Vec<EventId>,
    pub reinforces: Vec<EventId>,
    pub context: EventContext,
    pub evidence: Vec<EvidenceRef>,
    pub governance: Governance,
    pub payload: TwinEventPayload,
}

pub enum CausalStream { LocalOnly, SyncEligible }
```

Required event classes for this delivery:

- `ObservationRecorded`
- `NoteChanged`
- `ConversationTurnRecorded`
- `CanvasResponseRecorded`
- `MemoryProposed`
- `MemoryReviewed`
- `DecisionRecorded`
- `DecisionOutcomeRecorded`
- `FeedbackRecorded`
- `RelationshipContextObserved`

Events are append-only JSON records under the app data directory, not Markdown. Event IDs are content-addressed over a canonical representation. Duplicate IDs are no-ops; different content with the same ID fails closed. Local commands return only after the event and corresponding user mutation are durably staged according to the task-specific journal contract.

### Temporal model

One timestamp cannot describe a human state. Grafyn keeps:

- `recorded_at`: when the immutable event was committed
- `observed_at`: when Grafyn or the user observed/learned it
- `occurred_at`: when it happened, if known
- `valid_from` / `valid_to`: when a state claim applies
- `last_confirmed_at`: a derived memory field updated only by explicit reinforcing/confirming evidence
- `supersedes`: which prior proposal or reviewed memory it replaces
- `reinforces`: which prior claim it supports without replacing
- `causal_stream`: the explicit `LocalOnly` or conservatively `SyncEligible` causal lane
- `device_sequence`: a monotonic per-`(device_id, causal_stream)` sequence, chained through same-lane causal parents

The two lanes let sync omit local-only material without creating a device-sequence gap or a missing predecessor on a peer: each lane starts at 1 and later events directly cite the preceding event in that same lane. Cross-stream causal parents are invalid. `SyncEligible` is structural eligibility, not transport state: the event and every embedded relationship must have `SyncedVault` visibility, non-`Restricted` sensitivity, and `allowed_uses.sync = true`; `Sensitive` is allowed only when explicitly enabled. Every referenced event from a sync-eligible event must exist in the sync-eligible lane. A local-only event may retain non-causal references to either lane, but downstream resolution may only downgrade eligibility and never upgrade a failing baseline.

Projection code must not use wall-clock time as a conflict tiebreaker. Full and lane-filtered event sets are deterministically topologically ordered with all causal parents first; independent ready events are ordered by `event_id`. `device_sequence` validates a device-local lane but never orders independent devices. Decay is computed at query time from a declared reference time, so tests are deterministic even when delivery is reordered. Human-state time uses the explicit occurred/valid fields, not relay arrival order.

### Context and relationships

`EventContext` carries stable references and free tags:

```rust
pub struct EventContext {
    pub entities: Vec<ContextEntity>,
    pub relationships: Vec<RelationshipAssertion>,
    pub environments: Vec<String>,
    pub activities: Vec<String>,
    pub goals: Vec<String>,
    pub source_channel: SourceChannel,
    pub tags: Vec<String>,
}

pub struct ContextEntity {
    pub entity_id: EntityId,
    pub entity_type: EntityType,
    pub display_label: Option<String>,
    pub role_in_event: Option<String>,
}

pub struct RelationshipAssertion {
    pub subject_id: EntityId,
    pub predicate: String,
    pub object_id: EntityId,
    pub direction: RelationshipDirection,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub evidence: Vec<EvidenceRef>,
    pub governance: Governance,
}
```

The same person may behave differently with a manager, child, customer, friend, or alone. Projections therefore preserve relationship-conditioned variants instead of forcing every observation into one global trait. A global projection may summarize variants, but it must retain evidence counts and expose the conditions under which each variant was observed.

### Governance is orthogonal to attention

```rust
pub struct Governance {
    pub review: ReviewState,
    pub authority: AuthorityClass,
    pub sensitivity: Sensitivity,
    pub visibility: Visibility,
    pub allowed_uses: AllowedUses,
}

pub enum ReviewState { NotApplicable, Pending, Accepted, Rejected, Superseded }
pub enum AuthorityClass {
    EvidenceObservation,
    ReviewedMemory,
    CanonicalUserRule,
    DeterministicallyVerified { method: VerificationMethod },
}
pub enum VerificationMethod { HumanReview, SourceChecksum, SignedImport, RecordedOutcomeMatch }
pub enum Sensitivity { Standard, Sensitive, Restricted }
pub enum Visibility { LocalOnly, SyncedVault }

pub struct AllowedUses {
    pub recall: bool,
    pub twin_advisor: bool,
    pub twin_simulation: bool,
    pub export: bool,
    pub training: bool,
    pub sync: bool,
}
```

Review lifecycle, authority, sensitivity, and allowed uses are separate fields. Privacy/consent is a hard eligibility pass before scoring: material disallowed for Simulation, export, training, or sync never enters that candidate set, even with a maximal score. High recurrence, novelty, or relevance can raise attention only among eligible items. A proposal becomes reviewed memory only through an explicit review event. Deterministic verification requires a recorded method and cannot be inferred from model confidence.

A direct capture creates evidence that the user recorded something; it does not prove every claim inside the note. It receives `EvidenceObservation` authority. “Save as reviewed memory” is an explicit compound user action that records the observation and a separate review decision; ordinary quick capture never auto-promotes. The legacy `AutoPromoted` path must be migrated to pending proposals and must no longer enter authoritative context.

Default governance is explicit and versioned:

| Source/state | Recall | Advisor | Simulation | Export | Training | Sync |
|---|---:|---:|---:|---:|---:|---:|
| Direct observation | yes | evidence-only | no | yes | no | inherit visibility |
| Pending proposal | review surface only | no | no | no | no | inherit visibility |
| Accepted reviewed memory | yes | yes | yes | yes | no | inherit visibility |
| Canonical user rule | yes | yes | yes | yes | no | inherit visibility |
| Rejected/superseded | audit only | no | no | no | no | inherit visibility |

`Restricted` defaults to `LocalOnly` and all network/export uses false. `Sensitive` defaults to local recall/Advisor only; the user must explicitly enable each network/export use. `Standard` uses the table above. Training is always opt-in. Legacy `Private` becomes `Restricted + LocalOnly`; legacy `NoTrain` preserves its review state while setting `training = false`.

### Attention vectors and profiles

Each candidate receives an explainable vector:

```rust
pub struct AttentionVector {
    pub relevance: AttentionScore,
    pub confidence: AttentionScore,
    pub recency: AttentionScore,
    pub recurrence: AttentionScore,
    pub novelty: AttentionScore,
    pub relationship_match: AttentionScore,
    pub goal_match: AttentionScore,
    pub contradiction: AttentionScore,
}

pub struct AttentionScore(pub u16); // integer basis points: 0..=10_000
```

Named profiles are versioned data:

- **Recall:** relevance, confidence, recency
- **Decision:** confidence, goal and relationship match, contradiction
- **Simulation:** relationship match, recurrence, recency; reviewed-only by default
- **Reflection:** contradiction, novelty, recurrence
- **Capture review:** novelty, recurrence, confidence gap

Profiles use integer basis-point weights whose sum is 10,000. The scorer uses checked integer arithmetic and a documented rounding rule, then returns the component vector, profile weights, final score, and a short deterministic explanation. Raw scores are ranking aids, never truth labels. The UI can answer “why was this shown?” without invoking an LLM, and desktop/Android cannot drift because of floating-point serialization.

### Deterministic projections

The event log produces versioned read models:

- current reviewed memories
- pending dendritic proposals
- relationship-conditioned states
- temporal state timeline
- contradiction clusters
- recent observations
- attention-ranked recall candidates

Projection builds are pure over ordered events plus an explicit reference time. Rebuilding produces byte-equivalent canonical JSON. A projection cache is disposable and may always be regenerated from the event log.

Eligibility is evaluated before projection ranking in this order: allowed use, sensitivity/visibility, review and authority requirements, temporal validity, then attention. Every selection trace records excluded IDs and reason codes as well as selected IDs. No `HashMap` iteration order is allowed to affect output; stable IDs are the final tiebreaker.

### Promotion loop

```text
capture/action
  → immutable observation
  → proposal rule/model cites evidence
  → attention ranks it for the right review surface
  → owner accepts/rejects/edits
  → review event
  → B-memory projection updates
  → future recall/advice/simulation can use it
```

Failed or partial proposals remain useful evidence but cannot silently alter reviewed memory. This mirrors the useful part of biological immune memory: retain encounters, recognize patterns, and strengthen a response only through governed feedback.

## Capture hooks

The event spine records at shared service boundaries, not only in Vue components or Tauri commands, so MCP and future clients cannot bypass it.

- `KnowledgeStore` note mutations emit `NoteChanged`; mobile quick capture additionally emits `ObservationRecorded` with the new note as evidence.
- Canvas persistence emits prompt/response and feedback events after a successful session write.
- Twin review, Constitution, decisions, outcomes, and digest actions emit their governed event types.
- Imports emit one observation per imported conversation/container with provenance, not one event per every parser-internal step.
- A `MutationOrigin` prevents remote sync application and recovery replay from producing duplicate local events.

The existing index/optimizer fan-out remains a post-write concern. Event durability and sync journaling live at the underlying storage mutation boundary.

### Governed local mutation journal

Event capture is crash-coherent before sync is provisioned. A journal intent contains a canonical mutation ID, typed target, before digest, bounded desired after-image or tombstone, and the complete governed event group. The coordinator:

1. atomically stages the intent;
2. applies the user mutation idempotently;
3. appends the immutable event group idempotently;
4. removes the intent only after both are durable;
5. then runs disposable index/optimizer fan-out.

On restart, a target matching the before digest is advanced to the desired state; a target matching the after digest needs only the event append; an unknown third digest is quarantined as a recoverable conflict and boot reports it instead of overwriting newer user data. Injected crash tests cover each phase. The later encrypted sync journal wraps the same canonical operation IDs; it does not replace this local durability boundary.

## Twin chat

Twin chat remains RAG over local state and reviewed evidence. It does not fine-tune a model and does not persist hidden model state.

A new `TwinHistory` context mode combines compact parent conversation history with the existing Twin operating contract. Advisor remains the default. Simulation still requires identity setup, preserves its disclosure in the visible UI, and retrieves reviewed memories by default. The prompt records which projection snapshot and evidence IDs were used so an exported trace can be reproduced.

## E2EE sync foundation

### Identity and keys

- Each vault owns a stable UUID descriptor at `vault/_grafyn/vault.json`; moving the directory does not change identity.
- Each device owns an Ed25519 signing identity.
- Each synced vault owns a random 32-byte root key shared through a future pairing/recovery flow.
- Desktop secrets use the existing OS keyring through a `SecretStore` trait.
- Android secrets use a secure mobile adapter. No release build may fall back to plaintext settings or a compiled password.
- Pairing UI, rotation, revocation, recovery export, subscription purchase, and production relay deployment are outside this foundation and must be labelled unavailable rather than faked.

### Synced operations

Markdown notes use immutable whole-file revisions with causal parents. Concurrent edits retain all heads and deterministically materialize one; no automatic Markdown merge occurs. Tombstones sort above puts only for deterministic materialization, without deleting losing history.

Vault sync is opt-in. Once enabled, ordinary Markdown notes default to `grafyn_sync: inherit` and follow the vault's synced-device policy; `grafyn_sync: local_only` is a hard per-note exclusion recorded in frontmatter and honored before envelope creation. `_grafyn/program.md`, secrets, indexes, and app settings remain local-only. Observation events referencing a local-only note are also local-only unless the user explicitly creates a separate redacted eligible memory. A remote revision cannot change a local-only note's policy without an explicit locally authorized policy operation.

Twin events sync as immutable event operations. Because their IDs are content-addressed and their semantics append-only, duplicates are no-ops. Projection caches, search indexes, settings, API keys, MCP configuration, Ollama configuration, and transient UI state never sync.

Attachment manifests/chunks sync as bounded immutable operations. The first protocol version caps an attachment at 24 MiB decoded, uses 256 KiB plaintext chunks before envelope overhead, requires at most 96 chunks, and verifies the attachment SHA-256 after ordered reassembly. The same bytes deduplicate by digest across notes/events. A receiver materializes a blob only after every chunk and the full digest verify; partial chunks are never rendered. EXIF is stripped by default for newly generated/imported companion images unless the user explicitly elects to retain it before save.

### Envelope

Payloads are encrypted with XChaCha20-Poly1305, signed with Ed25519, and domain-separated. The receiver verifies device/public-key identity, signature, ciphertext authentication, canonical operation ID, vault ID, schema version, and causal dependencies before materialization.

The relay can observe vault/device routing identifiers, envelope size, counts, and timing. The threat model states this metadata leakage clearly. “End-to-end encrypted” does not mean metadata-private.

### Crash order

```text
1. stage signed encrypted operation in journal
2. commit local note/event mutation atomically
3. promote journal entry to immutable outbox
4. update disposable derived state
5. recover any journal entry at next startup
```

Known mutation failure cancels the staged journal record. Remote and recovery origins never create a new outbox operation. A deterministic two-device in-memory relay verifies reorder, duplicate, conflict, delete, event, and echo-suppression behavior.

## Companion information architecture

Bottom navigation has four destinations:

1. **Capture** — an immediate text composer, recent captures, and optional image generation action
2. **Recall** — full-screen query, attention reason, filters, and readable note/state detail
3. **Twin** — advisor chat, pending review, reviewed memory, and state timeline
4. **Canvas** — chronological one-model threads with Knowledge/Twin/plain context

Settings is a compact sheet. It exposes theme, local vault status, OpenRouter configuration, sync status, and secure-key/pairing state only when the runtime capability exists. Desktop-only controls never render on Android.

Image generation is deliberately one-shot in v1: prompt, supported model, square output, preview, save/share. Generated media is stored as a local asset with a metadata note and observation event only after explicit save. No video generation or media gallery is included in this delivery; the same governed asset-event contract makes those a later addition.

Saved media uses a content-addressed attachment record: digest, exact MIME, decoded byte size, dimensions, source, created time, optional annotation, and an explicit EXIF-retention decision. Original bytes live in an app-owned attachment directory; thumbnails are disposable projections. Generated previews are transient until save. Sync transports a signed encrypted manifest plus content-addressed chunks; the relay learns only encrypted chunk size/count/timing and routing metadata.

## Data and privacy rules

- Offline use is the default; network access is explicit per feature.
- Subscription or relay availability never gates local capture, recall, decryption, backup, export, or access to already local data.
- OpenRouter requests are user-initiated and include only the selected prompt/context.
- Sync is opt-in and disabled until a vault key exists.
- Event exports include schema versions, evidence IDs, governance fields, time, context, attention explanations, and projection snapshot IDs, subject to the export eligibility gate.
- Secrets are marked non-serializable and are excluded from logs, bug reports, exports, and sync.
- Corrupt identity, event, journal, or envelope files fail closed and are quarantined where recovery is safe. Grafyn never silently rotates a vault ID or key.

## Success scenario

The release-level proof is one deterministic journey:

1. On an Android-sized companion, generate and explicitly save one image while online, then block network access.
2. Create an offline capture about a context-specific preference and annotate the saved attachment.
3. The local vault contains the note, validated attachment bytes, and an observation event with time, relationship context, and evidence references.
4. After restart, Recall finds it and displays the attention-vector explanation for the Recall profile.
5. A dendritic proposal is created from the observation, but Simulation cannot use it as reviewed truth.
6. Accepting it creates a review event and updates the B-memory projection.
7. Twin Advisor chat uses compact conversation history plus the reviewed projection and records the evidence snapshot.
8. Device A seals its note revision, attachment chunks, and Twin events; a deterministic opaque relay delivers them reordered and duplicated to Device B.
9. Device B converges to the same note and attachment bytes, event set, heads, and projection; it produces no echo operations.
10. Wide desktop still supports the existing note, graph, spatial Canvas, import, MCP, updater, and Ollama surfaces.

The app must fail into a recoverable boot state if canonical vault identity, event storage, Markdown/attachment bytes, or operation journals cannot be initialized. Reviewed memory is derived solely from accepted review events and is rebuilt with the other projections. Search, chunk, graph, thumbnail, and projection caches may be recreated; canonical operations and user bytes are never treated as disposable.

## Explicit non-goals

- training or fine-tuning a new foundational/world model inside Grafyn
- claiming the Twin is conscious, biologically equivalent, or an identity replacement
- automatic promotion from recurrence or model confidence to verified memory
- a global “importance” score
- production billing, relay deployment, pairing/recovery/revocation, or store publication
- Android path imports, local Ollama, MCP, spatial Canvas, debates, or optimizer administration
- iOS build verification on Windows
- video generation in this delivery
- in-app accuracy benchmarking or scoring dashboards; capture and export remain Grafyn's boundary

## Verification strategy: zoom in and zoom out

**Zoomed in**

- pure canonicalization, attention, projection, causality, crypto, and capability tests
- storage crash/recovery and remote-origin integration tests
- Vue component tests for capture, recall, review, chat, Canvas, settings, and unavailable capabilities
- Rust feature builds for desktop, MCP, and mobile-compatible core

**Zoomed out**

- full frontend and Rust suites
- desktop Tauri build and smoke with existing features
- Android debug build and emulator/device smoke when the SDK is available
- Pixel-sized end-to-end companion journey
- deterministic two-device convergence scenario
- secret scanning, advisory/license checks, capability audit, and final code review

If Android tooling cannot be installed or used on this Windows host, the implementation still must compile the mobile-compatible Rust core and pass browser-sized companion E2E; the missing APK/emulator proof is reported as an external verification boundary, not silently treated as success.

## Invariants reviewers must defend

1. Privacy and allowed-use gates run before attention; a high score never changes review or authority.
2. Models propose; explicit review events and deterministic verification govern durable state.
3. Auto-promoted legacy records do not count as reviewed B-memory.
4. Relationship-conditioned behavior is retained, not averaged away without evidence.
5. Expired and superseded material is excluded by deterministic temporal rules.
6. Projection rebuilds are deterministic for the same events and reference time.
7. Local and remote replay cannot create duplicate events or sync echoes.
8. Sync relay code cannot access plaintext or vault keys.
9. Android is a local peer, not a remote-control client for the desktop.
10. Desktop capabilities do not appear on Android merely because both expose Tauri IPC.
11. No secret reaches settings JSON, logs, export bundles, or protocol payloads.
12. Canonical-store initialization fails closed; only derived indexes are disposable.
13. Existing wide-desktop behavior remains covered while the compact companion is added.
