# Grafyn Production-Readiness Refactor Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Every finding below carries file:line anchors from the 2026-07-09 audit; re-read the anchored code before writing the fix — do not code from this document alone.

**Goal:** Make Grafyn production-ready for its public, twin-first mission: fix all verified critical/high bugs, make twin + vault data integrity bulletproof, consolidate duplicated write paths, delete misleading cruft, and split the three oversized modules — without changing product behavior except where a bug *is* the behavior.

**Architecture:** Five phases, each an independent worktree branch → PR → CI → auto-merge. Phases are ordered so data-integrity fixes land before structural moves (so bug-fix diffs stay reviewable and don't conflict with file splits). No new dependencies, no new features.

**Tech Stack:** Rust (Tauri v1, Tantivy, tokio), Vue 3 + Pinia + Vitest, Playwright (e2e), GitHub Actions.

**Mission context (owner decision 2026-07-09):** Twin-first — the twin is the product; vault + Canvas are the capture layer. Priority: twin data integrity > canvas polish > PKM convenience. Twin *accuracy evaluation* stays external (owner decision 2026-06-10) — nothing in this plan adds in-app scoring.

## Global Constraints

- One phase = one worktree branch = one PR (owner's branch-workflow rule; use `superpowers:using-git-worktrees`).
- Never push or open a PR without explicit owner approval; once CI is green on an approved PR, auto-merge (`gh pr merge --auto --squash`).
- `cargo generate-lockfile` before every push (smoke test checks lockfile freshness).
- Rust CI prerequisites before `cargo test`: `cd frontend && npm run prepare:sidecar` + stub `dist/index.html`.
- Rust pinned at 1.93.1; all Tauri-compiling CI jobs on ubuntu-22.04. Do not touch Tauri features `process-all`/`protocol-all`.
- TDD per task: failing test → minimal fix → green → commit. `cargo clippy -D warnings` and eslint must stay clean (both block PRs).
- No behavior changes beyond the enumerated fixes; no re-implementation of existing background services.

---

## Phase 0 — Repo hygiene & honest docs (small, zero-risk, do first)

**Branch:** `chore/repo-hygiene`

### Task 0.1: Delete tracked Python/backend fossils

**Files:** Delete: `pyproject.toml`, `.python-version`, `package-lock.json` (root — empty stub named "Seedream", no root `package.json` exists).

- [ ] Verify nothing references them: `grep -rn "pyproject\|uv.lock" .github/ frontend/ e2e/ --include="*.yml" --include="*.json" --include="*.cjs"` → expect no hits
- [ ] `git rm pyproject.toml .python-version package-lock.json`
- [ ] Add `.venv/`, `uv.lock`, `nul` confirmation to `.gitignore` if not already covered (they are — verify only)
- [ ] Commit: `chore: remove dead Python-backend artifacts from repo root`

### Task 0.2: Relocate or delete orphaned splash PNGs

**Files:** `splash-hires-logo.png`, `splash-preview.png`, `splash-with-logo.png` (root, ~240 KB each, zero code references).

- [ ] Ask owner at PR time: keep as `docs/assets/` (GitHub social preview) or delete. Default action in this branch: `git mv` to `docs/assets/`.
- [ ] Commit: `chore: move orphaned splash images out of repo root`

### Task 0.3: Remove dead frontend code

**Files:** Delete: `frontend/src/components/canvas/PromptTile.vue`, `DebateTile.vue`, `DebateControls.vue`, `frontend/src/stores/notes.js`. Modify: `frontend/src/components/canvas/index.js` (drop the three exports).

- [ ] Verify each is truly unimported: `grep -rn "PromptTile\|DebateTile\|DebateControls" frontend/src --include="*.vue" --include="*.js"` — only `index.js` exports and self-references may appear. Same for `stores/notes` (`grep -rn "stores/notes" frontend/src`).
- [ ] **Caution:** `DebateTile.vue:237` documents the correct `continue` emit contract — capture that snippet into the Task 3.2 notes before deleting.
- [ ] Delete files, fix `index.js`, run `npm run test:run` (230 tests must stay green; delete any specs that only cover removed components).
- [ ] Commit: `chore: remove dead canvas components and unused notes store`

### Task 0.4: Honest README maturity labels (twin-first requires honest twin claims)

**Files:** Modify: `README.md` (status table).

- [ ] Change twin rows from "✅ Stable" to "🧪 Experimental" (or "Beta") for: Native RAG twin, Twin Identity/Constitution/Decision Mirror, Twin evidence capture. Rationale line: capture/export is stable; accuracy machinery (semantic retrieval, temporal validity, calibration) is roadmap (`TWIN_ACCURACY_ROADMAP.md`).
- [ ] Reconcile the "semantic note context" wording: current retrieval is lexical (Tantivy BM25 + graph boosts); do not claim embedding-based semantics.
- [ ] Commit: `docs: align README twin maturity labels with TWIN_ACCURACY_ROADMAP reality`

### Task 0.5: Wire e2e suite into CI as a non-blocking job

**Files:** Modify: `.github/workflows/test.yml`. Reference: `e2e/package.json`, `e2e/playwright.config.js` (6 specs).

- [ ] Add an `e2e` job on ubuntu-22.04 with `continue-on-error: true` (informational first; promote to blocking after one week of green runs). It needs the Tauri build prerequisites — if runtime cost is prohibitive, instead add an npm script `e2e` in `frontend/package.json` and a README note that the suite is manual; decide at execution time based on measured runtime.
- [ ] Commit: `ci: run e2e Playwright suite (non-blocking)`

**Phase 0 acceptance:** repo root contains no Python artifacts, no orphaned assets, no dead components; README twin claims match roadmap; CI unchanged-or-stronger.

---

## Phase 1 — Data integrity (CRITICAL/HIGH backend, twin-first core)

**Branch:** `fix/data-integrity`

### Task 1.1: Atomic write utility, adopted by every persistence site

**Files:** Create: `frontend/src-tauri/src/services/atomic_io.rs`. Modify: `services/knowledge_store.rs:634`, `services/canvas_store.rs:506`, `services/settings.rs:260`, `services/twin_store.rs:2643,2654`, `services/vault_optimizer.rs:344+`, `services/retrieval.rs:168`, `services/priority.rs:71`. Test: unit tests in `atomic_io.rs` + one adoption test per store.

**Interfaces — Produces:** `pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()>` — writes to `{path}.tmp-{pid}` in the same directory, fsyncs, then `std::fs::rename` over the target (rename is atomic on NTFS/ext4 same-volume).

- [ ] Failing test: write file, simulate interrupt by asserting no partial target exists when writer errors mid-way (inject via zero-length tmp dir permission trick or trait seam); plus happy-path replaces existing content.
- [ ] Implement `write_atomic`; replace every direct `std::fs::write` at the listed sites (grep `fs::write` across `src-tauri/src` to catch stragglers; audit found 7+ sites).
- [ ] `cargo test` green; commit: `fix: atomic temp+rename writes for all persisted JSON and notes`

### Task 1.2: Migration rollback actually restores notes (CRITICAL C1)

**Files:** Modify: `frontend/src-tauri/src/services/markdown_migration.rs` — `apply()` (220–407), `backup_note()` (540–569, the `if manifest exists` guard at 559), `rollback()` (437–476). Test: new integration test in the same file's `#[cfg(test)]`.

**Root cause:** `backup_note()` only records into `manifest.backup_files` when `manifest.json` already exists on disk, but the manifest is written *after* all backups → `backup_files` is always empty → `rollback()` restores nothing while reporting success.

- [ ] Failing test: run `apply()` in FullRewrite mode on a temp vault (2 notes), then `rollback()`; assert note contents byte-equal originals. Currently fails.
- [ ] Fix: accumulate backup records in the in-memory `manifest.backup_files` during `apply()` (drop the exists-on-disk guard), write manifest once complete; make `rollback()` restore every `backup_files` entry (copy back, then delete created hubs/overlays as today). Rollback must be idempotent.
- [ ] Also add a guard: `rollback()` returns an error (not success) if `backup_files` is empty but `rewritten_notes > 0` — never report a no-op rollback as success again.
- [ ] Commit: `fix: markdown-migration rollback restores backed-up notes (was a silent no-op)`

### Task 1.3: Kill the ABBA deadlock (H1)

**Files:** Modify: `frontend/src-tauri/src/commands/migration.rs:188-189` (`rollback_vault_optimizer_change`). Reference (do not change): `main.rs:484-486` worker order = `knowledge_store` → `vault_optimizer`.

- [ ] Establish and document the canonical lock order in `commands/mod.rs` doc comment: **knowledge_store before vault_optimizer, always**.
- [ ] Reorder acquisitions in `rollback_vault_optimizer_change` to match. Grep for any other multi-lock sites (`rg -U "write\(\)[\s\S]{0,200}write\(\)" src-tauri/src`) and normalize; audit found these two as the only pair.
- [ ] Test: this is hard to unit-test deterministically; add a `#[test]` that asserts via code comment + a loom-free smoke (spawn both paths 100× against tiny fixtures with a 5s timeout). If flaky, the doc-comment + reorder + review suffices — note it in the PR.
- [ ] Commit: `fix: normalize knowledge_store→vault_optimizer lock order (ABBA deadlock)`

### Task 1.4: Windows path-escape hardening (H3 — reachable from MCP)

**Files:** Modify: `frontend/src-tauri/src/services/knowledge_store.rs` — `validate_note_id` (415–420), `normalize_note_relative_path` (706–727). Test: same file.

- [ ] Failing tests (Windows-semantics, but validators are pure string checks so they run everywhere): reject ids/paths containing `:` (drive-relative `C:foo`, ADS `foo:bar`); reject reserved device stems case-insensitively (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`, with or without extension); keep accepting normal unicode titles.
- [ ] Implement: extend both validators; after join, add a belt-and-braces canonical-prefix check — resolved path must start with the vault path (`dunce::canonicalize` or manual component walk; no new deps — component walk).
- [ ] Verify both entry surfaces are covered: Tauri commands (`commands/notes.rs`) and MCP (`mcp_tools.rs`) both route through `KnowledgeStore`, so the store-level fix covers both — confirm by reading the call sites.
- [ ] Commit: `fix: reject drive-relative, ADS, and reserved-device note ids/paths`

### Task 1.5: Corrupt-file resilience — settings, twin store, sealed predictions (M1/M2/M5)

**Files:** Modify: `frontend/src-tauri/src/services/settings.rs:58`, `services/twin_store.rs:2309-2329` (`ensure_record_cache`), `2250-2261` (`list_session_traces`), `4089-4091` (sealed-prediction JSON extraction). Test: each in-module.

- [ ] `settings.rs:58` — replace `unwrap_or_default()`: on parse failure, rename the corrupt file to `settings.json.corrupt-{timestamp}`, log an error, then fall back to defaults. Never silently overwrite the only copy. Test: corrupt JSON → defaults returned AND `.corrupt-` file exists.
- [ ] `twin_store.rs` cache/list loops — replace `?` propagation with per-file skip + quarantine (rename to `{file}.corrupt-{ts}`) + collected warning list, so one truncated record never bricks the whole Twin Workspace. Test: directory with 2 valid + 1 truncated record → 2 records returned, 1 quarantined.
- [ ] `twin_store.rs:4089` — guard `start <= end` before slicing `&raw[start..=end]`; on failure call the existing `mark_twin_prediction_failed` path (the code's own comment at 852–854 demands this for eval integrity). Test: input `"Option A} — but {incomplete"` → prediction marked failed, no panic.
- [ ] Commit: `fix: quarantine corrupt persisted files instead of resetting or crashing`

### Task 1.6: Frontmatter must survive parse errors (M3)

**Files:** Modify: `frontend/src-tauri/src/services/knowledge_store.rs:436-441`. Test: same file.

- [ ] Failing test: note with malformed YAML (stray tab) → read succeeds with defaults, then `update_note` on it → original raw frontmatter block still present on disk (or update refused), not replaced by defaults.
- [ ] Implement: when frontmatter fails to deserialize, retain the raw frontmatter text on the in-memory note (new `raw_frontmatter: Option<String>` field or a `frontmatter_parse_failed` flag); on write, if the flag is set and the caller didn't explicitly edit frontmatter, re-emit the original raw block verbatim and log a warning. Choose the simplest variant that passes the test without touching serde schemas of persisted structs.
- [ ] Commit: `fix: preserve unparsable YAML frontmatter instead of silently destroying it`

**Phase 1 acceptance:** power-loss/corrupt-file scenarios never lose vault or twin data; rollback provably restores; no deadlock pair; MCP cannot write outside the vault. `cargo test` + clippy green.

---

## Phase 2 — Core-flow correctness (backend HIGH/MEDIUM)

**Branch:** `fix/core-flows`

### Task 2.1: Single note-write chokepoint; fix the reindex regression (H2)

**Files:** Modify: `frontend/src-tauri/src/commands/notes.rs:27-90`, `commands/zettelkasten.rs` (`apply_links`, `create_link`), `commands/mod.rs`. Reference implementations that already do it right: `commands/canvas.rs:829-850` (`export_to_note`), `commands/import.rs:174-195`, `commands/distill.rs`, `mcp_tools.rs`.

**Approach:** extract a `pub(crate) async fn commit_note_write(state, note_id)` helper in `commands/mod.rs` that performs the full post-write sequence: `search.index_note` + `sync_chunk_index_for_note` + `sync_topic_hubs` + `enqueue_vault_optimizer_note`. Convert all six call sites (notes create/update, zettelkasten ×2, canvas export, import, distill, MCP) to use it — five hand-rolled copies is how this regression happened.

- [ ] Failing test: `create_note` then `search_notes` for its content → currently no hit; after fix, hit. Same for `update_note` with new content.
- [ ] Implement helper; migrate call sites; delete the now-dead inline sequences.
- [ ] Commit: `fix: reindex notes on create/update via single commit_note_write chokepoint`

### Task 2.2: Boot backfill idempotence (M4)

**Files:** Modify: `frontend/src-tauri/src/services/markdown_migration.rs:478-507` (`backfill_legacy_grafyn_notes`).

- [ ] Failing test: single-word-title note → run backfill twice → file mtime/`updated_at` unchanged on second run.
- [ ] Fix: skip when alias candidates are empty (not only when `aliases` is non-empty); write only if the computed update actually differs from current frontmatter; preserve `updated_at` for backfill-only changes.
- [ ] Commit: `fix: make boot alias backfill idempotent (was rewriting notes every launch)`

### Task 2.3: Vault optimizer honors its own caps and stops hogging the lock (M6, M8)

**Files:** Modify: `frontend/src-tauri/src/services/vault_optimizer.rs` (`run_next`, 218–340), `main.rs:483-487` (worker loop).

- [ ] Failing tests: with `background_vault_optimizer_max_daily_writes = 2`, third write in a day is deferred; with `_llm_enabled = false`, no LLM call is attempted (assert via provider stub/flag).
- [ ] Implement: read `_max_daily_writes` (persist a daily counter), `_llm_enabled`, and `_budget_monthly` in `run_next`; move queue-entry removal to *after* successful processing (re-queue on error); narrow the worker's lock scope — snapshot what's needed under `knowledge_store.read()`, do LLM work lock-free, take `write()` only to apply (mirror the link-discovery worker's pattern at `link_discovery.rs:1169-1365`, which the audit verified as correct).
- [ ] Commit: `fix: vault optimizer enforces write caps and releases locks during LLM calls`

### Task 2.4: Streaming pipeline robustness (M7, L1, L2, L3)

**Files:** Modify: `frontend/src-tauri/src/commands/canvas.rs:566,1219,1227,1514,1765` (swallowed `let _ =` persistence results), `canvas.rs:1907` (`regenerate_response` missing 60s timeout), `services/openrouter.rs:240,301`, `services/ollama.rs:209` (UTF-8 chunk boundaries, dropped mid-stream SSE errors).

- [ ] Persistence failures: on `batch_update_tile_responses` error, emit a `canvas-stream` `Error` event for the tile (frontend already renders those) and `log::error` — never emit `SessionSaved` after a failed save.
- [ ] `regenerate_response`: wrap stream reads in `tokio::time::timeout(60s, …)` exactly like `send_prompt` does.
- [ ] UTF-8: buffer raw bytes and decode with a carry-over of incomplete trailing sequences (`str::from_utf8` error `valid_up_to`) in both providers. Test: feed a 2-chunk split of a CJK char → no U+FFFD.
- [ ] SSE: in `parse_sse_chunk`, if a `data:` payload parses as `{"error": …}`, surface it as a stream error instead of ignoring. Test with a synthetic error event.
- [ ] Commit: `fix: canvas stream persistence errors surface; UTF-8-safe chunk decoding; SSE errors propagate`

**Phase 2 acceptance:** create→search round-trip passes; optimizer respects caps; no per-boot vault rewrites; streamed responses either persist or visibly error.

---

## Phase 3 — Frontend canvas & views (HIGH/MEDIUM frontend)

**Branch:** `fix/frontend-canvas`

### Task 3.1: Scope stream handling per tile; fix streaming refcounts (F1 — HIGH)

**Files:** Modify: `frontend/src/stores/canvas.js` (`setupTauriStreamListener` 238–249, `sendPrompt` handlers 318–364, `addModelToTile` 837–865, `regenerateResponse` 893–927, `removeStreaming` 42). Test: `frontend/src/__tests__/unit/stores/canvas.spec.js` (extend).

- [ ] Failing test: simulate two concurrent `sendPrompt` streams (same session, same model id, different tiles) with interleaved `chunk`/`complete` events → each tile receives only its own content; both tiles end `complete`; no duplicate tile ids in `prompt_tiles`.
- [ ] Implement: every per-operation handler filters on the operation's `tile_id`(s) (captured from its own `tile_created`); `tile_created` push is deduped by tile id; streaming state becomes a `Set` keyed `"${tileId}:${modelId}"`; remove the double-decrement (`finally` blocks at 359/865/927 must not decrement what the `complete` handler already did — single ownership).
- [ ] Also fix `CanvasContainer.vue:224` `:is-streaming` to use the composite key.
- [ ] Commit: `fix: scope canvas stream events per tile; keyed streaming state; no double-decrement`

### Task 3.2: Repair debate Continue (F2 — HIGH)

**Files:** Modify: `frontend/src/components/canvas/DebateNode.vue:94` (emit), verify `CanvasContainer.vue:1531` (`handleDebateContinue`) and `stores/canvas.js:722`. Contract reference: the deleted `DebateTile.vue:237` emitted `emit('continue', props.debate.id, prompt)` — DebateNode needs a prompt input (small inline field or reuse of PromptDialog) before emitting.

- [ ] Failing test: component test asserting `continue` emits `[debateId, prompt]` with non-empty prompt; store test asserting `continueDebate` rejects empty prompt with a user-visible error instead of a silent console.error.
- [ ] Implement UI affordance + emit; replace the `console.error` swallow at `CanvasContainer.vue:1534-1536` with the app's standard error surface (whatever `sendPrompt` failures use).
- [ ] Commit: `fix: debate Continue sends a prompt and surfaces failures (was silently broken)`

### Task 3.3: ImportView owns its scroll (F3 — HIGH)

**Files:** Modify: `frontend/src/views/ImportView.vue:591-595`. Contract: `#app` is `overflow:hidden` (`src/style.css:177-181`); pattern reference: `TwinReviewView.vue:1536` (PR #85 fix).

- [ ] Fix: `.import-view { height: 100%; overflow-y: auto; }` (match the TwinReviewView pattern exactly).
- [ ] Verify with the app running (`npm run tauri:dev`) at a short window height: Apply button reachable. Add this view to whatever scroll-contract test exists; if none, add a unit test asserting the class rules.
- [ ] Commit: `fix: ImportView complies with the view scrolling contract`

### Task 3.4: Session lifecycle — saved-event reconciliation, viewport persistence, single loader, vault-switch reset (F4–F7)

**Files:** Modify: `frontend/src/stores/canvas.js:341-343,849-851,911-913` (session_saved → loadSession), `169` (loading flag), `frontend/src/components/canvas/CanvasContainer.vue:718-733,760-768` (viewport), `frontend/src/views/CanvasView.vue:249-253` (redundant watcher), `frontend/src/views/HomeView.vue:556-567` (vault switch).

- [ ] `session_saved`: replace wholesale `loadSession` with a silent refetch that does NOT set the global `loading` flag and merges server fields (`tokens_used`, normalized fields) without clobbering in-flight local streams or sub-150ms drag positions. Test: `session_saved` during an active drag does not revert position and shows no loading overlay.
- [ ] Viewport: persist via `updateViewport()` inside the `props.sessionId` watcher *before* loading the new session (not only `onBeforeUnmount`).
- [ ] Remove `CanvasView.vue`'s duplicate `route.params.id` watcher (container's `immediate: true` watcher is the single loader).
- [ ] Vault switch: in `handleSettingsSaved`/`handleSetupComplete`, when `changes.vaultPathChanged`, clear `selectedNoteId`/`selectedNote` and reload; also reload canvas sessions list.
- [ ] Commit: `fix: canvas session lifecycle — no loading flash, viewport persists, single loader, vault-switch reset`

### Task 3.5: Small leaks and silent errors (F8–F10)

**Files:** Modify: `frontend/src/components/GraphView.vue:233-273,393-404,502` (stop old simulation before replacing; don't inject `forceCenter` on resize — update `forceX`/`forceY` centers instead), `LLMNode.vue:712-716` + `DebateNode.vue:406-410` (+ PromptNode) (remove resize listeners on unmount), `HomeView.vue:184-191`, `GraphView.vue:200-202`, `stores/canvas.js:228-234` (surface load failures to the user via the app's standard error UI, not console-only).

- [ ] Implement each; run `npm run test:run` green.
- [ ] Commit: `fix: graph simulation cleanup, resize-listener removal, surfaced load errors`

**Phase 3 acceptance:** two concurrent prompts render correctly; debate Continue works end-to-end; ImportView scrolls; no loading flash on completion; Vitest suite green.

---

## Phase 4 — Modularity & lean structure (no behavior change)

**Branch:** `refactor/module-splits` — start only after Phases 1–3 merge (splits would poison those diffs).

### Task 4.1: Split `commands/canvas.rs` (176 KB)

**Files:** Create: `commands/canvas/mod.rs` (command surface, unchanged signatures — Tauri command names must not change), `canvas/streaming.rs` (send_prompt/add_models/regenerate stream tasks), `canvas/debate.rs`, `canvas/context.rs` (`build_note_context_prompt`, `build_twin_context_prompt`, `resolve_twin_prompt_context` — the twin gate stays here, with its tests), `canvas/session.rs` (CRUD/positions/viewport/export).

- [ ] Pure move-refactor: `git mv` + `pub(super)` plumbing; zero logic edits. `cargo test` + clippy green after each sub-move; existing unit tests move with their code.
- [ ] Commit per sub-move: `refactor: extract canvas::streaming` etc.

### Task 4.2: Split `services/twin_store.rs` (247 KB)

**Files:** Create: `services/twin/mod.rs` (TwinStore facade, same public API), `twin/records.rs`, `twin/constitution.rs`, `twin/decisions.rs` (episodes, sealed predictions — keep the seal-integrity logic and its comment block intact), `twin/digest.rs`, `twin/traces.rs`, `twin/export.rs`.

- [ ] Same pure-move discipline. Fix the read-lock mutation sites found in the audit (`commands/twin.rs:151,204,216` take `read()` for read-modify-write — change to `write()`) as the one permitted logic change, with a test for concurrent constitution updates not losing writes.
- [ ] Commits per sub-move.

### Task 4.3: Decompose `TwinReviewView.vue` (73 KB) and add a twin Pinia store

**Files:** Create: `frontend/src/stores/twin.js`, `frontend/src/components/twin/` (one component per workspace tab: records review, memory digest, constitution, action gaps, decision episodes, mirror config, guided setup — match the existing grouped nav rail from PR #85). Modify: `TwinReviewView.vue` shrinks to layout + routing between tabs.

- [ ] Move state/api calls into `stores/twin.js`; components consume the store. Behavior-identical; Vitest component tests move/extend accordingly.
- [ ] Commit per extracted tab.

### Task 4.4: Guardrails so it stays lean

**Files:** Modify: `CLAUDE.md` (convention note), optionally `.github/workflows/test.yml`.

- [ ] Add convention to CLAUDE.md: source files should stay under ~1,500 lines; new cross-cutting write paths must go through `commit_note_write`/`write_atomic`.
- [ ] Optional CI tripwire: a 10-line script failing the lint job when any `src-tauri/src/**/*.rs` or `src/**/*.{vue,js}` exceeds 2,000 lines (excluding tests). Decide with owner at PR time.

**Phase 4 acceptance:** no public behavior change (full Rust + Vitest + e2e suites green before/after); largest source file < 2,000 lines; twin state has a real store.

---

## Phase 5 — Verification & release

**Branch:** none (runs on main after merges) + `release/vNext` when cutting.

- [ ] Full matrix locally: `cargo test` (with sidecar + stub dist), `npm run test:run`, `npm run lint`, `cargo clippy -D warnings`, `cd e2e && npm test`.
- [ ] Manual twin-first smoke: create note → search hit (Task 2.1); run migration → rollback → verify restore (Task 1.2); two concurrent canvas prompts (Task 3.1); debate continue (Task 3.2); import a DOCX and scroll (Task 3.3); kill the app mid-save and relaunch (Task 1.1/1.5 — no data loss, no setup-wizard reset).
- [ ] Release via the two-phase flow (`release:prepare` on branch → PR → merge → `release:tag`); verify updater endpoint after CI.

---

## Explicitly deferred (tracked, not in this refactor)

- **L8** MCP-created notes invisible to the running app until restart — needs a file-watcher or IPC nudge; design work, twin-relevant but not data-loss.
- Embedding-based retrieval (`SimilarityProvider::encode_batch`) — roadmap Phase work, owner's external-eval boundary applies.
- Deeper decision-ledger/bitemporal features from TWIN_ACCURACY_ROADMAP — feature work, not refactor.

## Self-review notes

- Every audit CRITICAL/HIGH maps to a task: C1→1.2, H1→1.3, H2→2.1, H3→1.4, F1→3.1, F2→3.2, F3→3.3. Mediums: M1→1.1/1.5, M2→1.5, M3→1.6, M4→2.2, M5→1.5, M6/M8→2.3, M7→2.4, F4–F7→3.4, F8–F10→3.5. Lows L1–L3→2.4, L4/L5/L6/L7→L7 folded into 4.2; L4 (dialog thread-block) and L5/L6 are accepted-risk unless touched incidentally — call out in PR if adjacent.
- Line anchors are from the 2026-07-09 audit; implementers must re-verify against HEAD before editing.
