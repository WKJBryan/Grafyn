# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

| Component | Stack | Entry Point | Port |
|-----------|-------|-------------|------|
| **Backend (Python)** | FastAPI, LanceDB, sentence-transformers, OpenRouter | `backend/app/main.py` | 8080 |
| **Backend (Rust)** | Tauri, Tantivy, petgraph, reqwest | `frontend/src-tauri/src/main.rs` | N/A |
| **MCP Server** | rmcp, Tantivy, stdio transport | `frontend/src-tauri/src/mcp.rs` | stdio |
| **Frontend** | Vue 3, Vite, Pinia, D3.js | `frontend/src/main.js` | 5173 |

## Deployment Modes

| Mode | Backend | Bundle Size | Use Case |
|------|---------|-------------|----------|
| **Web** | Python/FastAPI | N/A | Development, MCP integration |
| **Desktop** | Rust/Tauri | ~15-30MB | Production desktop app |
| **Desktop + MCP** | Rust/Tauri + native MCP binary | ~25-40MB | Desktop app with Claude Desktop integration |

## Development Commands

### Backend

```bash
cd backend
pip install -r requirements.txt
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080

# Docker alternative
docker-compose up
```

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
cd backend
pip install -r requirements-dev.txt
pytest                                # All tests
pytest --cov=app --cov-report=html    # With coverage
pytest -m unit                        # Unit tests only
pytest -m integration                 # Integration tests
pytest -m security                    # Security tests
```

See `backend/tests/README.md` for full testing documentation.

## Architecture Overview

### Desktop App (Tauri)

The desktop app uses a **pure Rust backend** with Vue frontend in a single binary:

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
│  │   settings, feedback, mcp, memory)        │  │
│  └──────────────────────────────────────────┘  │
│  ~/Documents/Grafyn/                          │
│  ├── vault/  (markdown notes)                   │
│  └── data/   (search index, canvas, settings)   │
└────────────────────────────────────────────────┘
```

### Web Backend Service Layer

The backend uses a **singleton service pattern** initialized at startup via `lifespan`. Services are attached to `app.state` and accessed in routers:

```python
# Recommended: use dependency helpers from app/utils/dependencies.py
from app.utils import get_knowledge_store, get_vector_search, get_graph_index

@router.get("/example")
async def example(request: Request):
    knowledge_store = get_knowledge_store(request)
    return knowledge_store.list_notes()
```

**Available helpers:** `get_knowledge_store`, `get_vector_search`, `get_graph_index`, `get_openrouter`, `get_canvas_store`, `get_priority_scoring`, `get_priority_settings`, `get_distillation`, `get_link_discovery`, `get_import_service`, `get_feedback_service`, `get_memory_service`

### Core Services

| Service | File | Purpose |
|---------|------|---------|
| `KnowledgeStore` | `services/knowledge_store.py` | Markdown file I/O, YAML frontmatter, wikilink extraction |
| `VectorSearchService` | `services/vector_search.py` | LanceDB indexing, semantic search (384-dim vectors) |
| `GraphIndexService` | `services/graph_index.py` | In-memory adjacency lists for backlinks/outgoing links |
| `EmbeddingService` | `services/embedding.py` | sentence-transformers wrapper (all-MiniLM-L6-v2) |
| `TokenStore` | `services/token_store.py` | OAuth token management with TTL |
| `OpenRouterService` | `services/openrouter.py` | OpenRouter API client with streaming support |
| `CanvasSessionStore` | `services/canvas_store.py` | Canvas session persistence (JSON file storage) |
| `DistillationService` | `services/distillation.py` | Container → Atomic → Hub knowledge workflow |
| `ImportService` | `services/import_service.py` | LLM conversation import + quality assessment |
| `LinkDiscoveryService` | `services/link_discovery.py` | Semantic + LLM-based link discovery |
| `PriorityScoringService` | `services/priority_scoring.py` | Search result ranking with configurable weights |
| `PrioritySettingsService` | `services/priority_settings.py` | Priority weight persistence (JSON) |
| `FeedbackService` | `services/feedback.py` | GitHub Issues integration for bug reports/feature requests |
| `MemoryService` | `services/memory.py` | Semantic recall, contradiction detection, conversation extraction |

### Router Quick Reference

| Router | Prefix | Endpoints | Purpose |
|--------|--------|-----------|---------|
| `notes.py` | `/api/notes` | 12 | CRUD, list, reindex, properties |
| `search.py` | `/api/search` | 2 | Query, similar |
| `graph.py` | `/api/graph` | 7 | Backlinks, outgoing, neighbors, unlinked |
| `canvas.py` | `/api/canvas` | 22 | Sessions, prompts, debates, SSE streaming |
| `mcp_write.py` | `/api/mcp-write` | 9 | MCP write operations (create, update, find-or-create) |
| `distill.py` | `/api/distill` | 2 | Distill note, normalize tags |
| `priority.py` | `/api/priority` | 7 | Priority scoring configuration |
| `conversation_import.py` | `/api/import` | 7 | LLM conversation import workflow |
| `zettelkasten.py` | `/api/zettel` | 6 | Link discovery for Zettelkasten method |
| `feedback.py` | `/api/feedback` | 2 | Submit feedback, check status |
| `oauth.py` | `/auth` | 4 | GitHub OAuth flow |
| `memory.py` | `/api/memory` | 3 | Recall, contradictions, extract |

### Tauri IPC Commands (52 total across 9 modules)

| Module | Commands | Purpose |
|--------|----------|---------|
| `commands/notes.rs` | `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note` | Note CRUD |
| `commands/search.rs` | `search_notes`, `find_similar`, `reindex` | Full-text search |
| `commands/graph.rs` | `get_backlinks`, `get_outgoing`, `get_neighbors`, `get_unlinked`, `get_full_graph`, `rebuild_graph` | Link graph |
| `commands/canvas.rs` | `list_sessions`, `get_session`, `create_session`, `update_session`, `delete_session`, `get_available_models`, `send_prompt`, `update_tile_position`, `delete_tile`, `delete_response`, `update_viewport`, `update_llm_node_position`, `auto_arrange`, `export_to_note`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response` | Multi-LLM canvas (streaming via `canvas-stream` Tauri events) |
| `commands/distill.rs` | `distill_note`, `normalize_tags` | Rules-based + LLM distillation workflow |
| `commands/settings.rs` | `get_settings`, `get_settings_status`, `update_settings`, `complete_setup`, `pick_vault_folder`, `validate_openrouter_key`, `get_openrouter_status` | App settings & first-run setup |
| `commands/feedback.rs` | `submit_feedback`, `get_system_info`, `feedback_status`, `get_pending_feedback`, `retry_pending_feedback`, `clear_pending_feedback` | Feedback with offline queue |
| `commands/mcp.rs` | `get_mcp_status`, `get_mcp_config_snippet` | MCP config for Claude Desktop |
| `commands/memory.rs` | `recall_relevant`, `find_contradictions`, `extract_claims` | Memory recall & contradiction detection |

### Frontend API Client

```javascript
// src/api/client.js auto-detects Tauri vs web environment
import { notes, search, graph, canvas, settings, mcp, memory, zettelkasten, feedback, isDesktopApp } from '@/api/client'

// In Tauri: Uses invoke() for direct IPC to Rust backend
// In Web: Uses Axios HTTP calls to Python backend
// Canvas streaming: Tauri uses canvas-stream events, web uses HTTP SSE
```

**Pinia Stores:** `auth.js`, `notes.js`, `canvas.js`, `import.js`, `theme.js`

**Frontend Routes:** `/` (notes), `/canvas`, `/canvas/:id`, `/import`, `/import/review`, `/login`, `/oauth/callback`

## Key Concepts

### Wikilink Pattern

```markdown
[[Note Title]]              → Links to note with exact title
[[Note Title|Display]]      → Custom display text
```

**Parsing:** `KnowledgeStore.extract_wikilinks()` uses regex: `\[\[(.*?)(?:\|(.*?))?\]\]`

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

- Tag normalization: `#Tag` → `tag` (lowercase, strip #, spaces→hyphens)
- Inline `#tag` parsing (ignores headings and code blocks)
- Canvas exports use protected section markers to preserve user edits
- Dedup only matches against `draft`/`canonical` notes (not evidence/hubs)

### Zettelkasten Link Discovery

Discovers potential links using semantic similarity and LLM analysis. Three methods: **Semantic** (cosine similarity > threshold), **LLM** (OpenRouter analyzes content), **Hybrid** (semantic candidates + LLM ranking).

### Priority Scoring

Configurable search result ranking. Factors: `recency_weight`, `content_type_weights` (canonical/evidence/draft), `tag_boosts`.

### Vector Embeddings

- **Model:** `all-MiniLM-L6-v2` (384 dimensions)
- **Storage:** LanceDB with schema `{note_id, title, text, vector}`
- **Index trigger:** Automatic on note create/update via router
- **Search:** Cosine similarity via LanceDB's `.search(query_vector).limit(n)`

### MCP Integration

**Desktop (Rust):** Native `grafyn-mcp` binary using `rmcp` crate, stdio transport. See [MCP Server](#mcp-server-desktop--claude-desktop) section.

**Web (Python):** SSE-based MCP via `fastapi-mcp`:
- **Endpoint:** `/sse` (Server-Sent Events for MCP protocol)
- **Library:** `fastapi-mcp` wraps FastAPI routes as MCP tools
- **Setup:** `setup_mcp(app)` in `main.py` auto-exposes tagged endpoints
- **OAuth:** GitHub OAuth required for ChatGPT integration (tokens in `TokenStore`)
- **Write operations:** Notes created via MCP are tagged with `source: chatgpt-mcp`, `created_via: mcp`
- **Dev mode:** Set `ENVIRONMENT=development` to bypass OAuth

**MCP module files (Python):** `mcp/server.py`, `mcp/tools.py`, `mcp/oauth.py`, `mcp/write_tools.py`
**MCP files (Rust):** `src/mcp.rs` (binary entry point), `src/mcp_tools.rs` (9 tool definitions)

### Multi-LLM Canvas

Compare responses from multiple LLM models simultaneously via OpenRouter. Features: parallel model streaming, infinite canvas with D3.js zoom/pan, model debate mode, session persistence in `data/canvas/`.

**Streaming architecture (dual-path):**
- **Desktop (Tauri):** Commands return immediately, spawn async tasks, stream via `canvas-stream` Tauri events (`TileCreated`, `Chunk`, `Complete`, `Error`, `SessionSaved`, debate variants). Frontend listens via `@tauri-apps/api/event`.
- **Web:** Standard HTTP SSE (Server-Sent Events) via `fetch()` to Python backend.
- Streaming commands: `send_prompt`, `start_debate`, `continue_debate`, `add_models_to_tile`, `regenerate_response`

### Conversation Import

Import LLM conversations (ChatGPT, Claude, Grok, Gemini) into the knowledge base. Flow: Upload → Parse → Quality Assessment → Review → Import as Notes. Parsers in `services/parsers/` implement `BaseParser` with `can_parse()`, `parse()`, `to_markdown()`.

### Feedback & Bug Reporting

Submit bug reports, feature requests, and general feedback. Creates GitHub Issues automatically. Desktop app has offline queue with automatic retry.

### Settings System (Desktop)

First-run setup wizard and persistent settings. Manages vault path, OpenRouter API key, MCP configuration, and theme preferences. Settings stored as JSON in app data directory. Frontend: `SettingsModal.vue`.

**Runtime sync pattern:** When settings change via `update_settings`, dependent services are updated in-place — no restart required. The pattern (in `commands/settings.rs`): capture changed fields before moving the update, apply settings, then sync each affected service:
- **OpenRouter API key** → `openrouter.set_api_key()`
- **Vault path** → `knowledge_store.set_vault_path()` + rebuild search index + rebuild graph index

## Configuration

### Environment Setup

```bash
cp backend/.env.example .env
# Edit .env — must be in project root when running from root
```

### Critical Settings

| Variable | Default | Notes |
|----------|---------|-------|
| `ENVIRONMENT` | `development` | Affects CORS policy (strict in production) |
| `CORS_ORIGINS` | `*` (dev) | Comma-separated in production |
| `RATE_LIMIT_ENABLED` | `true` | Slowapi rate limiting (10/min, 50/hr, 200/day) |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Must match LanceDB vector dimension (384) |
| `OPENROUTER_API_KEY` | `""` | Required for Multi-LLM Canvas feature |
| `APP_URL` | `http://localhost:8080` | Used in OpenRouter API headers |
| `CANVAS_DATA_PATH` | `../data/canvas` | JSON storage for canvas sessions |
| `GITHUB_FEEDBACK_REPO` | `""` | Target repo for feedback issues (format: `owner/repo`) |
| `GITHUB_FEEDBACK_TOKEN` | `""` | GitHub PAT with `issues:write` scope |
| `TOKEN_ENCRYPTION_KEY` | — | Generate: `python -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())"` |

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

**9 MCP tools:** `list_notes`, `get_note`, `create_note`, `update_note`, `delete_note`, `search_notes`, `get_backlinks`, `get_outgoing`, `recall_relevant`

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

## CI/CD Release Pipeline

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

**Key detail:** The `build` matrix receives `releaseId` from `create-release` so `tauri-action` uploads to an existing release instead of each job racing to create one. For `workflow_dispatch` (no tag), `create-release` is skipped and builds produce artifacts only.

**Required secrets:** `TAURI_PRIVATE_KEY`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `FEEDBACK_REPO`, `FEEDBACK_TOKEN`

**Required vars:** `CLOUDFLARE_WORKER_URL` (optional, has default)

## Deployment Notes

### Web (Python Backend)

- **CORS:** Set `CORS_ORIGINS` to specific domains in production
- **OAuth:** Configure GitHub OAuth app with production redirect URI
- **Encryption:** Generate production `TOKEN_ENCRYPTION_KEY`
- **Docker:** Use `docker-compose.yml` in `backend/`

### Desktop (Tauri)

**Build output:** `frontend/src-tauri/target/release/bundle/` (NSIS `.exe`, DMG, DEB, or AppImage)

**Data location:** `~/Documents/Grafyn/` (`vault/` for notes, `data/` for indexes)

**Environment variables:** `OPENROUTER_API_KEY`, `GITHUB_FEEDBACK_REPO`, `GITHUB_FEEDBACK_TOKEN`, `RUST_LOG=info`
