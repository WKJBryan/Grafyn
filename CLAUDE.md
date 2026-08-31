# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

| Component | Stack | Entry Point | Port |
|-----------|-------|-------------|------|
| **Backend (Rust)** | Tauri 2, Tantivy, petgraph, reqwest | `frontend/src-tauri/src/lib.rs` (`run()`); thin desktop binary in `main.rs` | N/A |
| **MCP Server** | rmcp, Tantivy, stdio transport | `frontend/src-tauri/src/mcp.rs` (entry) + `mcp_tools.rs` (tool impls) | stdio |
| **Frontend** | Vue 3, Vite, Pinia, D3.js | `frontend/src/main.js` | 5173 |

## Development Commands

### Frontend

```bash
cd frontend
npm install
npm run dev          # Dev server on :5173
npm run build        # Production build
npm run lint         # Lint code
npm run format       # Format code
```

### Desktop App (Tauri)

**Prerequisites:** See [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/). Linux requires WebKitGTK 4.1. Also install Rust via [rustup](https://rustup.rs/). Generate app icons with `npm run generate-icons`.

```bash
cd frontend
npm install
npm run tauri:dev        # Dev mode with hot reload
npm run tauri:build      # Production build → src-tauri/target/release/bundle/
```

Environment: `set OPENROUTER_API_KEY=your-key` (Windows) or `export OPENROUTER_API_KEY=your-key`

The updater and process plugins are desktop-feature/target gated. After an accepted update finishes installing, macOS and Linux relaunch through the process plugin; Windows remains under the updater installer's supported restart behavior.

**Release prep:** Use `npm run release:prepare -- X.Y.Z` on a release branch (bumps versions, regenerates Cargo.lock, validates, commits). After merging to main, use `npm run release:tag -- X.Y.Z` to create the annotated tag. See `WORKING_GUIDE.md` for the full release process.

### Testing

```bash
# Rust tests — build the MCP sidecar binary first (required by Tauri's externalBin config)
cd frontend
npm run prepare:sidecar
cd src-tauri
cargo test

# Run a single Rust test
cargo test test_name            # or: cargo test module::path

# Frontend unit tests
cd frontend
npm run test:run

# Run a single frontend test file
npx vitest run src/__tests__/unit/components/PromptDialog.spec.js
```

On Windows/MSVC, `build.rs` embeds the Common Controls activation manifest in Rust test harnesses as well as the desktop build; this prevents `TaskDialogIndirect` loader failures while avoiding Tauri's duplicate desktop manifest. The Rust suite is filesystem-heavy, so use `cargo test -- --test-threads=4` on Windows to avoid transient scanner/ACL pressure from unrestricted parallel temp-root churn.

## Architecture Overview

**Current implementation:** Grafyn uses a Tauri 2 shared Rust library entry point with a working wide desktop shell and Vue frontend. The shell has platform-scoped capabilities and Android-neutral base configuration, but Android initialization, storage, routes, and compact UI remain unavailable until later companion-first tasks land. No web mode and no Python backend exist.

### Approved companion-first direction (owner decision, 2026-08-29)

Grafyn's mission is the local-first data-collection and governed-state layer for a person's digital twin. Desktop and Android are to become peer clients with app-owned offline vaults. Models may propose patterns, but explicit review/verification governs durable Twin memory; privacy and allowed-use gates run before per-use-case attention ranking. Time, relationship, activity, environment, and goal context remain first-class rather than being collapsed into one global trait or importance score.

The approved implementation is specified in `docs/superpowers/specs/2026-08-29-grafyn-companion-system-design.md` and decomposed in `docs/superpowers/plans/2026-08-29-grafyn-companion-first-implementation.md`. The target includes:

- Tauri 2 with a library entry point and capability-gated desktop/Android services;
- an append-only governed Twin event spine with deterministic temporal/context projections;
- orthogonal review, authority, sensitivity, allowed-use, and explainable attention data;
- compact Capture, Recall, Twin review/chat, linear Canvas, and one-shot image flows;
- an optional public E2EE sync protocol/local engine, while production relay, billing, pairing/recovery, and operations remain separate and unavailable until genuinely built;
- MPL-2.0 for the open client/local core, with the provenance decision recorded in `RELICENSING.md`; a future protocol package may explicitly use `Apache-2.0 OR MPL-2.0`, while any future hosted service remains separate and unavailable until built.

Until a task lands and its tests pass, the current implementation facts in the rest of this file remain authoritative. Update those sections in the same commit as each architectural change; never document a planned capability as already working.

```
┌────────────────────────────────────────────────┐
│        Tauri 2 Shared Core + Desktop Shell       │
│  ┌──────────────────────────────────────────┐  │
│  │         Vue 3 Frontend (WebView)          │  │
│  └──────────────────┬───────────────────────┘  │
│                     │ Tauri IPC (invoke)        │
│  ┌──────────────────▼───────────────────────┐  │
│  │            Rust Backend                   │  │
│  │  Commands → Services → Local Filesystem   │  │
│  │  (notes, search, graph, canvas, distill,   │  │
│  │   settings, feedback, mcp, memory, import, │  │
│  │   priority, retrieval, zettelkasten, twin, │  │
│  │   migration, boot)                          │  │
│  └──────────────────────────────────────────┘  │
│  ~/Documents/Grafyn/                          │
│  ├── vault/  (markdown notes)                   │
│  └── data/   (search index, canvas, settings)   │
└────────────────────────────────────────────────┘
```

### Repo Hygiene (root-level gotchas)

- The old Python backend is fully deleted; local `.venv/` and `uv.lock` are gitignored leftovers only — nothing in the app uses Python.
- `e2e/` is a committed Playwright suite (6 specs), manual-only via `npm run e2e` from `frontend/` (requires a live `npm run tauri:dev` IPC backend); it is not wired into CI.

### Tauri IPC Commands

The normal desktop shell registers 15 command modules from `frontend/src-tauri/src/commands/`. `canvas` is a directory module (split — see below); every other normal module is a single file. The `mcp` command module and both MCP invoke registrations are compiled only on desktop targets, so the shared/mobile library surface has 14 normal command modules. A sixteenth desktop source module, `twin_eval`, is compiled and registered only with the non-default `twin-eval-lab` feature. Enumerate exact command names with `grep -rn "#\[tauri::command\]" -A1 src/commands/` — purposes only below, to avoid drift.

| Module | Purpose |
|--------|---------|
| `notes.rs` | Note CRUD |
| `search.rs` | Full-text search, find-similar, reindex |
| `graph.rs` | Link graph: backlinks, outgoing, neighbors, unlinked, full graph, rebuild |
| `canvas/` | Multi-LLM canvas (18 commands) with note context; streaming via `canvas-stream` Tauri events. Split across `session.rs` (session/tile CRUD), `streaming.rs` (`send_prompt`/`add_models_to_tile`/`regenerate_response`), `debate.rs` (`start_debate`/`continue_debate`), `context.rs` (retrieval + twin-context prompt assembly, incl. `build_twin_context_prompt()`), `shared.rs` (common helpers), and `mod.rs` (re-exports) |
| `distill.rs` | LLM + rules-based distillation, tag normalization |
| `settings.rs` | Settings, first-run setup, OpenRouter key validation, Ollama status/models |
| `feedback.rs` | Feedback with offline queue |
| `mcp.rs` | MCP status + config snippet for Claude Desktop |
| `memory.rs` | Memory recall, contradiction detection, claim extraction |
| `priority.rs` | Configurable search-result ranking |
| `retrieval.rs` | Temporal + graph-aware retrieval pipeline + config |
| `zettelkasten.rs` | On-demand link discovery + background suggestion queue |
| `import.rs` | Conversation + document import (`preview_import` → `apply_import`) |
| `twin.rs` | 27 commands: user records, review, inference, Decision Mirror, Constitution, action gaps, setup, export, memory digest, session trace. Naming quirks: list commands are plural (`list_constitution_items`, `list_action_gaps`) and the digest review command is `review_memory_digest_item` |
| `migration.rs` | Markdown migration (preview/apply/rollback) + vault optimizer admin (status/settings/decisions/inbox/rollback) |
| `boot.rs` | App startup state (index ready, migration status) |

**Twin Eval lab isolation:** Normal desktop and all mobile builds omit `lab.html`, the lab frontend API, the `twin_eval` service, and all scoring/export invoke registrations. `npm run tauri:lab` and `npm run tauri:lab:build` are the only supported opt-in shell scripts; they enable both the Vite lab input and Cargo `twin-eval-lab` feature.

### Frontend

- `src/platform/runtime.js` / `src/platform/capabilities.js` — explicit `desktop-wide`, `android-compact`, and `plain-web` profiles. Android and iOS are compact; every other Tauri platform is wide desktop. The canonical capability matrix explicitly includes notes, Twin, Canvas, image, sync, native-vault, import, Ollama, MCP, migration, optimizer, and updater access; `assertCapability()` fails closed with `CapabilityUnavailableError`.
- `src/api/client.js` — preserves one namespace per command module plus `optimizer`, `isTauriApp`, and `isDesktopApp`, delegating each call through the injectable transport in `src/api/transport.js`. Its only public runtime bridge operations are `invoke`, `listen`, `openExternal`, and `showMainWindow`; `src/api/tauriTransport.js` is the static Tauri import boundary. Updater/process plugins remain deliberately desktop-only dynamic imports in `App.vue`.
- **Pinia stores (4):** `canvas.js`, `theme.js`, `boot.js`, `twin.js` — Twin records, Constitution, action gaps, decisions, Decision Mirror config, governed observation/proposal pages, projection, timeline, attention traces, and proposal-review refresh state live in the store; components read/act through it rather than holding local copies
- `components/twin/` — the Twin Workspace's tab components (`TwinOverviewTab.vue`, `TwinSetupTab.vue`, `TwinConfigTab.vue`, `TwinConstitutionTab.vue`, `TwinActionGapsTab.vue`, `TwinDecisionsTab.vue`, `TwinMemoryTab.vue`, `TwinGuideTab.vue`) plus shared pieces (`ReviewActions.vue`, `SetupField.vue`, `ActionGapRow.vue`, `DecisionRow.vue`, `EvidenceDrawer.vue`) and `twin-workspace.css`. `views/TwinReviewView.vue` is now a ~150-line shell that wires the store to these tabs — do not add Twin Workspace logic directly to the view.
- **Routes:** `/` (notes), `/canvas`, `/canvas/:id`, `/import`, `/twin` (component: `TwinReviewView.vue`, thin shell over `components/twin/`), plus catch-all → `NotFoundView.vue`
- **Tests:** `src/__tests__/{unit,integration,fixtures}/` + `setup.js` (Vitest)

## Key Concepts

### Governed Twin Event Spine

Canonical Twin history is stored as immutable JSON records below the app data directory at `twin/events/vaults/v1/<vault-scope>/records/v1/<event-id-prefix>/<event-id>.json`; quarantine and staging are siblings below the same vault scope. The stable scope is a domain-separated digest of the canonical UUID in the vault's strict `vault/_grafyn/vault.json` descriptor, so moving the vault preserves its namespace. The append/coordinator locks, mutation journal, writer identity, root-transition WAL, and active authority pointer remain data-root-global. `TwinEventStore::new` is side-effect free; `MutationCoordinator` construction holds the coordinator process lock, rejects any pending root-transition WAL, initializes the store, assigns recognized legacy state exactly once, and validates the active canonical namespace before asynchronous warm start. Legacy unscoped event paths are migration input only, never an active destination. A canonical path that is unusable, an unreadable record, or a failed quarantine becomes a recoverable `BootStatus` failure instead of a process exit or disposable/default Twin history.

The envelope preserves recorded, observed, occurred, and validity time separately, plus supersession, reinforcement, context, evidence, and orthogonal review/authority/sensitivity/visibility/allowed-use governance. All ten approved event classes use dedicated typed payload structs. Observation and proposal payloads carry typed claim assertions; Canvas, conversation, feedback, and decision payloads retain bounded meaningful content and decimal-text costs without API keys or raw unselected context.

`models/twin_state.rs` and `services/twin_events/{attention,proposals,projection}.rs` now provide the pure governed-state layer. `build_proposal_drafts()` creates stable pending drafts from three exact observations and creates unresolved two-sided clusters for exact affirm/deny conflicts; relationship-conditioned/global variants never merge, even when a qualifier is restrictive or inactive. Projected exposure intersects the causal lane plus event, every relationship qualifier, support, and event-evidence governance as immutable provenance: temporal or review inapplicability can remove current relationship applicability but never relax privacy or allowed uses. Any local source remains local and cannot sync/export. Derived proposal counts, exposure, bounded context, confirmation time, and timeline representative use the full matching support set; only the serialized evidence vector is the stable lowest 64 event IDs. `project()` internally topologically orders events and derives reviewed memory, pending proposals, relationship variants, timeline, contradiction clusters, and recent observations at an explicit reference time. Accepted review is the only event path into reviewed memory; the current review frontier removes superseded reviews and compares complete effective outcomes before converging or abstaining. Expired/superseded state stays in the audit timeline, and snapshot acceptance requires declared v1 versions plus sorted/unique top-level and nested vectors before validating the content hash. `rank()` applies allowed-use, sensitivity, visibility/causal-lane, review, authority, and temporal gates before fixed-point profile scoring; Simulation additionally requires the `ReviewedMemory` projection kind, and exclusion traces contain only IDs and first reason codes.

The five v1 attention profiles use integer basis points and checked half-up weighting rather than a global importance score. Recall weights relevance/confidence/recency; Decision weights confidence/goal/relationship/contradiction; Simulation is accepted reviewed-memory/verified-authority only; Reflection and Capture Review are local-only. The legacy serialized `AutoPromoted` variant remains readable for audit but is effectively pending: new user/inference mutations create `Candidate`, and legacy auto-promoted content is excluded from Canvas Twin prompts, Constitution activation, Decision evidence, and every export/training split. Already materialized Constitution/action-gap artifacts linked only to legacy auto-promoted records are downgraded/excluded at read time across listings, Canvas, decisions, reflection/benchmark, and export consumers; an independently Endorsed linked record preserves the artifact. No bulk on-disk rewrite occurs.

`commands/twin_state.rs` exposes versioned, bounded adapters for projected observations, pending proposals, the canonical snapshot, deterministic attention traces, and the temporal timeline. Page cursors bind the command kind, snapshot ID, and normalized relationship/goal/tag filter digest, so a later page cannot silently mix a different state or query. Proposal review accepts only `accept` or `reject`, owns all actor/time/governance metadata on the backend, rejects future or stale snapshots, and repeats the exact viewed-item check against the current pending projection inside the coordinator planner lock. The authoritative review time is captured inside that lock immediately before the current projection, so an item that expires while waiting cannot be admitted. Reviewing a derived proposal atomically materializes `MemoryProposed` plus `MemoryReviewed`; reviewing an existing proposal emits only its causally linked review. Both paths preserve the bounded sorted union of exact relationship qualifiers, validity, evidence, goals, tags, and effective local-only exposure, use exact committed event IDs, repair an authority-changing commit before rebuilding the response snapshot, and report uncertain committed outcomes as non-retryable. Frontend review requests bind the displayed proposal page and exact matching projection snapshot rather than whichever projection happened to load last.

Twin export bundle schema 3 retains the legacy train/eval/holdout bytes and adds deterministic `twin_events.jsonl` plus an export-specific `projection_manifest.json` at an explicit reference time. Event export first derives the canonical current review/supersession frontier from every event known at that time, then requires the sync-eligible lane, `synced_vault` visibility, non-`restricted` sensitivity, explicit export permission, acceptable review state, and the same privacy checks for every relationship. Typed semantic and explicit event dependencies close fail-safe: a child is removed rather than serializing a dangling or disallowed parent. A later reject, supersede, or private revoker cannot resurrect an older exportable chain; pending-only, rejected, superseded, local-only, restricted, or disallowed memory chains never enter the event artifact. The projection artifact contains accepted active reviewed memories only. The shareable manifest uses relative artifact names and contains no vault/root path, while the in-process `ExportBundle` retains absolute paths solely so the local UI can open the files.

Stored event JSON is strict: unknown fields are rejected at every envelope/context/governance/payload boundary, real provider/model IDs use their own path-aware validated type, and schema constants cap the complete record at 256 KiB, causal parents at 64, and every other evidence/context/claim/option collection at an explicit fixed limit. Decision options are an ordered user-authored list, so order and duplicates are preserved and contribute to canonical identity.

Event IDs are SHA-256 digests over an explicit domain-tagged, length-prefixed semantic encoding that excludes `event_id`; JSON field order and formatting never affect identity. Set-like fields are normalized or rejected deterministically. Appends use a cross-process lock and same-directory no-clobber installation, so identical duplicates are no-ops and ID/content collisions or wrong IDs fail closed. Each event explicitly belongs to a `local_only` or `sync_eligible` causal stream. Device sequences start at 1 independently in each `(device_id, causal_stream)` lane, and each later event must directly name the immediately preceding event in that same lane as a causal parent. This keeps a peer-valid sync chain when local-only events are omitted. Cross-stream causal parents are invalid.

`sync_eligible` is conservative structural eligibility, not evidence that transport occurred. The event and every embedded relationship require `synced_vault` visibility, non-`restricted` sensitivity, and explicit sync permission; `sensitive` data remains eligible only with that explicit permission. Every event reference from a sync-eligible event must resolve to another sync-eligible event. Local-only events may retain non-causal references to either lane, and a failing baseline is never auto-upgraded. Reads refresh across desktop/MCP peer writers and use a deterministic topological order for both the full set and stream-filtered sets: parents first, then stable event ID among independent ready events; timestamps and arrival order are never tiebreakers.

Canonical records must occupy exactly `twin/events/vaults/v1/<vault-scope>/records/v1/<two-lowercase-hex>/<event-id>.json`. Wrong-path, oversized, malformed, or transitively parentless records are quarantined before they can displace a canonical copy. Writer identity, event enumeration/read/quarantine/install, and both shared process locks retain capability-rooted handles; parent and leaf replacement cannot redirect them outside the trusted app-data root. Persistent component creation and immutable append installation synchronize directory entries on Windows and Unix; unsupported or failed durability calls fail the operation rather than silently weakening the persistence contract.

Every coordinator-owned Markdown, Canvas, and Twin write now passes through one `MutationCoordinator` shared by production desktop and MCP construction, including target-only UI/derived writes and Remote/Recovery application. Under the same cross-process lock it recovers pending work, verifies the active vault lease, plans from fresh durable state, prevalidates the finalized event group, durably stages a bounded pre-authority owner for every authority-changing local intent, advances the data-root-global authority generation, promotes the byte-identical intent into `twin/mutations/pending/v1`, applies non-tombstones then tombstones, appends events, and removes the intent only after the whole mutation is durable. This external owner preserves existing schema-1/2 intent bytes and IDs. Recovery either aborts a pre-advance owner without changing authority or reuses its proven post-advance generation and replays it exactly once; retained schema-3 owners keep bounded `AbortedBeforeAuthority` or `AbortedAfterAuthority` proof until their service owner durably acknowledges and consumes it. Local target-only and nonlocal mutations use the journal but emit no event or lifecycle call. Startup and MCP stdio recover before serving; exact before/after mixtures and partial event groups resume idempotently, while an unexpected third digest is quarantined without overwriting those bytes. Journal, staging, quarantine, lease, receipt, and pre-authority I/O use capability-rooted directory handles with bounded directory inventories: every parent and leaf is opened without following symlinks, bounded reads use one handle and `limit + 1` bytes, and same-parent create-new/rename/remove operations synchronize the parent directory. Target aliases and pairwise-overlapping Markdown/Canvas/Twin roots are rejected before staging.

Schema-3 coordinator intents may opt into a retained commit receipt; schemas 1 and 2 keep their byte-for-byte canonical identities and cannot request one. A receipt is bounded, capability-rooted, no-follow, parent-synchronized, and binds the mutation ID, full root scope, lease UUID, and exact authority generation. Once target/event effects are exact and durable, receipt or cleanup trouble is returned as committed-with-warning rather than a retryable failure. The optimizer uses this receipt with one durable per-job publication witness and strict `RetryFenced`/`Prepared`/`Committed` plus audit/count/queue/receipt cleanup phases. Its state lock and every protected optimizer read/write share one retained root capability; reservations, stable job/change IDs, exact cross-field validation, and strict bounded audit files make peer retry and restart idempotent. Terminal parking first persists a queue-owned witness, then publishes deterministic job-scoped inbox and audit records, and only then removes the queue owner, so a crash at any boundary resumes without duplicate user-visible records. Sidecar writes additionally bind a bounded no-follow Markdown snapshot as a schema-3 exact read guard and emit their governed `NoteChanged` event inside the same retained intent: drift before any writable effect aborts and retires the exact owner, while a post-authority guard abort retains exact proof for owner acknowledgement; the guard is never rewritten. Current-schema optimizer change records also bind exact raw before/after bytes, target/source digests, and a separate durable rollback owner. Rollback requires the live target and source to match those exact images, restores noncanonical Markdown or JSON bytes (or an exact tombstone), publishes one unique rollback audit/count, and consumes its retained proof idempotently. A rollback that advanced authority but has not restored its target reports a distinct recovery-pending committed warning; successful central repair converts that response to applied, while an `AbortedAfterAuthority` owner remains explicitly not applied. Legacy semantic-only change records remain readable for audit but are not rollback-capable. Complete repair invalidates readiness, drains the canonical mutation WAL, resolves optimizer apply and rollback witnesses under the retained coordinator guard, rebuilds derived state, and only then republishes ready.

Local event groups use the stable non-secret `twin/events/writer-v1.json` UUID and are finalized only after recovery under the coordinator lock. An established data root with a lease, signing/content binding, migration history, event history, or retained mutation proof but no writer fails before startup writes; a genuine first install or coherent pre-writer legacy upgrade installs the writer before the first active lease can be published. Writer installation has a dedicated `twin/events/writer-staging/v1` directory: a single bounded startup scan tolerates and retains only exact canonical UUID-v4 installer temps, while every unrecognized entry fails closed. Startup never deletes those name-classified remnants, and legacy event staging remains established-history evidence. That UUID is also the sync device ID; `twin/events/device-signing-v1.json` binds it to a public Ed25519 key, while the private seed exists only in keychain account `sync.device.ed25519.v1`. Vault root keys are never created on startup and, when deliberately provisioned by a later workflow, use `sync.vault.<vault_uuid>.root.v1`. The account-keyed secret abstraction preserves service `com.grafyn.app` and the existing immutable `openrouter_api_key/<version>` accounts. Sync device seeds and vault-root keys use zeroizing wrappers, redact `Debug`, and never serialize into settings, WALs, IPC, or public bindings. OpenRouter key import remains an explicit settings-command IPC boundary for compatibility; durable settings and transition WALs contain only its non-secret active key reference, while the secret is stored in its immutable keychain account.

Markdown intents carry the active root scope plus a lease epoch. Schema-1 path leases are recovered with their original semantics before the schema-2 UUID lease is published. Vault switching is a strict bounded `root-transition-v1` prepared/committed transaction over the old/new settings, root scope, lease, and non-secret active key reference: startup recovery rolls prepared state back and committed state forward before stores are constructed. A same-UUID path move preserves the namespace but rotates the lease epoch; if the configured old directory is missing, startup does not recreate it and accepts only an explicit forward-only reattach to a real directory with the same descriptor. First-install vault creation rechecks the root WAL and active lease while retaining the coordinator process lock. A newly planned candidate descriptor is installed only after a strict-created witness's filesystem identity is durably claimed by that exact prepared WAL, then hard-linked into place without clobbering; a separate durable phase records only Grafyn's successful live-name install. Rollback removes the live name only with that proof and uses identity-bound rename into a retained random quarantine rather than a racy pathname unlink, so pre-existing or same-byte replacement files are preserved; committed recovery retains the live descriptor and quarantines only its recorded witness. After settings/root-transition recovery succeeds, a runtime-vault preparation or coordinator-construction failure leaves the recovery UI available over process-lifetime temporary service roots outside the configured vault and app-data root; authoritative commands stay blocked, and dropping the final app state removes those temporary roots. Descriptor replacement, corruption, nil/noncanonical UUIDs, and unexplained partial migration state fail closed. Legacy Twin, derived, Canvas, and unscoped event state is moved under a per-vault retained migration marker with no-clobber renames; another vault's scoped history is never treated as the active vault's migration record.

Each settings/root/key transaction rereads and patches the fresh durable authority under the same process lock, so a stale process cannot overwrite a peer's key or unrelated setting. OpenRouter secrets live in immutable versioned keychain accounts; the durable active reference is published transactionally, legacy plaintext is sanitized only after a keychain-first migration is durably referenced, and an omitted environment-backed runtime key remains memory-only across unrelated success and rollback. A process-wide root gate serializes short root-dependent work; long model/network work carries the exact post-mutation authority token and revalidates it before each later publication. Switching drains the old root, advances the lease, swaps event/Knowledge/Canvas/Twin runtime roots, resets derived queues, rebuilds indexes, and publishes settings last. A stale desktop/MCP writer fails before bytes or events, including after an A-to-B-to-A switch. MCP CLI overrides are either absent or the inseparable `--vault` plus `--data` pair. Isolated MCP mode constructs no `SettingsService` or keyring/sync-secret authority. Its custom-root preflight is authority-neutral: it prepares and validates the real directories while rejecting transition/prepared-state conflicts, but writes no descriptor, binding, or lease. Coordinator construction then reacquires one retained process lock, rechecks the transition WAL, audits no-lease legacy ownership, loads or creates the identity as permitted, and validates or installs the no-clobber `custom-mcp-root-binding-v1.json` before lease publication. That binding ties the stable vault scope to one canonical vault path, so a same-UUID copy at another path is rejected; existing stable custom data without proof fails closed rather than being auto-adopted or rebound.

The optional sync foundation is now a vault-scoped, transport-neutral E2EE operation engine rather than a hosted-service claim. Canonical signed envelopes are stored immutably in bounded staged/inbox/outbox areas; caller batches are fully preflighted before individually atomic immutable installs, exact durable duplicates are no-ops even after quarantine, and ID/byte collisions fail before admission. Notes retain opaque signed identities across paths and editor round-trips, unknown peer IDs project only to hashed safe paths, and local-only policy is enforced before sealing and again against the exact current Markdown snapshot before materialization, including manual edits outside Grafyn. Policy changes or rejected ancestors drive a durable transitive terminal quarantine instead of wedging unrelated sync. Complete event dependencies include causal parents, supersedes, reinforces, and top-level/relationship event evidence. Deterministic note heads retain conflicts while projecting tombstone-first then operation-ID order; a vault-scoped cross-process materialization lock, exact before-image guards, bounded retry, and an idempotent projection repair keep disk bytes aligned with the logical winner across peer writers and crashes. Attachments use bounded manifests/chunks and publish only after exact size plus full digest verification; a standalone promotion witness recovers local batches without replaying poisoned remote attachment state. Existing-vault bootstrap preserves exact sealed witness bytes/nonces, orders the complete semantic event graph, and fails or regenerates rather than promoting an inventory, policy, or path mapping that changed after prepare. Desktop and MCP recover pending remote operations before bootstrap, and derived readiness is repaired from current authority even when replay is already an exact no-op. There is still no relay, account, subscription, acknowledgement/pruning protocol, background retry scheduler, or time-based retention: manual bounded export/import is the only supported transport and reaching a hard cap fails closed.

Vault-derived persisted state is isolated at `vault_derived/v1/<full-root-scope>/`; search/chunk indexes, Knowledge overlays, link discovery, optimizer artifacts, and Markdown migration state never share an unscoped directory between vaults. Readiness schema 2 binds that namespace to the active root scope, lease epoch, and exact global authority generation. A complete desktop repair invalidates readiness, drains pending replay, refreshes the authoritative Knowledge cache before normalization, captures exact token T, reloads Knowledge and complete Twin records/traces from durable bytes after T, builds Search/Chunk/Graph/Link/Optimizer/Migration from that snapshot, reloads the Tantivy reader, and publishes only by final CAS T. Every desktop authority mutation returns its exact post-commit token directly to its caller and passes that token through this one repair seam; there is no shared last-mutation slot. Knowledge note/import errors retain their exact `MutationCommit`, target-aborted state, and planned note IDs through GUI and MCP boundaries; successful repair returns the newer normalization authority for chained writes, while committed recovery-pending responses explicitly prohibit retry. Multi-step note, link, distill, and topic-hub workflows retain prior durable work when a later step fails or aborts, repair from the newest exact authority token, and report partial completion with an explicit no-retry result. MCP success-path refresh failures use the same sanitized non-retry notice. If rebuilding fails, the user mutation remains committed and derived admission stays unavailable rather than inviting a duplicate retry. Process-local loaded tokens and before/after read tickets reject stale authoritative or derived results; Link and Optimizer additionally persist checked service revisions for same-generation workflow state. MCP requires ready T before constructing its read-only services, reloads the reader, validates the same T afterward, and never recaptures a newer token for old caches. Authoritative note CRUD remains available when derived admission is unavailable and invalidates readiness on write. Canvas authoritative reads reload after token capture and finish-validate after releasing the cache lock; supported Ollama network reads fence both root and settings authority. Legacy derived assignment records the exact initial component set and move progress and refuses scoped occupancy or an unrecorded resume destination rather than merging. Prepared legacy Twin assignment accepts only its recorded source-present move or destination-present completion. Migration/optimizer mutations chain the exact post-commit token, and optimizer source/overlay publication fails before state changes on stale authority.

Capture records durable evidence rather than UI execution noise: notes emit `NoteChanged`; each conversation/document import adds one empty-claim container observation; persisted Canvas prompts, completed responses, regeneration, debate turns/responses, and explicit feedback use their dedicated payloads; and legacy Twin text remains empty-claim observation evidence while explicit reviews/actions become feedback. Canvas responses persist bounded provider/provenance at generation time, and events copy those values without model-name inference. A decision submission persists its Canvas tile, decision episode, trace, primitive assessment, and single `DecisionRecorded` group atomically. Once its sealed prediction request is durable, every same-root abandonment attempts an idempotent requested-to-failed transition against current exact authority; a genuine root/lease switch touches neither vault, and an already committed visible response is preserved. Generic legacy record writes cannot change promotion/privacy/rejection/training state; explicit governance actions use the feedback boundary. Derived indexes, optimizer overlays, digests, parser sections/turns, pending or failed Canvas states, layout/viewport/structural edits, and general product diagnostics emit nothing. A local/private/restricted member makes its whole event group local-only. Native `MemoryProposed`/`MemoryReviewed` is created only by the explicit proposal-review boundary; the sync operation/attachment foundation exists, while hosted relay and image/video capture remain unimplemented.

### Wikilink Pattern

```markdown
[[Note Title]]              → Links to note with exact title
[[Note Title|Display]]      → Custom display text
```

**Typed links:** Wikilinks support relationship annotations: `- [[Target]] (supports)`. Nine `RelationType` variants: `related`, `supports`, `contradicts`, `expands`, `questions`, `answers`, `example`, `part_of`, `untyped`. Bare `[[wikilinks]]` get `Untyped`. The graph index stores `TypedEdge` with relation types; backlinks get the reverse relation via `RelationType::reverse()`.

**Graph Index:** Parses all notes on `build_index()` to construct typed adjacency lists (`Vec<TypedEdge>`). Backlinks are reverse edges: if A links to B, B has backlink from A. Methods: `get_outgoing()`/`get_backlinks()` return `Vec<NoteMeta>`; `get_typed_outgoing()`/`get_typed_backlinks()` return `Vec<(NoteMeta, RelationType)>`.

### Note Status Workflow

```
draft → evidence → canonical
```

Stored in YAML frontmatter `status` field. Frontend filters/displays based on status.

### YAML Frontmatter Format

```markdown
---
title: Note Title
status: draft
tags: [tag1, tag2]
created_at: 2025-01-07T12:00:00Z
updated_at: 2025-01-07T12:00:00Z
---

Markdown content here with [[wikilinks]].
```

Additional frontmatter fields for provenance: `source`, `source_id`, `container_of`, `created_via`, `mcp_created_at`.

### Container → Atomic → Hub Workflow

Distillation splits large "container" notes into focused "atomic" notes:

```
Container (evidence) → Atomic Notes (draft) → Hub (topic index)
```

- **Extraction modes:** `rules` (H2/H3 splitting), `llm` (structured JSON via OpenRouter, model configurable via settings), `auto` (LLM with rules fallback)
- **Hub creation policy:** `auto` (tag frequency ≥3), `always`, `never`
- **Deduplication:** `skip` (default — skips matching titles), `merge`, `create`
- Tag normalization: `#Tag` → `tag` (lowercase, strip #, spaces→hyphens)
- Inline `#tag` parsing (ignores headings and code blocks)
- Canvas exports use protected section markers to preserve user edits

### Zettelkasten Link Discovery

Discovers potential links using semantic similarity and LLM analysis. Three methods: **Semantic** (cosine similarity > threshold), **LLM** (OpenRouter analyzes content), **Hybrid** (semantic candidates + LLM ranking).

### Multi-LLM Canvas (with Note Context)

Compare responses from multiple LLM models simultaneously via OpenRouter. Features: parallel model streaming, infinite canvas with D3.js zoom/pan, model debate mode, vault-scoped session persistence in `data/canvas/v1/<vault-scope>/`, **semantic note context** (retrieves relevant notes as LLM system prompt).

**Semantic context mode:** When `context_mode == Semantic` (the default), `send_prompt` runs a two-stage pipeline: (1) note-level retrieval as a quality gate, (2) if `chunk_retrieval_enabled` (default: `true`), chunk-level retrieval fills relevant paragraphs within `default_token_budget` (default: 4000 tokens). Falls back to whole-note truncation (1500 chars) if chunks are empty or disabled. Pinned notes per session (`pinned_note_ids`) are always included. Context notes are stored on the tile and emitted via `ContextNotes` event for frontend display.

**History-aware Twin context:** `ContextMode::TwinHistory` composes the selected root-to-leaf parent chain as compact ordered conversation history, then applies the existing Twin operating contract and the governed reviewed-memory projection. Advisor is the companion default; Simulation keeps the existing identity gate and retrieves through the stricter Simulation attention profile. A new tile freezes its effective OpenRouter/Ollama provider and persists an optional `TwinEvidenceSnapshot` containing the projection snapshot/reference time, sorted event and note IDs, and a bounded SHA-256 digest over the exact ordered messages, final composed system prompt, and context version. Add-model and regenerate replay use the frozen route and reference time and fail closed if the projection, ancestors, notes, or Constitution setup no longer reproduce the persisted context. Explicit provider overrides reject unknown values. Frontend stream mutations are scoped to the session that initiated them, and a regenerate rejected before streaming restores that session's exact prior response.

**Streaming architecture:** Commands return immediately, spawn async tasks, stream via `canvas-stream` Tauri events (`TileCreated`, `ContextNotes`, `Chunk`, `Complete`, `Error`, `SessionSaved`, debate variants). Frontend listens via `@tauri-apps/api/event`.

Streaming commands: `send_prompt`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response`

**Canvas response costs and caching:** OpenRouter's final streaming usage chunk provides the exact `cost_usd` persisted on each `ModelResponse` and `DebateResponse`; legacy sessions have no cost and show no label. Canvas OpenRouter requests include a stable session-and-model identifier so OpenRouter can apply compatible provider-side prompt caching to follow-up context without changing model routing.

### Public Encrypted Sync Protocol Foundation

`frontend/src-tauri/crates/grafyn-sync-protocol` is a transport-neutral workspace crate licensed `Apache-2.0 OR MPL-2.0`. It defines strict version-1 encrypted envelopes for whole-note revisions, immutable Twin events, attachment manifests, and 256 KiB attachment chunks. Canonical domain-separated length-prefixed bytes feed HMAC-SHA-256 operation IDs; HKDF-SHA-256 derives independent per-vault/device XChaCha20-Poly1305 and operation-ID subkeys; Ed25519 signatures are verified strictly before decryption. The full routing identity and nonce are authenticated, decrypted operations are re-encoded byte-for-byte, and attachment output is withheld until every bounded chunk and the complete SHA-256 digest verify.

Untrusted envelopes must enter through the raw-size-bounded `EnvelopeV1::from_json`/`from_json_bytes` boundary; generic Serde deserialization is intentionally unavailable. JSON fields and canonical Base64URL spellings are strict, secret wrappers zeroize and redact `Debug`, and plaintext-bearing types expose metadata/length-only debug output. The normative schema, golden vector, interoperability rules, visible relay metadata, and threat model live in `docs/sync/`. This is only the public protocol foundation plus local stable vault/device identity and secure-secret boundary. A local sync engine, deliberate user-facing vault-root provisioning, transport/relay, pairing/recovery/revocation, accounts, billing, and hosted operations do not yet exist and must not be represented as connected or available.

### Twin Identity, Constitution, And Decision Mirror

Twin context mode is a native RAG path, not model-weight training. `frontend/src-tauri/src/commands/canvas/context.rs` assembles the model-facing prompt through `build_twin_context_prompt()`.

`TwinStore` itself is defined in `services/twin/mod.rs` (struct + shared state) with its methods split by concern across sibling files in `services/twin/`: `records.rs` (user records, inference, promotion), `constitution.rs` (Constitution items + review), `decisions.rs` (decision episodes, outcomes, Decision Mirror), `digest.rs` (memory digest), `traces.rs` (session traces, reflection cards), `export.rs` (JSONL export bundles), and `shared.rs` (corrupt-file quarantine + common helpers). There is no single `twin_store.rs` file — treat `services/twin/` as one logical module split across files, the same pattern as `commands/canvas/`.

The prompt order is:

1. Twin Operating Contract
2. Twin Identity
3. Reviewed Constitution
4. Action Gap Risks
5. Relevant Evidence
6. Approved User Records
7. Tentative Candidate Records
8. Answer Instructions

Twin Identity lives in `ConstitutionSetup` and is persisted in `constitution_setup.json` with `twin_name`, `twin_role`, and optional `source_boundaries`. Name and role/context are required before `TwinAnswerMode::Simulation` can run. The backend enforces this in the twin context resolution path so direct IPC calls cannot bypass the setup gate.

Simulation mode uses first-person model-facing instructions such as `I am {twin_name}` and is tuned for mimicry from supplied Knowledge materials, reviewed Constitution, selected evidence, and reviewed twin records. Disclosure that this is a configured twin simulation belongs in the app UI and docs, not inside the Simulation system prompt. Advisor mode remains a decision-support assistant and may use Twin Identity as role/context without speaking as the twin.

Twin Workspace (`/twin`) owns review and setup: user records, memory digest, Constitution items, action gaps, decision episodes/outcomes, Decision Mirror config, and guided setup. `Save Setup` writes guided setup Constitution items for operating priors; the identity fields are setup metadata and should not become normal Constitution items.

See `TWIN_RAG_SPEC.md` for the full twin RAG specification and `WORKING_GUIDE.md` for release workflow details.

**Twin accuracy evaluation is external by design (owner decision, 2026-06-10):** Do NOT build in-app accuracy scoring, benchmark dashboards, or eval-result UIs. This is a public repo and the owner does not want to impose a specific evaluation format on users. The app's responsibility is **capture + export only**: sealed twin predictions at decision time, decision outcomes, feedback/ranking traces, and the JSONL export bundles (train/eval/holdout splits). Scoring, holdout replay, calibration analysis, and accuracy dashboards live in the owner's external evaluation harness (separate lab environment), consuming the exported data. See `TWIN_ACCURACY_ROADMAP.md`.

**Web search:** When `web_search: true`, OpenRouter's `plugins: [{"id": "web", "max_results": 5}]` is added to the API request (~$0.02/query per model). The `web_search` flag is threaded through the full stack and persisted on the `PromptTile` struct (`models/canvas.rs`) for regenerate/add-model replay.

**Smart web search auto-detection:** Controlled by `UserSettings.smart_web_search` (default: `true`). When enabled, `useWebSearchDetection.js` analyzes prompt text with 5 heuristic rules (temporal markers, explicit search intent, news patterns, freshness queries, comparisons) and suppression rules (code blocks, wikilinks, short prompts). Detection result is shown as a hint in `PromptDialog.vue`. Disable via Settings toggle.

### Conversation & Document Import

Import external content as evidence notes. Six parsers in `services/import/`: `chatgpt`, `claude`, `grok`, `gemini`, `transcript` (plain transcript/Codex-style exports), and `document` (DOCX/PDF). A seventh module, `services/import/semantic_links.rs`, runs an optional LLM semantic-link-suggestion pass over imports (own default model constant `DEFAULT_IMPORT_LINK_MODEL`). Conversation formats auto-detect via platform-specific JSON keys; each parser implements `can_parse()` + `parse()`. Document imports split DOCX/PDF files into linked section notes (PDF heading detection, with optional outline titles) and add structural wikilinks. Imported content becomes evidence-status container notes with provenance metadata (`source`, `source_id`, `created_via`). Both conversation and document paths flow through the same `preview_import` → `apply_import` commands.

### Temporal + Graph-Aware Retrieval

**Note-level pipeline:** Tantivy keyword search → timestamp enrichment from GraphIndex → priority scoring (recency/status/tags) → N-hop graph expansion (bidirectional, with relation-type weighting) → hub boost (highly-connected notes) → top-K results with relevance reasons. Graph expansion uses `get_typed_outgoing()`/`get_typed_backlinks()` and multiplies proximity boost by `RelationWeights` (e.g., `supports: 1.5x`, `contradicts: 1.2x`, `untyped: 1.0x`).

**Chunk-level pipeline:** `retrieve_chunks()` searches the `ChunkIndex` (paragraph-level Tantivy index built via TextTiling), applies the same graph/hub/priority boosts via parent note, then greedily fills a token budget. Used by canvas semantic mode for precise context injection.

Configurable via `RetrievalConfig` (persisted in `data/retrieval_config.json`): `graph_hop_depth`, `graph_proximity_weight`, `hub_boost_weight`, `hub_threshold`, `base_search_limit`, `default_token_budget`, `chunk_retrieval_enabled`, `relation_weights`.

### Topic Hub Auto-Management

`services/topic_hub.rs` automatically manages topic hub notes that act as tag-keyed index pages. Called via `sync_topic_hubs()` in `commands/mod.rs`. **This is the gateway to all index rebuilds** — `rebuild_all_indexes()` calls `sync_topic_hubs()` first, so hub state is always consistent before search/graph/chunk/optimizer indexes are rebuilt.

Hub clustering rules: label-propagation over linked note groups; noise filtering suppresses model names, provider names, and transcript artifacts from becoming hubs; minor themes are grouped under a parent hub's `Subtopics` section rather than creating new hubs.

### Note Write & Delete Chokepoints

Any note write or delete that must stay consistent with the search index, chunk index, topic hubs, and vault-optimizer queue goes through two chokepoints in `commands/mod.rs`: **`commit_note_write`/`commit_note_writes`** (single/batch write — indexes the note, syncs the chunk index, runs `sync_topic_hubs`, enqueues the note in the vault optimizer) and **`commit_note_delete`** (the delete-side equivalent). **New cross-cutting write paths must call these** rather than writing a note and re-indexing ad hoc.

Underlying atomic file I/O is `services/atomic_io.rs::write_atomic` (temp-file-then-rename, so a crash mid-write never leaves a half-written note on disk). `services/index_commit.rs` holds the shared search-index-write helpers (`index_note_for_search`, `commit_search`) used by both chokepoints and any other direct index writer.

`commands/mod.rs`'s module-level doc comment documents the **canonical lock order**: `knowledge_store` must always be acquired before `vault_optimizer` (order doesn't matter between read/write, but `vault_optimizer` must come second) — reversing it risks an ABBA deadlock between the background vault-optimizer worker and any command that touches both locks.

### Background Services

These services run automatically in the background and have dedicated inbox/decision/rollback APIs. Do not re-implement any of these — they already exist.

**`services/link_discovery.rs`** — background link-discovery worker. Distinct from on-demand zettelkasten discovery (`discover_links` command). Uses YAKE keyword extraction (`services/yake.rs`) and TF-IDF cosine similarity (`services/similarity.rs`) to find wikilink candidates without LLM calls. Optional LLM pass controlled by `background_link_discovery_llm_enabled`. Results surface via `list_link_suggestion_queue` / `dismiss_link_suggestion` commands.

**`services/vault_optimizer.rs`** — background vault optimizer. Queued via `enqueue_vault_optimizer_note()` whenever notes are created or migrated. Processes the queue and applies structural improvements in two modes: `sidecar_first` (overlay metadata) or `full_rewrite`. **The proposals are rule-based, not LLM-based** — title-alias normalization and topic tagging computed directly from vault content; there is no `OpenRouterService`/network call anywhere in this file. `background_vault_optimizer_llm_enabled` is a reserved no-op today (a characterization test, `run_next_ignores_llm_enabled_because_no_llm_path_exists`, locks in that toggling it changes nothing); it exists as the gate a future LLM-backed enrichment step must check before making any network call. The budget/daily-write settings (`background_vault_optimizer_max_daily_writes`, etc.) cap **disk writes to the vault** (`daily_write_count`), not LLM spend. Decisions are auditable via `list_vault_optimizer_decisions`; rollbacks are per-change via `rollback_vault_optimizer_change`.

**`services/markdown_migration.rs`** — one-shot structured vault migration. Preview → apply → rollback workflow. `apply_markdown_migration` runs `sync_topic_hubs` and rebuilds all indexes after applying, then enqueues touched notes in the vault optimizer. Rollback restores pre-migration state and rebuilds.

**`services/yake.rs`** / **`services/similarity.rs`** — keyword extraction (YAKE algorithm) and TF-IDF similarity, used by link discovery. Not to be re-implemented as generic utilities.

### Feedback & Bug Reporting

Submit bug reports, feature requests, and general feedback. Creates GitHub Issues automatically. Desktop app has offline queue with automatic retry.

### Settings System

First-run setup wizard and persistent settings. Manages vault path, OpenRouter API key, MCP configuration, theme preferences, and LLM model selection. Settings stored as JSON in app data directory. Frontend: `SettingsModal.vue`.

- **`llm_model`** — configurable LLM model for distillation and link discovery (default: `anthropic/claude-3.5-haiku`), selectable via Settings dropdown when API key is configured
- **`smart_web_search`** — enables automatic web search detection in canvas prompts (default: `true`). Uses `#[serde(default = "default_smart_web_search")]` for backward-compatible `true` default.
- **`twin_llm_provider`** — selects the LLM runtime for twin context answers: `"openrouter"` (default) or `"ollama"`. Gates whether `OllamaService` or `OpenRouterService` is used in `build_twin_context_prompt()`.
- **`ollama_base_url`** / **`ollama_model`** — endpoint and model for local inference. `get_ollama_status` probes the Ollama daemon; `list_ollama_models` enumerates pulled models. Synced via `ollama.set_base_url()` on settings change.
- **`background_link_discovery_enabled`** / **`background_link_discovery_llm_enabled`** — controls the background link-discovery worker. When enabled, `LinkDiscoveryService` processes notes in the background using YAKE keyword extraction and TF-IDF similarity.
- **`background_vault_optimizer_enabled`** — controls the `VaultOptimizerService`. When enabled, optimizer processes queued notes and applies structural improvements (sidecar overlay or full rewrite mode) using rule-based proposals only (no LLM call today). Max daily writes cap disk writes to the vault, not LLM spend — see Background Services above.
- **Vault optimizer sub-settings** (all on `UserSettings`): `background_vault_optimizer_llm_enabled` (reserved no-op — no LLM path exists yet), `_budget_monthly` (unused until an LLM path exists), `_max_daily_writes` (real — caps disk writes), `_edit_mode`, `_program_enabled`, and `vault_optimizer_program_path` — the last two enable a **vault-local `program.md` policy file** that steers optimizer behavior per-vault.
- **`canvas_model_presets`** — saved canvas model combinations (`CanvasModelPreset` struct in `models/settings.rs`).

**Runtime sync pattern:** When settings change via `update_settings`, dependent services are updated in-place — no restart required. The pattern (in `commands/settings.rs`): capture changed fields before moving the update, apply settings, then sync each affected service:
- **OpenRouter API key** → `openrouter.set_api_key()`
- **Ollama base URL** → `ollama.set_base_url()`
- **Vault path** → `knowledge_store.set_vault_path()` + rebuild search index + rebuild graph index + reinitialize `TwinStore`

## Configuration

Environment variables for the desktop app:

| Variable | Notes |
|----------|-------|
| `OPENROUTER_API_KEY` | Required for Multi-LLM Canvas (including note context), distillation, link discovery |
| `GITHUB_FEEDBACK_REPO` | Target repo for feedback issues (format: `owner/repo`) |
| `GITHUB_FEEDBACK_TOKEN` | GitHub PAT with `issues:write` scope |
| `RUST_LOG` | Logging level (default: `info`) |

## MCP Server (Desktop + Claude Desktop)

The desktop app bundles a native Rust MCP server binary (`grafyn-mcp`) that Claude Desktop launches directly via stdio transport. No Python, no sidecar process management — just a ~10MB binary that reuses the same Rust services as the Tauri app.

```
Claude Desktop → launches grafyn-mcp (stdio) → reads/writes vault files
                                              → queries Tantivy search index
                                              → traverses link graph
```

**Architecture:** The `grafyn-mcp` binary is a second `[[bin]]` target in the same `Cargo.toml`, compiled with `--no-default-features --features mcp` (no Tauri). It shares `services/` and `models/` modules with the Tauri app. `mcp.rs` is the thin binary entry point; all tool implementations live in `mcp_tools.rs` (`#[tool_router]` on `GrafynMcpServer`).

**Concurrent access:** The MCP binary tries to acquire the Tantivy writer lock. If the Tauri app holds it, it falls back to read-only search (queries work, index updates are skipped). File I/O to the vault is always safe.

**Building locally:**
```bash
cd frontend
npm run prepare:sidecar             # debug build + copy into src-tauri/binaries/
# or manually:
cd src-tauri
cargo build --release --bin grafyn-mcp --no-default-features --features mcp
```

**11 MCP tools:** `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note`, `search_notes`, `get_backlinks`, `get_outgoing`, `recall_relevant` (with optional `token_budget` for chunk retrieval), `search_chunks` (paragraph-level search with token budgeting), `import_conversation` (also accepts documents and transcripts, splitting them into linked section notes)

**Connecting Claude Desktop:** the Grafyn Settings UI generates the `claude_desktop_config.json` snippet (server key `grafyn`, args `--vault <path> --data <path>`) with correct paths pre-filled.

## CI/CD

### Test Pipeline

`.github/workflows/test.yml` — runs on push to main and PRs. Jobs: `release-preflight` (version + Cargo.lock alignment), `rust-tests` (ubuntu-22.04), `frontend-tests` (Vitest), `lint` (eslint + file-size tripwire + `cargo clippy -D warnings`), `security` (`npm audit --audit-level=high`), `build` (Vite), `test-summary`. npm audit **blocks** PRs; clippy currently runs with `continue-on-error: true` pending cleanup of pre-existing warnings (see the TODO in the `lint` job). The file-size tripwire (`npm run check:file-sizes`, `scripts/check-file-sizes.cjs`) blocks PRs when any non-test `.rs`/`.vue`/`.js` source file exceeds 2,500 lines.

### Release Pipeline

`.github/workflows/release.yml` — triggered by `v*` tags. Also supports `workflow_dispatch` with `dry_run` for debugging builds without publishing.

```
prepare-release → build (4-platform matrix: MCP binary + tauri-action) → verify-release-assets → publish-release → upload-to-r2 (Cloudflare R2 + updater endpoint verify) → cleanup-draft → build-summary
```

**Required secrets:** `TAURI_PRIVATE_KEY`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `FEEDBACK_REPO`, `FEEDBACK_TOKEN`

**Required vars:** `CLOUDFLARE_WORKER_URL` (optional, has default)

### Release Smoke Tests

`.github/workflows/release-smoke.yml` — runs on PRs and pushes to main. Builds Windows (x64 + ARM64) and Linux targets without signing to catch release-only build failures early. macOS is excluded from smoke (10x billing multiplier) and only built at actual release time by `release.yml`.

### Weekly Dependency Check

`.github/workflows/latest-deps.yml` — runs Mondays 4am UTC + manual trigger. Tests `cargo update` + full build + tests with both pinned Rust 1.93.1 and stable toolchains to detect breaking dependency changes early. Uses `continue-on-error` so failures are informational.

### Dependabot

`.github/dependabot.yml` — weekly auto-update PRs for Cargo, npm, and GitHub Actions dependencies.

## CI Pitfalls (Known Issues & Fixes)

### Tauri 2 Requires WebKitGTK 4.1

Every Linux job that compiles the Tauri shell installs `libwebkit2gtk-4.1-dev`. Ubuntu 22.04 remains the release baseline to avoid unnecessarily raising the AppImage glibc floor.

### Rust CI Requires Stub dist/; Desktop Bundles Require the MCP Binary

`cargo test` compiles the full crate including `tauri::generate_context!()`. Keep the test and bundle prerequisites separate:

1. **Stub frontend** — `tauri::generate_context!()` requires the base config's `frontendDist` directory to exist:
   ```bash
   mkdir -p ../dist && echo '<html></html>' > ../dist/index.html
   ```
2. **Desktop bundle sidecar** — `tauri.desktop.conf.json` declares `binaries/grafyn-mcp`, so desktop bundle jobs prepare the target-suffixed binary first:
   ```bash
   cd frontend && npm run prepare:sidecar
   ```
   This builds `grafyn-mcp` with `--no-default-features --features mcp` and copies it to `binaries/grafyn-mcp-<target-triple>`. It supports `--release`, `--locked`, and `--target <triple>`, and skips mobile targets.

### Cargo.lock Must Be Committed

`Cargo.lock` is committed (not gitignored) to ensure reproducible CI builds. Without it, CI resolves fresh dependency versions that may break — e.g., `webkit2gtk` updates that are incompatible with `wry` 0.24.x.

### Cargo.lock Must Be Regenerated After Version Bumps

When `Cargo.toml` version changes, `Cargo.lock` must be regenerated with `cargo generate-lockfile` (not just `cargo update -p grafyn`). The lockfile must satisfy `--locked` for all 4 release targets (Windows x64/ARM64, macOS ARM64, Linux x64) and both feature sets (default features for desktop app, `--no-default-features --features mcp` for MCP binary). The `npm run release:prepare` script handles this automatically.

### Desktop Configuration and Data Origin

`tauri.conf.json` is Android-neutral. Desktop builds must merge `tauri.desktop.conf.json` so the MCP external binary, updater endpoint/public key, `createUpdaterArtifacts: "v1Compatible"`, and desktop capability are present. The main desktop window keeps `useHttpsScheme: true`; changing it would reset the Tauri v1 HTTPS webview origin's IndexedDB, localStorage, and cookies for existing users.

### ESLint `_` Prefix Convention

The project's `.eslintrc.cjs` uses `argsIgnorePattern: '^_'` / `varsIgnorePattern: '^_'` / `destructuredArrayIgnorePattern: '^_'` for the `no-unused-vars` rule. Prefix intentionally unused variables with `_` to suppress lint errors.

## Release Rules

### Two-Phase Release Flow

Releases use a prepare → merge → tag workflow. Never push a tag before the version bump PR is merged to main.

1. `npm run release:prepare -- X.Y.Z` on a release branch (bumps versions, regenerates Cargo.lock, validates, commits)
2. Push the branch, open a PR, let CI pass, merge
3. `npm run release:tag -- X.Y.Z` on clean main (verifies, creates annotated tag)
4. `git push origin vX.Y.Z` triggers the release workflow

### Release Scripts

From `frontend/`:
- `npm run release:verify` — validates version alignment + Cargo.lock against all release targets
- `npm run release:prepare -- X.Y.Z` — version bump + lockfile regen + validation + commit (use on release branch)
- `npm run release:tag -- X.Y.Z` — final tag creation (use on clean main after PR merge)

### Release Invariants

- Never hand-edit version numbers for releases — use the release scripts
- Never reuse a release version/tag
- Never push directly to main — all changes go through PRs
- `Cargo.lock` must be regenerated with `cargo generate-lockfile` after any `Cargo.toml` version change
- The updater manifest (`latest.json`) is generated by `scripts/generate-updater-manifest.cjs`, not by Tauri's built-in generator
- See `WORKING_GUIDE.md` for the complete release workflow and troubleshooting

## Deployment

**Build output:** `frontend/src-tauri/target/release/bundle/` (NSIS `.exe`, DMG, DEB, or AppImage)

**Data location:** `~/Documents/Grafyn/` (`vault/` for notes, `data/` for indexes)

## Working conventions (added 2026-07-02 from session-friction audit)

- **CI: never poll PR checks in a loop.** After opening a PR, run `gh pr merge <PR> --auto --squash` once — GitHub merges automatically when checks pass. If a watch is genuinely needed, use `gh pr checks <PR> --watch` in the background, not repeated status checks.
- **Shell discipline (Windows).** The Bash tool is POSIX-only; the PowerShell tool is PS-only. Never PS cmdlets (`Select-Object`, `Select-String`) in bash, never bash idioms (`tail`, `$VAR=$(...)`, heredocs) in PowerShell. Windows paths in bash need forward slashes or quoting — unquoted backslashes get stripped.
- **Read before editing.** Always Read a file in-session before Edit/Write; "File has not been read yet" failures were the most repeated tool error in this repo's sessions.
- **Discussion-first.** When the user is exploring a design ("lets chat more", strategic questions), discuss — do not start implementing until explicitly told to build. Tool-use rejections are usually redirects back to discussion, not vetoes.
- **Local models:** before benchmarking a new Ollama tag, smoke-test it first (prompt-echo check, choice-extraction check, max-token truncation check) — these three failure modes consumed entire past sessions.
- **Source file size.** Keep new/edited source files under ~1,500 lines as an authoring target; CI hard-fails the `lint` job at 2,500 lines (`npm run check:file-sizes`, excludes `__tests__/`, `*.spec.js`, and Rust test-only files by path convention). New cross-cutting write paths must go through `commit_note_write`/`commit_note_delete` (`commands/mod.rs`) and `services/atomic_io.rs::write_atomic` — see "Note Write & Delete Chokepoints" above — not ad hoc file writes. When a file needs splitting, follow the mod-facade pattern already established by `commands/canvas/` (and `services/twin/`): one directory module, a `mod.rs` that re-exports, and sibling files split by concern (not by arbitrary line count).
