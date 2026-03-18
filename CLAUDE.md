# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

| Component | Stack | Entry Point | Port |
|-----------|-------|-------------|------|
| **Backend (Rust)** | Tauri, Tantivy, petgraph, reqwest | `frontend/src-tauri/src/main.rs` | N/A |
| **MCP Server** | rmcp, Tantivy, stdio transport | `frontend/src-tauri/src/mcp.rs` | stdio |
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

**Prerequisites:** See [Tauri v1 prerequisites](https://v1.tauri.app/v1/guides/getting-started/prerequisites). Also need Rust via [rustup](https://rustup.rs/). Generate app icons with `node scripts/generate-icons.cjs`.

```bash
cd frontend
npm install
npm run tauri:dev        # Dev mode with hot reload
npm run tauri:build      # Production build → src-tauri/target/release/bundle/
```

Environment: `set OPENROUTER_API_KEY=your-key` (Windows) or `export OPENROUTER_API_KEY=your-key`

**Version bump:** `npm run version:bump -- X.Y.Z` updates `package.json`, `tauri.conf.json` (version + window title), and `Cargo.toml` in one step.

### Testing

```bash
# Rust tests
cd frontend/src-tauri
cargo test

# Frontend unit tests
cd frontend
npm run test:run
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
│  │   settings, feedback, mcp, memory,         │  │
│  │   import, priority, retrieval, zettelkasten)│  │
│  └──────────────────────────────────────────┘  │
│  ~/Documents/Grafyn/                          │
│  ├── vault/  (markdown notes)                   │
│  └── data/   (search index, canvas, settings)   │
└────────────────────────────────────────────────┘
```

### Tauri IPC Commands (65 total across 13 modules)

| Module | Commands | Purpose |
|--------|----------|---------|
| `commands/notes.rs` | `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note` | Note CRUD |
| `commands/search.rs` | `search_notes`, `find_similar`, `reindex` | Full-text search (graph-aware similarity) |
| `commands/graph.rs` | `get_backlinks`, `get_outgoing`, `get_neighbors`, `get_unlinked`, `get_full_graph`, `rebuild_graph` | Link graph |
| `commands/canvas.rs` | `list_sessions`, `get_session`, `create_session`, `update_session`, `delete_session`, `get_available_models`, `send_prompt`, `update_tile_position`, `delete_tile`, `delete_response`, `update_viewport`, `update_llm_node_position`, `auto_arrange`, `export_to_note`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response` | Multi-LLM canvas with note context (streaming via `canvas-stream` Tauri events) |
| `commands/distill.rs` | `distill_note`, `normalize_tags` | LLM + rules-based distillation with dedup and hub creation |
| `commands/settings.rs` | `get_settings`, `get_settings_status`, `update_settings`, `complete_setup`, `pick_vault_folder`, `validate_openrouter_key`, `get_openrouter_status` | App settings & first-run setup |
| `commands/feedback.rs` | `submit_feedback`, `get_system_info`, `feedback_status`, `get_pending_feedback`, `retry_pending_feedback`, `clear_pending_feedback` | Feedback with offline queue |
| `commands/mcp.rs` | `get_mcp_status`, `get_mcp_config_snippet` | MCP config for Claude Desktop |
| `commands/memory.rs` | `recall_relevant`, `find_contradictions`, `extract_claims` | Memory recall & contradiction detection |
| `commands/priority.rs` | `get_priority_settings`, `update_priority_settings`, `reset_priority_settings` | Configurable search result ranking |
| `commands/retrieval.rs` | `retrieve_relevant`, `get_retrieval_config`, `update_retrieval_config` | Temporal + graph-aware retrieval pipeline |
| `commands/zettelkasten.rs` | `discover_links`, `apply_links`, `create_link`, `get_link_types` | Zettelkasten link discovery |
| `commands/import.rs` | `preview_import`, `apply_import`, `get_supported_formats` | Conversation import (ChatGPT, Claude, Grok, Gemini) |

### Frontend API Client

```javascript
// src/api/client.js — all calls go through Tauri IPC
import { notes, search, graph, canvas, settings, mcp, memory,
         zettelkasten, feedback, priority, retrieval, importApi, isDesktopApp } from '@/api/client'

// Every function calls invoke() directly to the Rust backend
// Canvas streaming uses canvas-stream Tauri events (including ContextNotes for semantic mode)
```

**Pinia Stores:** `notes.js`, `canvas.js`, `theme.js`, `boot.js`

**Frontend Routes:** `/` (notes), `/canvas`, `/canvas/:id`, `/import`

## Key Concepts

### Wikilink Pattern

```markdown
[[Note Title]]              → Links to note with exact title
[[Note Title|Display]]      → Custom display text
```

**Graph Index:** Parses all notes on `build_index()` to construct adjacency lists. Backlinks are reverse edges: if A links to B, B has backlink from A.

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

**Semantic context mode:** When `context_mode == Semantic` (the default), `send_prompt` uses the retrieval pipeline to find relevant notes, fetches their content, and injects it as system prompt context. Pinned notes per session (`pinned_note_ids`) are always included. Context notes are stored on the tile and emitted via `ContextNotes` event for frontend display.

**Streaming architecture:** Commands return immediately, spawn async tasks, stream via `canvas-stream` Tauri events (`TileCreated`, `ContextNotes`, `Chunk`, `Complete`, `Error`, `SessionSaved`, debate variants). Frontend listens via `@tauri-apps/api/event`.

Streaming commands: `send_prompt`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response`

**Web search:** When `web_search: true`, OpenRouter's `plugins: [{"id": "web", "max_results": 5}]` is added to the API request (~$0.02/query per model). The `web_search` flag is threaded through the full stack and persisted on `PromptTile` for regenerate/add-model replay.

**Smart web search auto-detection:** Controlled by `UserSettings.smart_web_search` (default: `true`). When enabled, `useWebSearchDetection.js` analyzes prompt text with 5 heuristic rules (temporal markers, explicit search intent, news patterns, freshness queries, comparisons) and suppression rules (code blocks, wikilinks, short prompts). Detection result is shown as a hint in `PromptDialog.vue`. Disable via Settings toggle.

### Conversation Import

Import conversations from ChatGPT, Claude, Grok, or Gemini as evidence notes. Four format parsers with auto-detection via platform-specific JSON keys. Each parser implements `can_parse()` + `parse()`. Imported conversations become evidence-status container notes with provenance metadata (`source`, `source_id`, `created_via`).

### Temporal + Graph-Aware Retrieval

Pipeline: Tantivy keyword search → timestamp enrichment from GraphIndex → priority scoring (recency/status/tags) → N-hop graph expansion (bidirectional wikilinks) → hub boost (highly-connected notes) → top-K results with relevance reasons.

Configurable via `RetrievalConfig` (persisted in `data/retrieval_config.json`): `graph_hop_depth`, `graph_proximity_weight`, `hub_boost_weight`, `hub_threshold`, `base_search_limit`.

### Feedback & Bug Reporting

Submit bug reports, feature requests, and general feedback. Creates GitHub Issues automatically. Desktop app has offline queue with automatic retry.

### Settings System

First-run setup wizard and persistent settings. Manages vault path, OpenRouter API key, MCP configuration, theme preferences, and LLM model selection. Settings stored as JSON in app data directory. Frontend: `SettingsModal.vue`.

- **`llm_model`** — configurable LLM model for distillation and link discovery (default: `anthropic/claude-3.5-haiku`), selectable via Settings dropdown when API key is configured
- **`smart_web_search`** — enables automatic web search detection in canvas prompts (default: `true`). Uses `#[serde(default = "default_smart_web_search")]` for backward-compatible `true` default.

**Runtime sync pattern:** When settings change via `update_settings`, dependent services are updated in-place — no restart required. The pattern (in `commands/settings.rs`): capture changed fields before moving the update, apply settings, then sync each affected service:
- **OpenRouter API key** → `openrouter.set_api_key()`
- **Vault path** → `knowledge_store.set_vault_path()` + rebuild search index + rebuild graph index

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

**Architecture:** The `grafyn-mcp` binary is a second `[[bin]]` target in the same `Cargo.toml`, compiled with `--no-default-features --features mcp` (no Tauri). It shares `services/` and `models/` modules with the Tauri app.

**Concurrent access:** The MCP binary tries to acquire the Tantivy writer lock. If the Tauri app holds it, it falls back to read-only search (queries work, index updates are skipped). File I/O to the vault is always safe.

**Building locally:**
```bash
cd frontend/src-tauri
cargo build --release --bin grafyn-mcp --no-default-features --features mcp
```

**10 MCP tools:** `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note`, `search_notes`, `get_backlinks`, `get_outgoing`, `recall_relevant`, `import_conversation`

**Connecting Claude Desktop:** Add to `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "grafyn": {
      "command": "path/to/grafyn-mcp",
      "args": ["--vault", "path/to/vault", "--data", "path/to/data"]
    }
  }
}
```
The Grafyn Settings UI shows this config snippet with the correct paths pre-filled.

## CI/CD

### Test Pipeline

`.github/workflows/test.yml` — runs on push to main and PRs:

| Job | Purpose |
|-----|---------|
| `rust-tests` | `cargo test` with Swatinem/rust-cache |
| `frontend-tests` | `npm run test:run` (Vitest) |
| `lint` | `npm run lint` + `cargo clippy` |
| `security` | `npm audit` |
| `build` | `npm run build` (Vite production build) |
| `test-summary` | Aggregates job results into GitHub Actions summary |

### Release Pipeline

`.github/workflows/release.yml` — triggered by `v*` tags and `workflow_dispatch`.

```
create-release → build (4-job matrix) → publish-release → upload-to-r2 → build-summary
```

| Job | Purpose |
|-----|---------|
| `create-release` | Creates a single **draft** GitHub release (avoids matrix race condition) |
| `build` | 4-platform matrix: builds MCP binary first, then `tauri-action` (with `releaseId`) bundles it |
| `publish-release` | Marks draft → published after all builds upload artifacts |
| `upload-to-r2` | Downloads release assets, rewrites `latest.json` URLs, uploads to Cloudflare R2 |
| `build-summary` | Writes build status table to GitHub Actions summary |

**Required secrets:** `TAURI_PRIVATE_KEY`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `FEEDBACK_REPO`, `FEEDBACK_TOKEN`

**Required vars:** `CLOUDFLARE_WORKER_URL` (optional, has default)

## CI Pitfalls (Known Issues & Fixes)

### Tauri v1 Requires Ubuntu 22.04

Tauri v1 depends on `libwebkit2gtk-4.0-dev` which **does not exist on Ubuntu 24.04** (`ubuntu-latest`). The `rust-tests` and `lint` CI jobs must use `runs-on: ubuntu-22.04`. Do NOT use `ubuntu-latest` for any job that compiles Tauri Rust code.

- `libwebkit2gtk-4.1-dev` (Ubuntu 24.04) does NOT satisfy Tauri v1's `webkit2gtk-sys` crate — it provides `webkit2gtk-4.1.pc` but Tauri v1 needs `webkit2gtk-4.0.pc`.
- The `linux-ipc-protocol` Tauri feature is **Tauri v2 only** — do not attempt to use it with Tauri v1.8.

### Rust CI Requires MCP Binary + Stub dist/

`cargo test` compiles the full crate including `tauri::generate_context!()`. Two prerequisites must exist before running tests:

1. **MCP binary** — Tauri's `externalBin` config expects `binaries/grafyn-mcp-{target-triple}` at compile time.
   ```bash
   cargo build --bin grafyn-mcp --no-default-features --features mcp
   cp target/debug/grafyn-mcp "binaries/grafyn-mcp-$(rustc -vV | grep host | awk '{print $2}')"
   ```
2. **Stub dist directory** — `tauri::generate_context!()` panics if `distDir` (configured as `../dist`) doesn't exist.
   ```bash
   mkdir -p ../dist && echo '<html></html>' > ../dist/index.html
   ```

### Cargo.lock Must Be Committed

`Cargo.lock` is committed (not gitignored) to ensure reproducible CI builds. Without it, CI resolves fresh dependency versions that may break — e.g., `webkit2gtk` updates that are incompatible with `wry` 0.24.x.

### Tauri Features Must Include `process-all` and `protocol-all`

Removing `process-all` or `protocol-all` from the Tauri features in `Cargo.toml` changes the `wry`/`webkit2gtk` feature graph and breaks the Linux build. The `wry` crate's webkitgtk code depends on `SettingsExt` trait methods that are only in scope when these features are enabled.

### ESLint `_` Prefix Convention

The project's `.eslintrc.cjs` uses `argsIgnorePattern: '^_'` / `varsIgnorePattern: '^_'` / `destructuredArrayIgnorePattern: '^_'` for the `no-unused-vars` rule. Prefix intentionally unused variables with `_` to suppress lint errors.

## Deployment

**Build output:** `frontend/src-tauri/target/release/bundle/` (NSIS `.exe`, DMG, DEB, or AppImage)

**Data location:** `~/Documents/Grafyn/` (`vault/` for notes, `data/` for indexes)
