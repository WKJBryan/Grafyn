# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

| Component | Stack | Entry Point | Port |
|-----------|-------|-------------|------|
| **Backend (Rust)** | Tauri, Tantivy, petgraph, reqwest | `frontend/src-tauri/src/main.rs` | N/A |
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

**Prerequisites:** See [Tauri v1 prerequisites](https://v1.tauri.app/v1/guides/getting-started/prerequisites). Also need Rust via [rustup](https://rustup.rs/). Generate app icons with `npm run generate-icons`.

```bash
cd frontend
npm install
npm run tauri:dev        # Dev mode with hot reload
npm run tauri:build      # Production build → src-tauri/target/release/bundle/
```

Environment: `set OPENROUTER_API_KEY=your-key` (Windows) or `export OPENROUTER_API_KEY=your-key`

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

## Architecture Overview

Grafyn is a **desktop-only** app — a single Tauri binary with a Vue frontend and Rust backend. No web mode, no Python backend.

```
┌────────────────────────────────────────────────┐
│           Tauri Desktop App (Single Binary)     │
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

16 modules in `frontend/src-tauri/src/commands/`. Enumerate exact command names with `grep -rn "#\[tauri::command\]" -A1 src/commands/` — purposes only below, to avoid drift.

| Module | Purpose |
|--------|---------|
| `notes.rs` | Note CRUD |
| `search.rs` | Full-text search, find-similar, reindex |
| `graph.rs` | Link graph: backlinks, outgoing, neighbors, unlinked, full graph, rebuild |
| `canvas.rs` | Multi-LLM canvas (18 commands) with note context; streaming via `canvas-stream` Tauri events |
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

### Frontend

- `src/api/client.js` — all backend calls go through Tauri `invoke()`; exports one namespace per command module plus `optimizer` and `isDesktopApp`
- **Pinia stores (3):** `canvas.js`, `theme.js`, `boot.js` — there is no twin store; twin UI state lives in `TwinReviewView.vue` and `stores/canvas.js`
- **Routes:** `/` (notes), `/canvas`, `/canvas/:id`, `/import`, `/twin` (component: `TwinReviewView.vue`), plus catch-all → `NotFoundView.vue`
- **Tests:** `src/__tests__/{unit,integration,fixtures}/` + `setup.js` (Vitest)

## Key Concepts

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

Compare responses from multiple LLM models simultaneously via OpenRouter. Features: parallel model streaming, infinite canvas with D3.js zoom/pan, model debate mode, session persistence in `data/canvas/`, **semantic note context** (retrieves relevant notes as LLM system prompt).

**Semantic context mode:** When `context_mode == Semantic` (the default), `send_prompt` runs a two-stage pipeline: (1) note-level retrieval as a quality gate, (2) if `chunk_retrieval_enabled` (default: `true`), chunk-level retrieval fills relevant paragraphs within `default_token_budget` (default: 4000 tokens). Falls back to whole-note truncation (1500 chars) if chunks are empty or disabled. Pinned notes per session (`pinned_note_ids`) are always included. Context notes are stored on the tile and emitted via `ContextNotes` event for frontend display.

**Streaming architecture:** Commands return immediately, spawn async tasks, stream via `canvas-stream` Tauri events (`TileCreated`, `ContextNotes`, `Chunk`, `Complete`, `Error`, `SessionSaved`, debate variants). Frontend listens via `@tauri-apps/api/event`.

Streaming commands: `send_prompt`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response`

### Twin Identity, Constitution, And Decision Mirror

Twin context mode is a native RAG path, not model-weight training. `frontend/src-tauri/src/commands/canvas.rs` assembles the model-facing prompt through `build_twin_context_prompt()`.

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

**Web search:** When `web_search: true`, OpenRouter's `plugins: [{"id": "web", "max_results": 5}]` is added to the API request (~$0.02/query per model). The `web_search` flag is threaded through the full stack and persisted on canvas tiles for regenerate/add-model replay.

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

### Background Services

These services run automatically in the background and have dedicated inbox/decision/rollback APIs. Do not re-implement any of these — they already exist.

**`services/link_discovery.rs`** — background link-discovery worker. Distinct from on-demand zettelkasten discovery (`discover_links` command). Uses YAKE keyword extraction (`services/yake.rs`) and TF-IDF cosine similarity (`services/similarity.rs`) to find wikilink candidates without LLM calls. Optional LLM pass controlled by `background_link_discovery_llm_enabled`. Results surface via `list_link_suggestion_queue` / `dismiss_link_suggestion` commands.

**`services/vault_optimizer.rs`** — background vault optimizer. Queued via `enqueue_vault_optimizer_note()` whenever notes are created or migrated. Processes the queue and applies structural improvements in two modes: `sidecar_first` (overlay metadata) or `full_rewrite`. Budget caps and daily write limits prevent runaway LLM spend. Decisions are auditable via `list_vault_optimizer_decisions`; rollbacks are per-change via `rollback_vault_optimizer_change`.

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
- **`background_vault_optimizer_enabled`** — controls the `VaultOptimizerService`. When enabled, optimizer processes queued notes and applies structural improvements (sidecar overlay or full rewrite mode). Budget and max daily writes cap LLM costs.
- **Vault optimizer sub-settings** (all on `UserSettings`): `background_vault_optimizer_llm_enabled`, `_budget_monthly`, `_max_daily_writes`, `_edit_mode`, `_program_enabled`, and `vault_optimizer_program_path` — the last two enable a **vault-local `program.md` policy file** that steers optimizer behavior per-vault.
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

`.github/workflows/test.yml` — runs on push to main and PRs. Jobs: `release-preflight` (version + Cargo.lock alignment), `rust-tests` (ubuntu-22.04), `frontend-tests` (Vitest), `lint` (eslint + `cargo clippy -D warnings`), `security` (`npm audit --audit-level=high`), `build` (Vite), `test-summary`. Clippy and npm audit **block** PRs.

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

### Tauri v1 Requires Ubuntu 22.04

Tauri v1 depends on `libwebkit2gtk-4.0-dev` which **does not exist on Ubuntu 24.04** (`ubuntu-latest`). The `rust-tests` and `lint` CI jobs must use `runs-on: ubuntu-22.04`. Do NOT use `ubuntu-latest` for any job that compiles Tauri Rust code.

- `libwebkit2gtk-4.1-dev` (Ubuntu 24.04) does NOT satisfy Tauri v1's `webkit2gtk-sys` crate — it provides `webkit2gtk-4.1.pc` but Tauri v1 needs `webkit2gtk-4.0.pc`.
- The `linux-ipc-protocol` Tauri feature is **Tauri v2 only** — do not attempt to use it with Tauri v1.8.

### Rust CI Requires MCP Binary + Stub dist/

`cargo test` compiles the full crate including `tauri::generate_context!()`. Two prerequisites must exist before running tests:

1. **MCP binary** — Tauri's `externalBin` config expects `binaries/grafyn-mcp-{target-triple}` at compile time. Use the script that CI uses:
   ```bash
   cd frontend && npm run prepare:sidecar
   ```
   (Builds `grafyn-mcp` with `--no-default-features --features mcp` and copies it to `binaries/grafyn-mcp-<host-triple>`. Supports `--release`, `--locked`, `--target <triple>`.)
2. **Stub dist directory** — `tauri::generate_context!()` panics if `distDir` (configured as `../dist`) doesn't exist.
   ```bash
   mkdir -p ../dist && echo '<html></html>' > ../dist/index.html
   ```

### Cargo.lock Must Be Committed

`Cargo.lock` is committed (not gitignored) to ensure reproducible CI builds. Without it, CI resolves fresh dependency versions that may break — e.g., `webkit2gtk` updates that are incompatible with `wry` 0.24.x.

### Cargo.lock Must Be Regenerated After Version Bumps

When `Cargo.toml` version changes, `Cargo.lock` must be regenerated with `cargo generate-lockfile` (not just `cargo update -p grafyn`). The lockfile must satisfy `--locked` for all 4 release targets (Windows x64/ARM64, macOS ARM64, Linux x64) and both feature sets (default features for desktop app, `--no-default-features --features mcp` for MCP binary). The `npm run release:prepare` script handles this automatically.

### Tauri Features Must Include `process-all` and `protocol-all`

Removing `process-all` or `protocol-all` from the Tauri features in `Cargo.toml` changes the `wry`/`webkit2gtk` feature graph and breaks the Linux build. The `wry` crate's webkitgtk code depends on `SettingsExt` trait methods that are only in scope when these features are enabled.

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
