# Python / Rust Backend Duplication Report

## Executive Summary

Grafyn runs two backends: **Python/FastAPI** for web mode, **Rust/Tauri** for desktop mode. The frontend API client (`frontend/src/api/client.js`) uses `invokeOrHttp()` to route calls: Tauri IPC when `window.__TAURI__` is detected, HTTP to Python otherwise. This means **both backends MUST implement the same core features** -- they serve different deployment targets and never call each other at runtime (except the MCP sidecar, which is Python-only).

The desktop app **never** calls the Python backend for core features. The web app **never** calls the Rust backend. They are parallel implementations of the same product.

---

## 1. KnowledgeStore (Note CRUD + File I/O)

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/knowledge_store.py` (L1-384) | `frontend/src-tauri/src/services/knowledge_store.rs` (L1-255) |
| **Frontmatter lib** | `python-frontmatter` | `gray_matter` crate + `serde_yaml` |
| **Wikilink regex** | `\[\[([^\]|]+)(?:\|[^\]]+)?\]\]` (L19) | `\[\[([^\]|]+)(?:\|[^\]]+)?\]\]` (L12) |
| **Note ID generation** | `re.sub(r'[^\w\s-]', '', title).strip().replace(' ', '_')` (L60-61) | Lowercase slug with dedup counter (L127-153) |
| **Path traversal protection** | Sanitizes + resolves + checks `relative_to()` (L42-56) | **None** -- just `vault_path.join(format!("{}.md", id))` (L156-158) |
| **Rename link updates** | `update_links_on_rename()` -- scans all .md files and rewrites wikilinks (L146-186) | **Missing** -- no equivalent |
| **Wikilinks with anchors** | `extract_wikilinks_with_anchors()`, heading/block extraction (L106-144) | **Missing** -- only basic wikilink extraction |
| **Note type inference** | `_infer_note_type()` from title prefix or frontmatter (L68-92) | **Missing** -- no note_type field on Rust Note struct |
| **Content type field** | `ContentType` enum (claim/decision/insight/etc.) in frontmatter (L17-24) | **Missing** -- Rust Note has no content_type |
| **Aliases** | Read from frontmatter, used in graph resolution (L258, graph L45) | **Missing** |
| **List notes** | Returns `NoteListItem` with link_count, note_type, outgoing_links, source, container_of | Returns `NoteMeta` with only id, title, status, tags, timestamps |
| **Directory walk** | `glob("*.md")` -- flat only | `WalkDir` with `max_depth(2)` -- supports subdirs |

### Behavior Divergences

1. **ID generation is completely different**: Python uses `re.sub` + underscores (`My_Note`), Rust uses lowercase slug + hyphens with dedup (`my-note`, `my-note-1`). A note created in web mode may not be found by ID in desktop mode.

2. **Security**: Python has path traversal protection (L42-56); Rust has none (L156-158).

3. **Frontmatter field names differ**: Python uses `created`/`modified`; Rust uses `created_at`/`updated_at`. Notes written by one backend may not read timestamps correctly in the other.

4. **Python is significantly more feature-rich**: Wikilink anchors, note type inference, content types, aliases, link-on-rename, heading/block extraction are all Python-only.

### Verdict
**Both MUST exist** (different runtimes). Python is the more complete implementation. Rust is missing several features and has compatibility issues with Python-generated files.

---

## 2. Search Services

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/vector_search.py` (L1-494) | `frontend/src-tauri/src/services/search.rs` (L1-259) |
| **Technology** | LanceDB + sentence-transformers (semantic/vector) | Tantivy (full-text/keyword) |
| **Search type** | Cosine similarity on 384-dim embeddings | BM25-style full-text with QueryParser |
| **Embedding model** | `all-MiniLM-L6-v2` (384 dimensions) | N/A -- no embeddings |
| **Index storage** | LanceDB files in `data/lancedb/` | Tantivy index in `data/search_index/` |
| **Similar notes** | Vector cosine similarity (true semantic) | Keyword overlap (first 20 long words from content) |
| **Power search** | Operator parsing: `tag:`, `status:`, `type:`, `has:`, `path:`, negation (L196-279) | **None** -- plain text query only |
| **Canvas tile search** | Indexes tiles + notes in same db, `search_all()` (L401-493) | **None** -- notes only |
| **Score normalization** | `1/(1+distance)` mapped to 0-1 | Tantivy raw BM25 scores (unbounded) |

### Behavior Divergences

These are **fundamentally different search approaches**, not duplicated logic:
- Python: semantic/vector search (understands meaning)
- Rust: keyword/full-text search (matches exact terms)

The frontend calls the same `search.query()` API but gets qualitatively different results depending on backend. The `semantic` parameter in the web API is ignored by Rust.

### Verdict
**Genuinely different implementations** by design. The Rust version is simpler and faster but less capable. Not candidates for deduplication -- they serve different use cases. However, the frontend doesn't communicate to users which search type they're getting.

---

## 3. Graph Index

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/graph_index.py` (L1-319) | `frontend/src-tauri/src/services/graph_index.rs` (L1-236) |
| **Data structure** | `Dict[str, Set[str]]` for outgoing/incoming + title-to-id maps | `HashMap<String, HashSet<String>>` -- identical approach |
| **Build from notes** | Two-pass: build title maps, then resolve wikilinks to IDs (L26-70) | Same two-pass approach (L35-61) |
| **Title resolution** | Case-sensitive: `_title_to_id[title]` (L57) | Case-insensitive: `title_to_id[title.to_lowercase()]` (L29, L51) |
| **Alias support** | Yes -- aliases added to title_to_id map (L44-45) | **No** |
| **Backlinks with context** | `get_backlinks_with_context()` -- extracts surrounding text (L92-111) | **Missing** -- returns `NoteMeta` only |
| **Unlinked mentions** | `find_unlinked_mentions()` -- title text search (L154-198) | **Missing** |
| **Full graph export** | `get_full_graph()` with hub-based coloring (L250-319) | **Missing** |
| **Incremental update** | `update_note()` -- removes old links, adds new (L200-226) | `update_note()` -- removes then re-adds (L73-95) |
| **Orphan detection** | `get_unlinked_notes()` with full metadata (L228-248) | `get_unlinked()` -- returns `NoteMeta` (L178-196) |
| **Neighbors** | BFS with configurable depth (L127-152), returns `Dict[str, List[str]]` | Single-depth, returns `Vec<GraphNeighbor>` with link type (L147-175) |
| **Graph stats** | None | `stats()` method with total notes, links, backlinks, orphans (L209-225) |

### Behavior Divergences

1. **Case sensitivity**: Python resolves wikilinks case-sensitively; Rust lowercases all titles. `[[My Note]]` and `[[my note]]` behave differently across backends.

2. **Return types differ**: Python backlinks return `List[str]` (note IDs); Rust returns `Vec<NoteMeta>` (full metadata). The frontend graph endpoints (`get_backlinks`, `get_outgoing`) have different response shapes.

3. **Python has richer graph features**: unlinked mentions, context extraction, hub-based coloring, multi-depth BFS, full graph export -- all missing from Rust.

### Verdict
**Both MUST exist.** Core logic (adjacency lists) is equivalent. Rust version is a minimal subset. Python is canonical for graph features.

---

## 4. Canvas Store (Session Persistence)

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/canvas_store.py` (L1-955) | `frontend/src-tauri/src/services/canvas_store.rs` (L1-335) |
| **Storage** | JSON files, in-memory cache `Dict[str, CanvasSession]` | JSON files, read-from-disk on every operation |
| **CRUD** | list/get/create/update/delete -- identical API surface | Same operations |
| **Tile management** | `add_prompt_tile()` with color palette, position calc, branch logic (L139-231) | `add_tile()` takes pre-built tile -- no color/position logic (L109-115) |
| **Streaming support** | `update_response_content()` -- updates in-memory, defers save (L396-418) | `update_tile_response()` -- reads/writes file each time (L286-305) |
| **Conversation context** | `build_full_history()`, `build_compact_history()`, `build_semantic_context()` (L605-767) | **Missing** -- context building is in canvas commands |
| **Node graph methods** | `find_node_groups()`, `get_node_edges()` (L773-954) | **Missing** |
| **Debate management** | `add_debate()`, `add_debate_round()`, `update_debate_status()` | `add_debate()`, `update_debate()` |
| **Batch positions** | `batch_update_positions()` with node ID parsing (L846-906) | `batch_update_positions()` -- same logic (L222-261) |
| **Color assignment** | MODEL_COLORS palette, per-model color assignment (L157-203) | **Missing** -- tile creation logic is in commands layer |

### Behavior Divergences

1. **Performance model differs**: Python caches all sessions in memory and saves on mutation; Rust reads from disk on every operation. The Rust approach is simpler but slower for frequent updates (like streaming).

2. **Tile creation responsibility**: In Python, `CanvasSessionStore.add_prompt_tile()` handles all positioning, coloring, and branch logic. In Rust, this logic lives in `commands/canvas.rs:send_prompt()` (L70-260). Same logic, different location.

3. **Python has conversation context features** (`build_full_history`, `build_compact_history`, `build_semantic_context`) that don't exist in Rust at all -- the Rust canvas commands don't use conversation history.

### Verdict
**Both MUST exist.** Python is substantially more feature-rich. The Rust canvas store is a minimal persistence layer with business logic scattered into the commands layer.

---

## 5. OpenRouter / LLM Integration

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/openrouter.py` (L1-194) | `frontend/src-tauri/src/services/openrouter.rs` (L1-365) |
| **HTTP client** | `httpx.AsyncClient` with connection pooling | `reqwest::Client` |
| **Streaming** | `AsyncGenerator[str, None]` via `aiter_lines()` | `futures::Stream<Item = Result<String>>` via `bytes_stream()` |
| **Non-streaming** | `complete()` collects from stream (L169-182) | `chat()` -- separate non-streaming endpoint (L88-145) |
| **API headers** | `HTTP-Referer: {app_url}`, `X-Title: Grafyn Knowledge Platform` (L31-33) | `HTTP-Referer: https://grafyn.app`, `X-Title: Grafyn` (L123-124) |
| **Model caching** | 5-minute TTL cache (L53-59) | **None** -- fetches fresh every time |
| **Default models** | None -- returns empty list if unconfigured | Hardcoded fallback list of 6 models (L248-299) |
| **Key validation** | `validate_api_key()` method (L184-193) | **Missing** (validation is in settings command) |
| **Key management** | Reads from env/settings once at init | `set_api_key()` for dynamic updates (L25-26) |
| **SSE parsing** | Line-by-line `data: ` prefix stripping (L131-153) | `parse_sse_chunk()` function (L209-229) |
| **Error handling** | Specific `HTTPStatusError`, `TimeoutException` (L155-167) | Generic `anyhow::Error` (L130-133) |

### Behavior Divergences

1. **SSE parsing**: Both parse the same OpenRouter SSE format but with different chunking strategies. Python processes line-by-line; Rust processes byte chunks that may contain multiple SSE lines. Edge cases around chunk boundaries could differ.

2. **System prompt injection**: Rust's `chat()` and `chat_stream()` accept a separate `system_prompt` parameter and prepend it to messages. Python's `stream_completion()` expects system prompts to already be in the messages list. The calling code must handle this differently.

3. **Referer headers differ**: Python uses configurable `settings.app_url`; Rust hardcodes `https://grafyn.app`. Minor but inconsistent.

### Verdict
**Both MUST exist.** Core logic is equivalent. Rust version has cleaner separation of concerns (separate system_prompt param, default models fallback). Python has better caching.

---

## 6. Models / Structs

### Note Models

| Field | Python (`backend/app/models/note.py`) | Rust (`frontend/src-tauri/src/models/note.rs`) |
|-------|----------------------------------------|------------------------------------------------|
| `id` | str | String |
| `title` | str | String |
| `content` | str | String |
| `status` | str (validated: draft/evidence/canonical) | `NoteStatus` enum (Draft/Evidence/Canonical) |
| `tags` | List[str] | Vec<String> |
| `created` / `created_at` | `created` (Optional[datetime]) | `created_at` (DateTime<Utc>) |
| `modified` / `updated_at` | `modified` (Optional[datetime]) | `updated_at` (DateTime<Utc>) |
| `frontmatter` | `NoteFrontmatter` (rich: aliases, source, content_type, note_type, properties) | `NoteFrontmatter` (minimal: title, status, tags, timestamps, extra HashMap) |
| `outgoing_links` | List[str] (via frontmatter) | `wikilinks` (Vec<String>) |
| `backlinks` | List[str] | **Missing** |
| `content_type` | `ContentType` enum (6 variants) | **Missing** |
| `note_type` | `NoteType` enum (4 variants) | **Missing** |
| `properties` | `Dict[str, TypedProperty]` (typed with validation) | `HashMap<String, serde_json::Value>` (untyped) |
| `NoteListItem` fields | link_count, note_type, outgoing_links, source, container_of | **Not present** -- `NoteMeta` is simpler |
| `SearchResult` | note_id, title, snippet, score (0-1), tags | `NoteMeta` + score (f32, unbounded) + snippet |
| `BacklinkInfo` | note_id, title, context | **Missing** |

### Canvas Models

| Aspect | Python (`backend/app/models/canvas.py`) | Rust (`frontend/src-tauri/src/models/canvas.rs`) |
|--------|------------------------------------------|--------------------------------------------------|
| `CanvasSession` | Identical core fields | Same + extra `context_mode` field on PromptTile |
| `ModelResponse.color` | `color: str = "#7c5cff"` | **Missing** -- no color field |
| `ModelResponse.completed_at` | Yes | **Missing** |
| `ModelResponse.error_message` | `error_message` | `error` (different name) |
| `DebateRound` (Python) | `Dict[str, str]` for rounds | Structured: `DebateRound { round_number, topic, responses: Vec<DebateResponse> }` |
| `CanvasStreamEvent` | N/A (uses HTTP SSE) | Tauri-specific enum with 11 variants |
| `NodeEdge`, `TileEdge` | Present | **Missing** |
| Default tile dimensions | width=280, height=200 | width=400, height=300 (different!) |

### Key Divergences

1. **Timestamp field names**: `created`/`modified` (Python) vs `created_at`/`updated_at` (Rust). This means **notes saved by one backend may not read correctly in the other**.

2. **Properties typing**: Python has `TypedProperty` with validation (string/number/date/boolean/link). Rust uses `serde_json::Value` -- no type enforcement.

3. **Debate round structure completely different**: Python stores `List[Dict[str, str]]` (flat); Rust stores `Vec<DebateRound>` (structured with round_number, topic, responses). **Debate sessions are not portable between backends.**

4. **Default tile sizes differ**: 280x200 (Python) vs 400x300 (Rust). Canvas sessions created on one platform will render differently on the other.

---

## 7. Memory Service

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/memory.py` (L1-227) | `frontend/src-tauri/src/services/memory.rs` (L1-233) |
| **Recall** | Semantic search + graph boost (1.25x multiplier) (L27-92) | Full-text search + graph boost (0.15 additive) (L17-69) |
| **Contradictions** | Semantic similarity > 0.8 threshold, checks status + tags (L94-153) | Full-text similarity > 0.7/0.8, checks status + tags (L72-133) |
| **Extract claims** | From assistant messages only, extracts title + tags (L155-226) | From all messages, classifies claim type (decision/question/insight/claim) (L136-181) |
| **Tag extraction** | Uses `normalize_tag()` from distillation module | Standalone regex `(?:^|\s)#([\w-]+)` |
| **Claim output** | Dict with title, content, tags, status, source | `ExtractedClaim` struct with title, content, tags, claim_type, confidence |

### Behavior Divergences

1. **Completely different scoring**: Python boosts graph neighbors by 1.25x (multiplicative); Rust adds 0.15 (additive). Same query will rank results differently.

2. **Extract claims logic differs fundamentally**: Python only processes assistant messages and creates note suggestions. Rust processes all messages and classifies by claim type with confidence scores. Different output schema.

3. **Underlying search differs**: Python uses semantic search (vectors); Rust uses keyword search (Tantivy). Memory recall quality will be substantially different.

### Verdict
**Both MUST exist.** Behavior is meaningfully different -- users get different quality results depending on platform. Neither is strictly "more correct" -- Python's semantic approach is better for recall quality, Rust's is faster.

---

## 8. Feedback Service

| Aspect | Python | Rust |
|--------|--------|------|
| **File** | `backend/app/services/feedback.py` (L1-185) | `frontend/src-tauri/src/services/feedback.rs` (L1-339) |
| **Offline queue** | **None** -- returns error on failure | Full offline queue with JSON file persistence, retry (L228-313) |
| **System info** | Hardcoded "Web Browser" / "python-fastapi" | Real platform detection: `std::env::consts::OS` + `CARGO_PKG_VERSION` |
| **Issue body footer** | "Submitted via Grafyn Web App" | "Submitted via Grafyn Desktop App" |
| **Connectivity check** | **None** | `is_online()` -- checks GitHub API rate_limit endpoint |
| **Pending management** | **None** | get_pending, retry_pending, clear_pending |

### Verdict
**Both MUST exist.** Rust version is more feature-rich (offline queue, real system info). Both submit to GitHub Issues with identical format. The offline queue is a desktop-appropriate enhancement.

---

## 9. Python-Only Services (No Rust Equivalent)

These services exist **only in the Python backend** and are not available in desktop mode:

| Service | File | Purpose | Desktop Impact |
|---------|------|---------|----------------|
| `EmbeddingService` | `services/embedding.py` | sentence-transformers wrapper | Desktop has no semantic search |
| `DistillationService` | `services/distillation.py` | Container -> Atomic -> Hub workflow | Cannot distill notes in desktop |
| `ImportService` | `services/import_service.py` | LLM conversation import | Cannot import conversations in desktop |
| `LinkDiscoveryService` | `services/link_discovery.py` | Semantic + LLM link discovery | No Zettelkasten link discovery in desktop |
| `PriorityScoringService` | `services/priority_scoring.py` | Configurable search ranking | Desktop search has no priority scoring |
| `PrioritySettingsService` | `services/priority_settings.py` | Weight persistence | Not available in desktop |
| `TokenStore` | `services/token_store.py` | OAuth token management | N/A -- desktop doesn't need OAuth |
| **Parsers** | `services/parsers/` (5 files) | ChatGPT/Claude/Grok/Gemini import | Not available in desktop |

The frontend API client (`client.js`) confirms this: `distill`, `normalizeTags`, `zettelkasten.*` all make HTTP calls directly with no Tauri fallback (L87-88, L364-383).

---

## 10. Rust-Only Services (No Python Equivalent)

| Service | File | Purpose |
|---------|------|---------|
| `SettingsService` | `services/settings.rs` | First-run wizard, vault path, API key management |
| `McpSidecarService` | `services/mcp_sidecar.rs` | Spawns/manages Python sidecar process |

These are desktop-specific and appropriately Rust-only.

---

## Summary: What MUST Be Duplicated vs. What Doesn't Need To Be

### MUST exist in both (core features used by frontend `invokeOrHttp()`):

| Feature | Python Status | Rust Status | Notes |
|---------|--------------|-------------|-------|
| Note CRUD | Complete | Minimal | Rust missing: path traversal protection, note types, content types, aliases, rename-link-update |
| Search | Semantic (vector) | Full-text (keyword) | Fundamentally different; both valid |
| Graph index | Full-featured | Minimal | Rust missing: unlinked mentions, context, full graph, multi-depth BFS |
| Canvas CRUD | Complete | Complete | Feature-equivalent for basic operations |
| Canvas streaming | HTTP SSE | Tauri events | Different transport, same concept |
| OpenRouter | Complete | Complete | Largely equivalent |
| Feedback | Basic | Full (offline queue) | Rust is more capable |
| Memory | Semantic-based | Keyword-based | Different quality, both work |

### Only needs to exist in ONE backend:

| Feature | Currently In | Should Stay In | Reason |
|---------|-------------|----------------|--------|
| Distillation | Python only | Python (or move to Rust) | Desktop users can't distill notes |
| Conversation import | Python only | Python (or move to Rust) | Desktop users can't import |
| Link discovery | Python only | Python (or move to Rust) | Desktop users can't discover links |
| Priority scoring | Python only | Python only | Could be added to Rust but low priority |
| OAuth/Token management | Python only | Python only | Desktop doesn't need OAuth |
| Settings/first-run | Rust only | Rust only | Web doesn't need settings wizard |
| MCP sidecar management | Rust only | Rust only | Desktop-specific process management |

---

## Recommendations

### Critical Compatibility Issues (Fix First)

1. **Timestamp field names**: Unify `created`/`modified` (Python) with `created_at`/`updated_at` (Rust). Currently, notes created in one backend have broken timestamps in the other. **Recommend: standardize on `created_at`/`updated_at` in both.**

2. **Note ID generation**: Python uses underscores (`My_Note`), Rust uses lowercase hyphens (`my-note`). A note created on web cannot be found by ID on desktop. **Recommend: standardize on one algorithm.**

3. **Default tile dimensions**: Python 280x200 vs Rust 400x300. Canvas sessions look different across platforms. **Recommend: unify defaults.**

4. **Debate round schema**: Python stores `List[Dict[str, str]]`; Rust stores structured `Vec<DebateRound>`. Debate sessions are **not portable**. **Recommend: unify to the Rust structured format (it's richer).**

### Security

5. **Rust KnowledgeStore needs path traversal protection**: Python sanitizes note IDs (L42-56); Rust does not (L156-158). This is a security gap.

### Feature Parity

6. **Rust graph index should support case-insensitive wikilink resolution** (it already does) **and Python should too** -- currently Python is case-sensitive which is surprising behavior.

7. **Consider which Python-only features should be ported to Rust** for desktop feature parity. Distillation, import, and link discovery are significant missing features.

### Canonical Version

For shared logic, **Python is canonical** for most services (more complete, more tested). Exceptions:
- **Feedback**: Rust is canonical (offline queue, real system info)
- **Settings**: Rust is canonical (only lives there)
- **Canvas streaming architecture**: Rust is canonical (Tauri event system is more sophisticated)
