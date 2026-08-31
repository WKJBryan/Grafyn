# Grafyn Companion-First Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Use superpowers:test-driven-development for every behavior change, superpowers:systematic-debugging for unexpected failures, and superpowers:verification-before-completion before any completion claim.

**Goal:** Turn Grafyn into a verified local-first desktop/Android companion and data-collection layer for digital twins, with a bio-inspired governed event spine, contextual temporal state, explainable per-use-case attention, an open E2EE sync foundation, and no regression to the wide desktop app.

**Architecture:** Migrate the desktop shell to Tauri 2 and a mobile-compatible library entry point; move runtime access behind capability-aware adapters; record append-only governed Twin events at shared Rust service boundaries; derive deterministic attention/state projections; synchronize whole-note revisions and immutable Twin events through a public E2EE protocol; then add compact companion views over the same local core. Models may propose patterns, but review/verification authority remains separate from attention and deterministic state.

**Tech Stack:** Rust 2021, Tauri 2.11, Vue 3, Pinia, Vitest, Playwright, Tantivy, petgraph, XChaCha20-Poly1305, Ed25519, HKDF/HMAC/SHA-256, Android WebView.

**Design authority:** `docs/superpowers/specs/2026-08-29-grafyn-companion-system-design.md`. If this plan and the design conflict, stop the task, record the conflict in the SDD ledger, and resolve the documents before code.

## Global execution rules

- Work only on `codex/companion-first` in `.worktrees/companion-first`.
- One task at a time. One implementer, then task contract review, then quality review when the SDD workflow calls for it.
- Test first. Run the named focused RED test and observe the expected failure before production code.
- Every source change must trace to this plan. Do not refactor adjacent code.
- Preserve the existing external Twin-evaluation boundary: capture/export only, no in-app accuracy dashboard.
- Keep desktop and Android local-first. Do not introduce a remote-control transport to the desktop.
- Never serialize secrets into settings, events, exports, logs, or relay payloads.
- Keep review lifecycle, authority class, sensitivity, and allowed uses as orthogonal data. Apply privacy/use gates before attention; never let score, recurrence, or an LLM response promote a proposal.
- Use explicit reference times and deterministic ordering in temporal/projection tests.
- Update `CLAUDE.md` in the task that changes the corresponding live architecture.
- Do not publish packages, deploy a relay, push branches, open PRs, merge, or release without a separate explicit request.

## Baseline evidence

Before this plan was written:

- `npm run build` passed.
- `npm run lint` exited zero with seven pre-existing Vue warnings.
- `npm run test:run` had six failures, all in `TwinEvalLab.spec.js`, because the template omitted controls already required by the committed test contract.
- `npm run prepare:sidecar` failed in `services/ollama.rs` because one `OllamaOptions` initializer was not updated when optional generation fields were introduced.
- Networked dependency installation required an approved unsandboxed `npm ci`; local cache and node modules are present in this worktree.

Task 1 repairs these baselines before migration. Later tasks must not redefine them as acceptable.

---

## Task 1: Restore a green pre-migration baseline

**Files:**

- Modify: `frontend/src/lab/TwinEvalLab.vue`
- Modify: `frontend/src-tauri/src/services/ollama.rs`
- Test: `frontend/src/__tests__/unit/lab/TwinEvalLab.spec.js`
- Test: existing inline tests in `frontend/src-tauri/src/services/ollama.rs`

**Contract:** The isolated lab exposes the controls its existing tests specify; the Ollama non-streaming request uses the same complete options shape as streaming.

- [ ] Run `npx vitest run src/__tests__/unit/lab/TwinEvalLab.spec.js` and confirm six expected failures.
- [ ] Read every failing assertion and add only the missing template controls using existing component state/methods: `preview-input`, `context-mode`, `show-trace`, `structured-output`, `system-prompt`, and `export-json`. Do not add a normal app route to the lab.
- [ ] Run the focused Vitest file and confirm green.
- [ ] Run `npm run prepare:sidecar` and confirm the `OllamaOptions` compile error.
- [ ] In `chat`, set `temperature: Some(temperature)`, `top_p: None`, `num_predict: None`, and `stop: None`, matching `chat_stream`.
- [ ] Run `npm run prepare:sidecar`, then `cargo test services::ollama --locked` from `frontend/src-tauri`.
- [ ] Run `npm run test:run` and record the new baseline counts.
- [ ] Commit: `fix: restore green Twin lab and Ollama baseline`

## Task 2: Align repository governance with the approved open-client model

**Files:**

- Replace: `LICENSE`
- Create: `LICENSES/MPL-2.0.txt`
- Create: `LICENSES/Apache-2.0.txt`
- Create: `CONTRIBUTING.md`
- Create: `TRADEMARKS.md`
- Create: `RELICENSING.md`
- Modify: `README.md`
- Modify: `frontend/package.json`
- Modify: `frontend/package-lock.json`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `e2e/package.json`
- Modify: `e2e/package-lock.json`

**Contract:** The client/core package metadata consistently declares MPL-2.0 only after a file/commit provenance audit supports the owner's authorization; protocol-package metadata may declare `Apache-2.0 OR MPL-2.0`; contributor/trademark boundaries are explicit. No source copyright is claimed from third parties.

- [ ] Add a failing repository metadata test at `frontend/scripts/check-license-alignment.cjs` and expose `npm run check:licenses`. It must inspect the files above and fail against the current inconsistent grants: AGPL text in root `LICENSE`, GPL-3.0-only Rust/frontend metadata, and ISC E2E metadata.
- [ ] Audit all non-merge commits and changed files by author identity, distinguish merge-only/tree-identical commits, and inspect vendored/copied source notices. Record evidence and the owner's 2026-08-29 authorization in `RELICENSING.md`. Git identity is evidence, not legal proof; if independently authored code without relicensing consent is found, keep that material under its existing license or replace it cleanly instead of silently relicensing it.
- [ ] Only after the audit gate passes, replace the root license with the unmodified official MPL-2.0 text and retain official Apache-2.0 text for the protocol package.
- [ ] Align package metadata and lockfiles. `RELICENSING.md` records the date, previous public license, owner authorization, and the fact that earlier recipients retain their prior grants.
- [ ] `CONTRIBUTING.md` uses DCO 1.1 sign-off and inbound-equals-outbound. `TRADEMARKS.md` reserves the Grafyn name/logo without restricting forks from describing compatibility truthfully.
- [ ] Update the README license/edition language without claiming the future hosted service is already available.
- [ ] Run `npm run check:licenses`, `npm run release:verify`, and `cargo metadata --locked --no-deps`.
- [ ] Commit: `docs: adopt MPL-2.0 open-client governance`

## Task 3: Migrate the desktop shell to Tauri 2 without behavior loss

**Files:**

- Create: `frontend/src-tauri/src/lib.rs`
- Create: `frontend/src-tauri/capabilities/desktop.json`
- Create: `frontend/src-tauri/tauri.desktop.conf.json`
- Modify: `frontend/src-tauri/src/main.rs`
- Modify: `frontend/src-tauri/src/commands/settings.rs`
- Modify: `frontend/src-tauri/src/commands/twin_eval.rs`
- Modify: `frontend/src-tauri/src/services/twin_eval.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `frontend/src-tauri/tauri.conf.json`
- Modify: `frontend/src-tauri/tauri.lab.conf.json`
- Modify: `frontend/package.json`
- Modify: `frontend/package-lock.json`
- Modify: `frontend/src/main.js`
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/views/ImportView.vue`
- Modify: `frontend/src/api/client.js`
- Modify: frontend Tauri mocks under `frontend/src/__tests__/`
- Modify: `frontend/scripts/run-tauri.cjs`
- Modify: `frontend/scripts/prepare-sidecar.cjs`
- Modify: `frontend/vite.config.js`
- Modify: `.github/workflows/test.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/release-smoke.yml`
- Modify: `.github/workflows/latest-deps.yml`
- Modify: `CLAUDE.md`

**Version pins:** Tauri `2.11.5`, CLI `2.11.4`, JS API `2.11.1`, `tauri-build 2.6.3`, dialog `2.7.2`, opener `2.5.4`, updater `2.10.1`. Re-check compatible latest patch versions only if resolution proves one pin unavailable; document any deviation.

**Contract:** Wide desktop starts, shows its initially hidden main window after frontend mount, opens external links, picks a vault folder, streams events, bundles the MCP binary, and retains its updater origin/storage across the v1→v2 transition.

- [ ] Add/update frontend mocks first so imports from `@tauri-apps/api/core`, `@tauri-apps/api/webviewWindow`, `@tauri-apps/plugin-dialog`, `@tauri-apps/plugin-opener`, and `@tauri-apps/plugin-updater` are expected; observe focused failures.
- [ ] Upgrade dependencies. Add `[lib] name = "grafyn_lib"` with `crate-type = ["staticlib", "cdylib", "rlib"]` and move builder/state wiring into `run()` in `lib.rs`; keep desktop `main.rs` as the thin binary entry.
- [ ] Convert `emit_all` to `Emitter::emit`, `Window` to `WebviewWindow`, and the folder picker to `tauri-plugin-dialog`.
- [ ] Convert config to Tauri 2: `frontendDist`, `devUrl`, top-level product/version/identifier, `app.windows`, capabilities instead of allowlist, `mainBinaryName`, `useHttpsScheme: true`, desktop-only external binary/updater, and `createUpdaterArtifacts: "v1Compatible"`.
- [ ] Convert frontend APIs to core/webviewWindow/dialog/opener/updater plugins. Implement an explicit desktop updater check/install flow because the v1 built-in dialog no longer exists.
- [ ] Add a non-default `twin-eval-lab` Cargo/Vite build feature. Default desktop and every Android build omit the lab HTML and Twin-eval commands; lab scripts opt in explicitly. Extend `twinEvalLabIsolation.spec.js` and build checks to prove normal routes, assets, capabilities, and invoke registration cannot reach scoring/export commands.
- [ ] Update Linux prerequisites to WebKitGTK 4.1, current tauri-action, Android-neutral build scripts, and release artifact expectations.
- [ ] Regenerate lockfiles. Run focused API/store tests, `npm run test:run`, `npm run build`, `npm run prepare:sidecar`, `cargo test --locked`, and `cargo test --locked --no-default-features --features mcp`.
- [ ] Run `npm run tauri:build`; inspect the produced desktop bundle and MCP external binary.
- [ ] Update `CLAUDE.md` from Tauri v1/desktop-only facts to Tauri 2 shared-core facts, keeping mobile features marked unavailable until their tasks land.
- [ ] Commit: `refactor: migrate Grafyn desktop shell to Tauri 2`

## Task 4: Introduce the runtime capability and transport seams

**Files:**

- Create: `frontend/src/platform/runtime.js`
- Create: `frontend/src/platform/capabilities.js`
- Create: `frontend/src/api/transport.js`
- Create: `frontend/src/api/tauriTransport.js`
- Create: `frontend/src/__tests__/unit/platform/runtime.spec.js`
- Create: `frontend/src/__tests__/unit/api/transport.spec.js`
- Modify: `frontend/src/api/client.js`
- Modify: `frontend/src/stores/boot.js`
- Modify: `frontend/src/stores/canvas.js`
- Modify: `frontend/src/main.js`
- Modify: `frontend/src/App.vue`

**Contract:** Frontend feature access comes from an explicit runtime/capability matrix. Android Tauri IPC never implies desktop permissions. All invoke/listen/open/show operations are injectable in tests.

- [ ] Write failing tests for desktop-wide, Android-compact, and plain-web profiles; Android must lack MCP, Ollama, path import, migration, optimizer, updater, and spatial Canvas.
- [ ] Write failing injection tests proving the API client and event stores work through a fake transport without importing Tauri modules directly.
- [ ] Implement the minimum modules and preserve every existing API namespace/signature.
- [ ] Add an `assertCapability(name)` helper that fails closed with a typed unavailable error used by routes/components later.
- [ ] Run focused tests, `rg "@tauri-apps" frontend/src` and verify only platform adapters plus deliberately desktop-only dynamic plugin modules remain.
- [ ] Run full frontend tests/build.
- [ ] Commit: `refactor: isolate runtime capabilities and Tauri transport`

## Task 5: Create the canonical append-only Twin event store

**Files:**

- Create: `frontend/src-tauri/src/models/twin_event.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mod.rs`
- Create: `frontend/src-tauri/src/services/twin_events/canonical.rs`
- Create: `frontend/src-tauri/src/services/twin_events/store.rs`
- Create: `frontend/src-tauri/src/services/twin_events/test_support.rs`
- Modify: `frontend/src-tauri/src/models/mod.rs`
- Modify: `frontend/src-tauri/src/services/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `CLAUDE.md`

**Contract:** `TwinEventStore::append` atomically persists canonical append-only events, treats an identical duplicate as a no-op, rejects ID/content collision, quarantines malformed records, and returns events in deterministic order. IDs never depend on JSON object key order. Failure to initialize canonical storage yields a recoverable boot error instead of continuing with disposable/default state.

- [ ] Write failing tests: canonical field-order independence; duplicate no-op; collision reject; corrupt quarantine; causal-parent-before-child ordering; independent cross-device arrival-order independence using stable event-ID ties; invalid/reused device sequence; required recorded/observed/occurred/valid/supersedes/reinforces/context/evidence fields; governance serde round-trip; canonical-directory initialization failure surfaces in boot state.
- [ ] Define the event classes and payloads from the design. Keep payloads strongly typed; do not accept arbitrary JSON as the primary internal API.
- [ ] Canonicalize via explicit length-prefixed semantic fields and SHA-256, not a raw serialized JSON hash.
- [ ] Validate a monotonic per-device sequence chained through causal parents. Projection order is a deterministic topological sort: parents first, then stable event ID for independent ready events. Timestamps and relay arrival never break conflicts/ties.
- [ ] Store immutable event files below app data `twin/events/v1/<prefix>/<event_id>.json`; maintain only disposable derived indexes.
- [ ] Add `EventRecorder` and `NoopEventRecorder` traits for service injection.
- [ ] Initialize the store in app state without adding capture hooks yet.
- [ ] Run the new unit tests plus existing Twin store tests and both default/MCP compilation.
- [ ] Update `CLAUDE.md` with event location, ordering, authority, and append-only invariants.
- [ ] Commit: `feat: add governed Twin event spine`

## Task 6: Implement explainable attention and deterministic state projections

**Files:**

- Create: `frontend/src-tauri/src/models/twin_state.rs`
- Create: `frontend/src-tauri/src/services/twin_events/attention.rs`
- Create: `frontend/src-tauri/src/services/twin_events/projection.rs`
- Create: `frontend/src-tauri/src/services/twin_events/proposals.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/mod.rs`

**Contract:** Given identical events, profile version, and reference time, projection output is byte-equivalent. Relationship variants remain distinct. Attention returns every component, weights, final score, and deterministic explanation. Privacy/allowed-use, review/authority, and temporal gates run before ranking and report deterministic exclusion reasons.

- [ ] Write failing pure tests for each attention component clamped to integer basis points `0..=10_000`, profile weights summing to 10,000, checked/defined rounding, every named profile, stable ID tie ordering, relationship match/mismatch, contradiction boost, and explicit reference-time decay.
- [ ] Write failing projection tests for reviewed-memory promotion/rejection, proposal retention, supersession, reinforcement, temporal expiry, context-conditioned variants, hard sensitivity/use exclusion, and full rebuild determinism.
- [ ] Implement versioned `AttentionProfile::{Recall, Decision, Simulation, Reflection, CaptureReview}` and `AttentionExplanation`.
- [ ] Define orthogonal `ReviewState`, `AuthorityClass`, `Sensitivity`, and `AllowedUses`, plus typed entities and directional relationship assertions with validity/provenance/governance. Implement rule-based proposal creation for repeated, contradictory, and relationship-specific observations. It must cite evidence IDs and remain pending; a direct capture is evidence that the user recorded content, not proof of every claim in it.
- [ ] Implement projection snapshots with canonical JSON and a content-derived `snapshot_id`.
- [ ] Add guard tests: one hundred high-recurrence pending proposals still do not appear in Simulation's reviewed-only set; a disallowed/private record is excluded even with a maximal attention vector; legacy `AutoPromoted` records migrate to pending and no longer enter approved context.
- [ ] Run focused tests, Twin tests, clippy, and format checks.
- [ ] Commit: `feat: derive temporal contextual Twin state and attention`

## Task 7: Record governed events at real mutation boundaries

**Files:**

- Modify: `frontend/src-tauri/src/services/knowledge_store.rs`
- Modify: `frontend/src-tauri/src/services/canvas_store.rs`
- Create: `frontend/src-tauri/src/services/twin_events/journal.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mutation_coordinator.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/store.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/mod.rs`
- Modify: `frontend/src-tauri/src/services/twin/mod.rs`
- Modify: relevant siblings in `frontend/src-tauri/src/services/twin/`
- Modify: `frontend/src-tauri/src/services/import/mod.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/src/mcp.rs`
- Modify: `frontend/src-tauri/src/mcp_tools.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Test: service-level integration tests in the touched modules

**Contract:** Successful note, capture, Canvas, feedback, Twin record/digest/Constitution review, decision/outcome, and import mutations append exactly one appropriate governed event group even when invoked through MCP. Remote/recovery origins never re-record. A crash at any injected coordinator boundary recovers to either no logical operation or one complete logical operation, never a half-applied durable state.

- [x] Introduce `MutationOrigin::{Local, Remote, Recovery}` at the storage-service boundary and write failing local/remote/recovery tests before changing public methods.
- [x] Inject `Arc<dyn EventRecorder>` into shared stores using compatibility constructors that default to `NoopEventRecorder` for existing unit fixtures.
- [x] Stage a typed local intent containing mutation ID, target, before digest, bounded desired after-image/tombstone, and event group; then apply user bytes, append events idempotently, and remove the intent. Recovery advances before→after, completes event append when after already matches, and quarantines an unknown third digest instead of overwriting it. Do not rely on post-write command fan-out for event durability.
- [x] Write restart/fault-injection tests for crashes after journal stage, after user mutation, after event append, and before journal cleanup; rerunning recovery must be idempotent.
- [x] Map service changes to typed payloads and evidence refs. Imported conversations emit a container observation, not parser-noise events.
- [x] Ensure Canvas events occur only after session persistence succeeds and contain prompt/response/model/cost/provenance, excluding API keys and raw unselected context.
- [x] Add MCP integration coverage for create/update/delete event emission.
- [x] Run storage, Canvas, Twin, import, and MCP tests plus full Rust tests.
- [x] Commit: `feat: capture governed events across Grafyn mutations`

**Verification (2026-08-31):** 685 desktop Rust tests, 573 MCP tests, 12 Twin Eval lab tests, and 484 frontend tests passed; strict-warning builds, production frontend build, license alignment, source-size, scoped rustfmt, and diff checks passed. Independent review reported no Critical or Important findings.

## Task 8: Expose Twin state, review, attention explanations, and export

**Files:**

- Create: `frontend/src-tauri/src/commands/twin_state.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/src/services/twin/export.rs`
- Modify: `frontend/src/api/client.js`
- Modify: `frontend/src/stores/twin.js`
- Add/modify frontend and Rust tests for the new API

**Commands:**

```text
list_twin_observations
list_twin_proposals
review_twin_proposal
get_twin_state_projection
rank_twin_attention
get_twin_event_timeline
```

**Contract:** The UI layer can review a proposal, rebuild/read contextual state, and explain a ranking. Export JSONL adds eligible events and projection manifests while preserving existing train/eval/holdout semantics and hard allowed-use gates.

- [x] Write command tests for paging, context filters, profile selection, invalid review transitions, rejected/pending/private/disallowed exclusion, deterministic exclusion reason codes, snapshot ID stability, and redaction.
- [x] Implement commands as thin adapters over event/projection services.
- [x] Extend export with versioned `twin_events.jsonl` and `projection_manifest.json`; never add in-app scoring.
- [x] Add client/store methods and focused store tests.
- [x] Run focused Rust/frontend tests and inspect an export fixture for secrets and path leakage.
- [x] Commit: `feat: expose governed Twin state and event exports`

**Verification (2026-08-31):** 709 desktop Rust tests, 584 MCP tests, and 505 frontend tests passed; focused Twin-state/export/client-store regressions passed 12/7/92. Production frontend build, source-size and license checks, scoped rustfmt, and diff checks passed. ESLint reported zero errors and the same seven pre-existing warnings. Independent review found four stale-frontier/time/provenance/UI-binding defects; all four were fixed and covered by regressions before commit.

## Task 9: Add conversational Twin history with reproducible evidence snapshots

**Files:**

- Modify: `frontend/src-tauri/src/models/canvas.rs`
- Modify: `frontend/src-tauri/src/commands/canvas/context.rs`
- Modify: `frontend/src-tauri/src/commands/canvas/shared.rs`
- Modify: `frontend/src-tauri/src/commands/canvas/streaming.rs`
- Modify: `frontend/src-tauri/src/services/canvas_store.rs`
- Modify: `frontend/src/stores/canvas.js`
- Test: existing Canvas Rust and frontend store suites

**Contract:** `ContextMode::TwinHistory` composes compact ancestor history, then the existing Twin operating contract and reviewed contextual projection. Each prompt tile persists the projection snapshot ID and evidence event/note IDs used.

- [x] Write failing serialization, parent-order, provider-route, Advisor default, Simulation identity-gate, reviewed-only, and persisted-replay tests.
- [x] Add the enum variant with backward-compatible serde.
- [x] Refactor context assembly only enough to share compact history between FullHistory and TwinHistory; preserve existing modes.
- [x] Record snapshot/evidence metadata in a backward-compatible optional field on `PromptTile`.
- [x] Add `sendCompanionPrompt({ prompt, modelId, mode, parentTileId, parentModelId, provider })` as an object wrapper without changing existing positional `sendPrompt`.
- [x] Run Canvas focused tests and the full Rust/frontend suites.
- [x] Commit: `feat: add history-aware reproducible Twin chat`

**Verification (2026-08-31):** Focused Canvas Rust tests passed 95/95, Canvas-store tests passed 35/35, the serial full Rust library suite passed 717/717, the MCP feature suite passed 585/585, the full frontend suite passed 508/508, and the production frontend build passed. Scoped ESLint, rustfmt, and diff checks passed. Two earlier parallel Rust runs each met a different Windows tempfile ACL error; both exact tests passed alone, and the complete single-threaded run passed. Independent review found and fixed four provider/replay/session-state defects before commit: provider overrides now fail closed and freeze the effective route, replay binds the exact prompt/Constitution digest, late events cannot mutate another session, and rejected regeneration restores the prior response.

## Task 10: Create the public encrypted sync protocol crate

**Files:**

- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/Cargo.toml`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/src/lib.rs`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/src/canonical.rs`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/src/crypto.rs`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/src/types.rs`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/tests/golden.rs`
- Create: `frontend/src-tauri/crates/grafyn-sync-protocol/testdata/envelope-v1.json`
- Create: `docs/sync/SYNC_PROTOCOL_V1.md`
- Create: `docs/sync/envelope-v1.schema.json`
- Create: `docs/sync/THREAT_MODEL.md`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`

**Contract:** A transport-neutral crate seals and opens version-1 note-revision, Twin-event, attachment-manifest, and attachment-chunk operations. It verifies device identity, strict Ed25519 signature, XChaCha20-Poly1305 authentication, vault binding, canonical operation ID, and schema before returning plaintext.

- [x] Start with fixed keys/nonces and a failing golden-vector round trip.
- [x] Add tamper tests for protocol, vault, device, public key, nonce, ciphertext, signature, operation ID, and payload type; add wrong-key and JSON-field-order tests. Add manifest/chunk count, index, total size, duplicate chunk, full digest, and partial-materialization tests.
- [x] Use explicit length-prefix/domain-separated canonical bytes, HKDF-derived independent subkeys, `verify_strict`, and zeroizing secret containers.
- [x] Use `chacha20poly1305`, `ed25519-dalek`, `hkdf`, `hmac`, `sha2`, `getrandom`, `zeroize`, and `base64` versions that compile with pinned Rust. Do not enable legacy/hazmat features.
- [x] Document visible metadata, replay/DoS limits, exclusions, and protocol compatibility rules. Schema and golden vector must match generated types.
- [x] Run `cargo test --manifest-path crates/grafyn-sync-protocol/Cargo.toml`, `cargo audit` if installed, and the app's default/MCP compilation.
- [x] Commit: `feat: publish transport-neutral Grafyn sync protocol`

**Verification (2026-08-31):** The protocol suite passed 20/20 (three unit tests, sixteen integration tests, and one compile-fail API doctest); strict all-target Clippy, scoped rustfmt, schema/canonical-Base64 probes, default application compilation, MCP compilation, and diff checks passed. `cargo audit` was not installed, so dependency auditing remains an explicit final-gate boundary. Independent security review found no cryptographic break and identified three important parser/log/schema differentials; bounded private-wire parsing, metadata-only debug formatting, and canonical Base64 schema patterns were implemented with regressions before commit.

## Task 11: Add stable vault/device identity and secure secret abstraction

**Files:**

- Create: `frontend/src-tauri/src/models/sync.rs`
- Create: `frontend/src-tauri/src/services/sync/mod.rs`
- Create: `frontend/src-tauri/src/services/sync/identity.rs`
- Create: `frontend/src-tauri/src/services/sync/secrets.rs`
- Create: `frontend/src-tauri/src/services/sync/device.rs`
- Create: `frontend/src-tauri/src/services/sync/vault_keys.rs`
- Create: `frontend/src-tauri/src/services/root_transition_tests.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mutation_coordinator/custom_mcp.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mutation_coordinator/root_lease.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mutation_coordinator/stable_migration.rs`
- Create: `frontend/src-tauri/src/services/twin_events/mutation_coordinator/stable_migration_tests.rs`
- Modify: `frontend/src-tauri/src/models/mod.rs`
- Modify: `frontend/src-tauri/src/models/settings.rs`
- Modify: `frontend/src-tauri/src/services/mod.rs`
- Modify: `frontend/src-tauri/src/services/settings.rs`
- Modify: `frontend/src-tauri/src/services/twin/mod.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/mutation_coordinator.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/mutation_coordinator/engine.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/mutation_coordinator_tests.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/journal.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/secure_fs.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/secure_fs_contract_tests.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/store.rs`
- Modify: `frontend/src-tauri/src/services/vault_namespace.rs`
- Modify: `frontend/src-tauri/src/services/root_transition.rs`
- Modify: `frontend/src-tauri/src/services/canvas_store.rs`
- Modify: `frontend/src-tauri/src/commands/commit_note_write_tests.rs`
- Modify: `frontend/src-tauri/src/commands/settings.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/src/mcp.rs`
- Modify: `frontend/src-tauri/crates/grafyn-sync-protocol/src/crypto.rs`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `CLAUDE.md`

**Contract:** Vault identity survives path moves; corrupt descriptors fail without replacement or identity rotation; devices remain distinct; keys never serialize. The canonical vault UUID is domain-separated into the existing `ContentDigest` root-scope type so authority tokens remain strongly typed, while canonical paths are retained only as capability bindings. Schema-1 path authorities and transitions are recovered using their original semantics before a schema-2 UUID authority is installed; old durable bytes are never silently reinterpreted. A path change or UUID change rotates the lease epoch; a same-UUID move preserves the data namespace while retargeting the runtime capability, and a missing-old-path move uses an explicit forward-only reattach path. Replacing the descriptor UUID at the same canonical path fails closed. A candidate descriptor is planned before WAL publication but is installed only after a strict-created rollback witness's filesystem identity is durably recorded in that exact prepared transaction; the live descriptor is a no-clobber hard link to that identity, with a second durable phase recorded only after Grafyn wins the live-name install. Prepared rollback removes the live name only with that proof, preserving pre-existing, raced, or byte-identical replacement files. Identity-bound cleanup atomically moves the verified entry to a retained unpredictable quarantine and never follows it with a racy pathname unlink; committed cleanup keeps the live descriptor and quarantines only its recorded witness. Legacy FNV/path-SHA Twin, derived, Canvas, and unscoped event data is assigned exactly once without merging, only after pending mutation and retained workflow recovery. Canonical Twin events, Twin state, Canvas, derived state, and vault-root secret/account identity are vault-scoped; vault-scoped sync operation state is reserved for Task 12. The coordinator lock, mutation journal, root-transition WAL, authority generation, writer identity, and active namespace pointer remain data-root-global.

- [x] Write failing tests: vault move/reopen/concurrent creation; corrupt, symlinked, noncanonical, nil, and unsupported descriptor rejection without replacement; same-UUID path retarget with fresh epoch; descriptor replacement invalidates active authority; desktop/MCP parity.
- [x] Atomically create strict, bounded `vault/_grafyn/vault.json` schema 1 through retained no-follow capability I/O. Install absent descriptors with no-clobber semantics and reread the winner; never rewrite invalid existing bytes.
- [x] Generalize the existing OpenRouter key backend into one account-keyed `SecretStore`, desktop `KeyringSecretStore`, and test `MemorySecretStore`. Preserve service `com.grafyn.app`, every `openrouter_api_key/<version>` account, root-authority binding bytes, and root-transition recovery compatibility.
- [x] Use accounts `sync.device.ed25519.v1` and `sync.vault.<vault_uuid>.root.v1`.
- [x] Reuse the stable global `writer-v1.json` UUID as device ID. Persist and validate its signing-public-key binding; a missing or mismatched previously bound secret fails closed rather than rotating the device. Custom MCP mode may read public identity but remains keyring/sync-secret disabled.
- [x] Recover legacy pending mutations under the old lease before a crash-resumable, retained-lock assignment of exactly one recognized legacy source. Reject FNV plus path-SHA, source plus destination, occupied cross-vault destinations, and unexplained partial migration states; publish the UUID-derived lease last and rebuild before readiness.
- [x] Version the active lease/root authority transition instead of reinterpreting schema 1. Bind the migration marker to UUID/scope, legacy scope/epoch, writer UUID, exact component inventory, destination, and per-component progress; test crash recovery around initial marker publication, representative Twin/derived assignment and component rename boundaries, and lease publication.
- [x] Make the event store route canonical records/quarantine through the active UUID namespace while keeping its global process lock. Retarget event, Canvas, Twin, and derived roots atomically on a live vault switch; assign legacy global event/Canvas data only once because those records contain no prior vault ID.
- [x] Revalidate the live descriptor-derived scope during durable lease verification. Split canonical-path change from identity change so a same-UUID directory move rotates the epoch and capability without moving namespaces.
- [x] Block migration while any pre-authority owner, receipt, optimizer publication/rollback owner, or Markdown migration transaction still binds the legacy authority. Rebuild disposable indexes, but preserve authority-bearing audit/decision state and never discard it as cache.
- [x] Keep sync `not_provisioned` until a root key is deliberately provisioned; do not invent pairing, relay, or automatic vault-root keys. Test that two device stores share only an explicitly provisioned vault root.
- [x] Run identity/secret/settings/root-transition/coordinator/event/Canvas/Twin tests, desktop and MCP construction tests, strict Clippy, and scans of settings, WAL, IPC, debug, and test fixtures for serialized key material.
- [x] Commit: `feat: add stable vault identity and secure sync keys`

**Verification (2026-08-31):** Focused root-transition, secure-filesystem, writer, stable-migration, and detached-boot regressions passed 44/44, 12/12, 21/21, 25/25, and 2/2. The Windows-controlled full desktop and MCP suites passed 882/882 and 739/739 with four test workers; the protocol suite passed 21/21, desktop/MCP compilation passed, and the source-size gate passed after extracting the root-lease helpers. Unrestricted Windows parallel runs produced different non-repeating failures in independent temporary roots, including raw `Access is denied` filesystem errors; each reported test passed alone, the authority race passed 100/100 repeated runs, and both serial and four-worker full runs passed. Strict protocol Clippy passed. Full-app strict Clippy remains baseline-limited by 46 pre-existing warning errors outside the Task 11 identity/transition/security paths; no Task 11 warning remains. Scoped rustfmt, diff checks, key-material scans, and an independent adversarial review passed with no remaining Critical or Important finding. A final power-loss review then closed the descriptor-live-install/WAL-update window by atomically moving the owned witness into `vault.json`; the focused root-transition suite passed 42/42 after the regression was added, including external same-file race preservation.

## Task 12: Implement crash-safe note/event synchronization and convergence

**Files:**

- Create: `frontend/src-tauri/src/services/sync/operation_store.rs`
- Create: `frontend/src-tauri/src/services/sync/graph.rs`
- Create: `frontend/src-tauri/src/services/sync/engine.rs`
- Create: `frontend/src-tauri/src/services/sync/test_harness.rs`
- Create: `frontend/src-tauri/src/models/attachment.rs`
- Create: `frontend/src-tauri/src/services/attachment_store.rs`
- Modify: `frontend/src-tauri/src/models/mod.rs`
- Modify: `frontend/src-tauri/src/services/mod.rs`
- Modify: `frontend/src-tauri/src/services/sync/mod.rs`
- Modify: `frontend/src-tauri/src/services/atomic_io.rs`
- Modify: `frontend/src-tauri/src/services/knowledge_store.rs`
- Modify: `frontend/src-tauri/src/services/twin_events/store.rs`
- Modify: `frontend/src-tauri/src/services/markdown_migration.rs`
- Modify: `frontend/src-tauri/src/services/topic_hub.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/src/mcp.rs`

**Contract:** Local mutations are recoverably staged before disk change; remote/recovery application never echoes; causal operations are idempotent; concurrent devices converge to identical materialized note bytes, attachment bytes, heads, event sets, and projections under reorder and duplication.

- [x] Write RED tests for journal stage-before-write, mutation failure cancellation, startup recovery, immutable outbox, duplicate no-op, collision rejection, and child-before-parent deferral.
- [x] Implement whole-file note put/tombstone revisions with sorted unique causal parents. Use operation IDs from the protocol; timestamps are audit-only.
- [x] Enforce note `grafyn_sync` and event governance before sealing. Add tests that local-only/restricted/disallowed material creates no outbox operation and that a remote operation cannot silently relax a locally enforced local-only policy.
- [x] Implement deterministic head selection: tombstone above put, then operation ID; retain every head and expose conflicts; never merge Markdown automatically.
- [x] Add origin-aware whole-file application and remote index refresh without optimizer enqueue or new events/outbox entries.
- [x] Sync Twin events as immutable operations and rebuild projections after inbox drain.
- [x] Sync content-addressed attachments as encrypted manifests plus 256 KiB chunks, capped at 24 MiB and 96 chunks. Verify each chunk and the full digest; do not expose partial materialization. Deduplicate identical digests.
- [x] Write the two-device harness tests: create/update/delete; reorder/duplicates; concurrent edits; delete-vs-put; explicit conflict resolution; event convergence; complete attachment convergence; duplicate/out-of-order/missing/tampered attachment chunks; remote echo suppression; existing-vault bootstrap.
- [x] Run all sync/storage/Twin tests, default/MCP Rust suites, and clippy.
- [x] Commit: `feat: add crash-safe convergent encrypted sync engine`

**Verification (2026-09-01):** The focused sync, mutation-coordinator, and knowledge-store suites passed 122/122, 87/87, and 34/34. Fresh serial full suites passed 997/997 for the desktop library and 848/848 for the MCP binary; these rebuilt Windows test executables ran without the former `TaskDialogIndirect` entry-point dialog. Rustfmt passed. Strict full-app Clippy remains baseline-limited by pre-existing warnings outside Task 12 plus intentionally test-only/public foundation surfaces that Task 13 and later tasks consume. Two adversarial reviews closed stale bootstrap witnesses, malformed-frontmatter fail-open behavior, and external local-only identity/race handling; the final external-privacy review passed its focused tests, including 10 repeated linearization runs, with no remaining Critical or Important finding.

## Task 13: Expose optional sync status without pretending the hosted service exists

**Files:**

- Create: `frontend/src-tauri/src/commands/sync.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src/api/client.js`
- Create: `frontend/src/stores/sync.js`
- Create: `frontend/src/components/sync/SyncStatusCard.vue`
- Add focused Rust/Vue tests

**Commands:**

```text
get_sync_status
list_sync_conflicts
export_sync_outbox
import_sync_envelopes
rebuild_sync_state
```

**Contract:** Foundation mode can inspect, export, and deterministically import encrypted envelopes for testing/manual transport. It clearly reports `not_provisioned`, `local_only`, `pending`, `conflict`, or `error`. No endpoint URL, account, subscription, or “connected” fiction is added.

- [x] Write failing command/store/component tests including secret redaction, malformed envelope rejection, unavailable capability, and conflict display.
- [x] Implement thin commands over `SyncEngine`; envelope export/import operates on ciphertext bytes/files only.
- [x] Show status in desktop settings and later compact settings, labelled “sync foundation / relay not configured.”
- [x] Run focused and full suites.
- [x] Commit: `feat: expose honest local E2EE sync status`

**Verification (2026-09-01):** The command layer passed 11 focused Rust tests, including same-authority recovery after an exact/no-op import. The sync store/card and settings/API focused frontend set passed 96/96, the full Vue suite passed 522/522, and the production frontend build completed. Fresh serial Rust suites passed 1000/1000 for the desktop library and 848/848 for the MCP binary; neither rebuilt Windows executable reproduced the former `TaskDialogIndirect` loader dialog. Rustfmt and `git diff --check` passed. Recovery-pending results remain visibly non-successful and IPC/store boundaries expose only allowlisted status, counts, IDs, and ciphertext envelopes.

## Task 14: Build the adaptive companion shell, capture, and recall

**Files:**

- Create: `frontend/src/composables/useCompanionLayout.js`
- Create: `frontend/src/composables/useRecallSearch.js`
- Create: `frontend/src/components/companion/CompanionShell.vue`
- Create: `frontend/src/components/companion/CompanionBottomNav.vue`
- Create: `frontend/src/components/companion/CapabilityUnavailable.vue`
- Create: `frontend/src/components/companion/QuickCaptureCard.vue`
- Create: `frontend/src/components/companion/CaptureContextFields.vue`
- Create: `frontend/src/views/companion/CaptureView.vue`
- Create: `frontend/src/views/companion/RecallView.vue`
- Create: `frontend/src/views/ResponsiveHomeView.vue`
- Modify: `frontend/src/router/index.js`
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/style.css`
- Modify: `frontend/src/components/SearchBar.vue`
- Modify: `frontend/src-tauri/src/commands/twin_state.rs`
- Modify: `frontend/src-tauri/src/models/twin_event.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src/api/client.js`
- Modify: `frontend/src/__tests__/unit/api/client.spec.js`
- Add component/composable/router tests

**Contract:** Wide `/` renders the existing desktop Home unchanged. Compact `/` is fast capture; `/recall` is full-screen search/detail with attention explanations. Path import deep links fail closed on Android.

- [x] Write RED tests for layout selection, navigation, 44px targets, safe-area classes, blank capture, title derivation, save failure retention, recent captures, optional person/role/relationship/environment/activity/goal context, attachment association, sync-policy selection, debounce, stale-response suppression, backend result normalization, and recall states.
- [x] Add one transactional `create_companion_capture` command that writes a draft note tagged `inbox` and its contextual `ObservationRecorded` event through the Task 7 coordinator. Represent the capture kind and attachment digests in existing note `properties`; do not invent a nonexistent `note_type` field. Accept `grafyn_sync: inherit | local_only`, with local-only enforced before outbox creation.
- [x] Derive title from first non-empty Markdown-stripped line, cap at 80 characters, fallback to timestamp using an injected clock.
- [x] Move shared search behavior into `useRecallSearch` while retaining `SearchBar` as the desktop consumer.
- [x] Use `100dvh`, safe-area insets, non-hover actions, visible focus, and no horizontal overflow at 393px.
- [x] Run focused tests, full tests/build, and a browser mobile viewport smoke.
- [x] Commit: `feat: add adaptive companion capture and recall`

**Verification (2026-09-01):** Companion-focused frontend tests passed 160/160; the final shared working tree passed 596/596 Vue tests, selected ESLint, and the production build. Rust command, projection, and Knowledge Store sets passed 19/19, 27/27, and 34/34, and the final full desktop run passed 1014/1014. A 393x852 browser smoke exercised capture through success and candidate-bound Recall through the governed explanation/detail flow with equal scroll widths, safe-area/`100dvh` layout, and 44px-or-larger actions. Review found and closed stale-response, uncertain-commit retry, recent-list generation, recovered-event identity, post-commit root-retarget, and same-vault normalization-authority races. The final regressions derive observation identity from the committed intent, propagate repair's exact continuation authority, and full-token-fence optimizer enqueue. Rustfmt and targeted diff checks passed. Windows recorded no Grafyn application-popup event during or after the final suite.

## Task 15: Build companion Twin review/chat and linear Canvas

**Files:**

- Create: `frontend/src/components/companion/TwinReviewQueue.vue`
- Create: `frontend/src/components/companion/TwinChat.vue`
- Create: `frontend/src/views/companion/TwinCompanionView.vue`
- Create: `frontend/src/components/companion/CanvasSessionSheet.vue`
- Create: `frontend/src/components/companion/LinearCanvasThread.vue`
- Create: `frontend/src/components/companion/CanvasComposer.vue`
- Create: `frontend/src/views/companion/CanvasCompanionView.vue`
- Create: `frontend/src/views/ResponsiveTwinView.vue`
- Create: `frontend/src/views/ResponsiveCanvasView.vue`
- Modify: `frontend/src/router/index.js`
- Add focused view/component tests

**Contract:** Compact Twin exposes pending proposals/digest items, reviewed memory/state timeline, and multi-turn Advisor chat. Compact Canvas renders deterministic chronological threads without mounting D3, supports one model and plain/Knowledge/Twin follow-up, regenerate, and feedback.

- [ ] Write failing tests for review accept/reject/edit, attention explanation, relationship filter, history-aware chat parent IDs, Simulation gate/disclosure, and evidence snapshot display.
- [ ] Write failing Canvas tests for wide/compact selection, no `CanvasContainer` mount in compact mode, chronological ordering, session CRUD, follow-up context, streaming/error visibility, regenerate, and accept/reject actions without hover.
- [ ] Implement the smallest components over existing stores/APIs; do not duplicate persistence or streaming state.
- [ ] Keep desktop Twin workspace, spatial Canvas, debates, and multi-model comparison unchanged.
- [ ] Run focused and full frontend tests/build.
- [ ] Commit: `feat: add companion Twin and linear Canvas`

## Task 16: Add one-shot OpenRouter image generation with governed local save

**Files:**

- Create: `frontend/src-tauri/src/models/image_generation.rs`
- Modify: `frontend/src-tauri/src/models/attachment.rs`
- Create: `frontend/src-tauri/src/commands/image_generation.rs`
- Modify: `frontend/src-tauri/src/services/attachment_store.rs`
- Modify: `frontend/src-tauri/src/models/mod.rs`
- Modify: `frontend/src-tauri/src/commands/mod.rs`
- Modify: `frontend/src-tauri/src/services/mod.rs`
- Modify: `frontend/src-tauri/src/services/openrouter.rs`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src/api/client.js`
- Create: `frontend/src/utils/generatedImage.js`
- Create: `frontend/src/components/canvas/ImageGenerationDialog.vue`
- Create: `frontend/src/components/companion/QuickImageComposer.vue`
- Modify: `frontend/src/views/companion/CanvasCompanionView.vue`
- Modify: `frontend/src/components/canvas/CanvasContainer.vue`
- Add Rust and Vue tests

**Contract:** A user can request one supported square image, preview it, and explicitly save/share it. Generation alone does not mutate the vault. Save writes a bounded content-addressed attachment, a metadata note, and an observation event; secrets and provider response internals are excluded.

- [ ] Verify the current official OpenRouter dedicated image endpoints, request/response contract, capability descriptors, cost field, and supported model metadata before writing parsing code; use live discovery and record the primary-source URL in a code comment or doc, not assumptions. Never hardcode a “latest” paid model or silently retry a generation.
- [ ] Write failing request mapping, API-key/capability, provider-error, empty-image, MIME, base64, decoded-size, and path-safety tests.
- [ ] Add separate non-streaming model-discovery and `images.generate({ prompt, modelId, resolution, aspectRatio })` commands. Send exactly one image and `stream: false`; require explicit live-discovered model selection; enforce advertised resolution/aspect capability before spending; use a dedicated approximately 180-second timeout with no automatic retry. Cap JSON at 40 MiB, decoded image at 24 MiB, either dimension at 4096, and total pixels at 16,777,216. Accept only fully decoded PNG/JPEG/WebP with matching MIME, and reject SVG/unknown/mismatched media. Do not route image bytes through Canvas text response persistence.
- [ ] Write frontend tests for loading, duplicate-submit prevention, Blob URL creation/revocation, typed errors, exact-or-unavailable cost, discard warning, desktop Save As, Android file share, explicit governed save, and offline/unconfigured states. Wide and compact Canvas share the feature without changing text-model state.
- [ ] On save, use the canonical note/event mutation path and content-addressed app-owned attachment storage with digest, MIME, byte size, dimensions, source, annotation, and EXIF-retention policy; thumbnails are derived. Expose no arbitrary filesystem path on Android and surface Task 12's real attachment outbox/status instead of a generic success claim.
- [ ] Run focused/full tests and inspect a saved fixture.
- [ ] Commit: `feat: add governed companion image generation`

## Task 17: Initialize and harden the Android-first local companion

**Files:**

- Create: `frontend/src-tauri/capabilities/mobile.json`
- Create: `frontend/src-tauri/tauri.android.conf.json`
- Generate and modify: `frontend/src-tauri/gen/android/`
- Create: `frontend/src/components/companion/CompanionSettingsSheet.vue`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/src/services/settings.rs`
- Modify: `frontend/src-tauri/src/services/sync/secrets.rs`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `frontend/src-tauri/src/services/openrouter.rs`
- Modify: `frontend/src/platform/capabilities.js`
- Modify: `frontend/src/App.vue`
- Modify: `.github/workflows/test.yml`
- Modify: `CLAUDE.md`
- Modify: `HOW_TO_RUN.md`

**Contract:** Android uses an app-owned local vault and data directory, works offline for capture/recall/review over existing local state, contains no desktop-only capabilities, and never stores sync/OpenRouter secrets in plaintext. A debug APK builds when host prerequisites are present.

- [ ] Probe Java, Android SDK/NDK, Rust Android targets, and `adb`; record exact missing prerequisites before installing anything.
- [ ] Run `npx tauri android init`; review generated identifiers/minimum SDK before accepting generated files.
- [ ] Write failing Rust/frontend capability and path tests: Android app-private directories; no Documents assumption; no MCP external binary; no updater/Ollama/path picker/import/migration/optimizer; local notes/Twin/Canvas/image APIs available as declared.
- [ ] Implement platform-specific service registration. Desktop services remain intact; Android does not even initialize unavailable dependencies.
- [ ] Make canonical vault/event/attachment initialization fail into a recoverable boot screen; only search, graph, chunk, thumbnail, and projection caches may be recreated automatically.
- [ ] Delete a reviewed-memory projection cache in a test, restart, and prove it rebuilds identically from accepted review events; corrupt/missing canonical events must fail closed instead of falling back to a stale cache.
- [ ] Implement one secure Android secret adapter used by both sync and OpenRouter through the shared secret abstraction, using an OS-backed mechanism supported by the selected Tauri plugin/native layer. Target-gate `keyring`, `notify`, PDF/DOCX tooling, MCP, updater, and other desktop-only dependencies. Add a release-build assertion that rejects plaintext/fallback storage. If no production-grade adapter can be verified on this host, every secret-dependent Android capability (sync, OpenRouter text, and image generation) remains disabled and the UI/build diagnostics must say so explicitly.
- [ ] Add compact settings for theme, local vault summary, OpenRouter secret entry/status, sync state, and capability diagnostics. Desktop-only settings never render or invoke.
- [ ] Add Android CI compile/build steps with documented secret-free debug behavior.
- [ ] Run `npx tauri android build --debug` and, if an emulator/device is available, install/launch and capture logs. Otherwise run the mobile-compatible Rust target check and record the precise external boundary.
- [ ] Update live docs with commands, data paths, capability matrix, and mobile limitations.
- [ ] Commit: `feat: initialize secure Android Grafyn companion`

## Task 18: Verify the complete organism, not just its cells

**Files:**

- Create: `e2e/tests/companion-mobile.spec.js`
- Create: `e2e/tests/twin-event-sync.spec.js`
- Create: `e2e/fixtures/openrouter-stub.js`
- Create: `frontend/src-tauri/src/test_runtime.rs`
- Create: `frontend/src-tauri/src/bin/grafyn-test-runtime.rs`
- Modify: `e2e/playwright.config.js`
- Modify: `e2e/README.md`
- Modify: `frontend/package.json`
- Modify: `frontend/src-tauri/Cargo.toml`
- Modify: `frontend/src-tauri/Cargo.lock`
- Modify: `frontend/src-tauri/src/lib.rs`
- Modify: `frontend/src-tauri/src/services/openrouter.rs`
- Modify: `frontend/src/platform/runtime.js`
- Modify: `frontend/src/api/transport.js`
- Modify: `.github/workflows/test.yml`
- Modify: `CLAUDE.md`
- Modify: `README.md`
- Modify: `HOW_TO_RUN.md`

**Contract:** Automated evidence proves the success scenario from the design and current desktop features remain usable. Documentation distinguishes working local sync foundations from undeployed paid hosting.

- [ ] Add an `e2e-test-runtime` Cargo feature and required-feature test binary that calls the same production builder/commands/services with explicitly injected temporary app/vault paths. It is excluded from release capabilities/binaries. Use a bounded local HTTP stub only at the OpenRouter network boundary so no test spends money; note/event/attachment/projection/sync behavior remains production code.
- [ ] Write the Pixel-sized flow: generate and explicitly save one image while online → block network → capture contextual text → restart → recall note/attachment with attention explanation → proposal not used by Simulation → review promotion → history-aware Advisor chat → encrypted envelope delivery with duplicate/reorder → second-device note/attachment/event/projection convergence and zero echo.
- [ ] Add layout checks for all four destinations: `scrollWidth <= clientWidth`, safe areas, visible composer, 44px targets, and no desktop-only controls.
- [ ] Add wide-desktop smoke for notes/search/graph, spatial Canvas mount, import capability, settings, updater surface, and MCP status.
- [ ] Run zoomed-in suites: focused frontend files, event/projection tests, protocol crypto tests, sync convergence tests, Android capability/path tests.
- [ ] Run zoomed-out gates: `npm run test:run`, `npm run lint`, `npm run build`, `npm run check:file-sizes`, `npm run prepare:sidecar`, `cargo test --locked`, MCP feature tests, `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, desktop Tauri build, E2E, and Android debug build/target check.
- [ ] Run `npm audit --audit-level=high`, `cargo audit`, license alignment, secret scan, schema/golden-vector validation, and inspect `git diff --check` plus full branch diff.
- [ ] Use superpowers:requesting-code-review for an independent final review; resolve all blocking findings and re-run affected gates.
- [ ] Update `CLAUDE.md` as the final live architecture map and remove every obsolete desktop-only/Tauri-v1 statement.
- [ ] Commit: `test: verify Grafyn companion system end to end`

---

## Final acceptance matrix

| Area | Required evidence |
|---|---|
| Governance | metadata alignment test; exact license texts; contribution/trademark docs |
| Desktop | Tauri 2 bundle; main window/updater/import/MCP/spatial Canvas smoke |
| Event spine | append/idempotency/collision/recovery tests; real mutation hooks |
| Human state | temporal/context variants; reviewed promotion; deterministic rebuild |
| Attention | eligibility-before-ranking tests; profile/vector/explanation tests; governance-orthogonality guards |
| Twin chat | parent history, identity gate, reviewed evidence, persisted snapshot |
| Sync | golden crypto vector; tamper suite; crash journal; two-device convergence; no echo |
| Companion | capture/recall/Twin/Canvas/image component and Pixel E2E evidence |
| Android | secure app-private paths/capabilities; APK or explicit host-tooling boundary |
| Privacy | no secrets in settings/events/export/protocol/log fixtures; threat model |
| Honesty | no hosted-sync/billing claim; no in-app accuracy claim; limitations documented |

## Deferred after this delivery

- production opaque relay and subscription/billing service
- device pairing, vault-key recovery, rotation, and revocation UX
- push notifications and background continuous sync
- iOS packaging and App Store release
- video generation and a governed media gallery
- automated Markdown conflict editor/merge
- ecology/community layer beyond the single-person symbiont
- training a future world/foundation model on exported state trajectories

## Plan self-review checklist

- [ ] Every design invariant maps to a task and a test.
- [ ] Every task identifies production files, tests, focused commands, and a commit.
- [ ] No task requires the private relay to prove the public/local foundation.
- [ ] Mobile is a peer local vault, never a remote-control client.
- [ ] Event capture occurs below Tauri commands and covers MCP.
- [ ] Sync covers governed Twin events as well as Markdown notes.
- [ ] Review, authority, sensitivity, allowed uses, and attention remain different data.
- [ ] Android secret storage fails closed.
- [ ] Existing desktop behavior remains an explicit gate.
- [ ] Deferred scope is named instead of represented as implemented.
