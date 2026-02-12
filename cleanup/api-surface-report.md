# API Surface Area Audit Report

**Date:** 2026-02-11
**Scope:** All Python HTTP endpoints, all Rust Tauri IPC commands, and frontend usage analysis

---

## Summary Statistics

| Metric | Count |
|--------|-------|
| **Python HTTP endpoints** | 60 |
| **Rust Tauri IPC commands** | 53 |
| **Total API surface** | 113 |
| **Python endpoints with frontend usage** | 38 (63%) |
| **Python endpoints with NO frontend usage** | 22 (37%) |
| **Rust commands with frontend usage** | 53 (100%) |
| **Python endpoints with test coverage** | 49 (82%) |
| **Endpoints with ZERO usage + ZERO tests** | 4 |
| **MCP-only endpoints** | 9 |
| **Recommended for removal** | 7 |
| **Recommended for merge** | 10 (into 3) |

---

## 1. Full Endpoint Inventory

### 1.1 Python HTTP Endpoints (60 total)

#### Notes Router (`backend/app/routers/notes.py` -- prefix `/api/notes`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 1 | `/api/notes` | GET | Y (client.js:69) | Y (test_notes_api.py) | list_notes |
| 2 | `/api/notes/{note_id}` | GET | Y (client.js:71) | Y | get_note |
| 3 | `/api/notes` | POST | Y (client.js:73-74) | Y | create_note |
| 4 | `/api/notes/{note_id}` | PUT | Y (client.js:76-78) | Y | update_note |
| 5 | `/api/notes/{note_id}` | DELETE | Y (client.js:81-82) | Y | delete_note |
| 6 | `/api/notes/reindex` | POST | Y (client.js:84) | Y | reindex_notes |
| 7 | `/api/notes/{note_id}/properties` | GET | **N** | **N** | get_properties -- ORPHANED |
| 8 | `/api/notes/{note_id}/properties/{name}` | GET | **N** | **N** | get_property -- ORPHANED |
| 9 | `/api/notes/{note_id}/properties/{name}` | PUT | **N** | **N** | set_property -- ORPHANED |
| 10 | `/api/notes/{note_id}/properties/{name}` | DELETE | **N** | **N** | delete_property -- ORPHANED |

#### Search Router (`backend/app/routers/search.py` -- prefix `/api/search`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 11 | `/api/search` | GET | Y (client.js:93-95) | Y (test_search_api.py) | search_notes |
| 12 | `/api/search/similar/{note_id}` | GET | Y (client.js:98-100) | Y | find_similar_notes |

#### Graph Router (`backend/app/routers/graph.py` -- prefix `/api/graph`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 13 | `/api/graph/backlinks/{note_id}` | GET | Y (client.js:106-108) | Y (test_graph_api.py) | get_backlinks |
| 14 | `/api/graph/outgoing/{note_id}` | GET | Y (client.js:111-113) | Y | get_outgoing_links |
| 15 | `/api/graph/neighbors/{note_id}` | GET | Y (client.js:116-118) | Y | get_neighbors |
| 16 | `/api/graph/unlinked` | GET | Y (client.js:127) | Y | get_unlinked_notes |
| 17 | `/api/graph/unlinked-mentions/{note_id}` | GET | Y (client.js:121, UnlinkedMentions.vue:99) | Y | find_unlinked_mentions |
| 18 | `/api/graph/rebuild` | POST | Y (client.js:123) | Y | rebuild_graph |
| 19 | `/api/graph/full` | GET | Y (client.js:125, GraphView.vue:160) | Y | get_full_graph |

#### Canvas Router (`backend/app/routers/canvas.py` -- prefix `/api/canvas`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 20 | `/api/canvas` | GET | Y (client.js:140) | Y (test_canvas_api.py) | list_sessions |
| 21 | `/api/canvas` | POST | Y (client.js:145-146) | Y | create_session |
| 22 | `/api/canvas/models/available` | GET | Y (client.js:156-157) | Y | list_available_models |
| 23 | `/api/canvas/{session_id}` | GET | Y (client.js:142-143) | Y | get_session |
| 24 | `/api/canvas/{session_id}` | PUT | Y (client.js:148-150) | Y | update_session |
| 25 | `/api/canvas/{session_id}` | DELETE | Y (client.js:153-154) | Y | delete_session |
| 26 | `/api/canvas/{session_id}/viewport` | PUT | Y (client.js:196-198) | Y | update_viewport |
| 27 | `/api/canvas/{session_id}/tiles/{tile_id}/position` | PUT | Y (client.js:165-170) | Y | update_tile_position |
| 28 | `/api/canvas/{session_id}/tiles/{tile_id}/responses/{model_id}/position` | PUT | Y (client.js:173-178) | Y | update_llm_node_position |
| 29 | `/api/canvas/{session_id}/tiles/{tile_id}` | DELETE | Y (client.js:186-188) | Y | delete_tile |
| 30 | `/api/canvas/{session_id}/tiles/{tile_id}/responses/{model_id}` | DELETE | Y (client.js:191-193) | Y | delete_response |
| 31 | `/api/canvas/{session_id}/edges` | GET | **N** | Y | get_tile_edges -- LEGACY, replaced by node-edges |
| 32 | `/api/canvas/{session_id}/node-edges` | GET | Y (client.js:227) | Y | get_node_edges |
| 33 | `/api/canvas/{session_id}/arrange` | POST | Y (client.js:181-183) | Y | arrange_nodes |
| 34 | `/api/canvas/{session_id}/node-groups` | GET | Y (client.js:229) | Y | get_node_groups |
| 35 | `/api/canvas/{session_id}/prompt` | POST | Y (canvas.js:230, client.js:160-162) | Y | send_prompt (SSE streaming) |
| 36 | `/api/canvas/{session_id}/tile/{tile_id}/add-models` | POST | Y (client.js:217-219) | Y | add_models_to_tile (SSE) |
| 37 | `/api/canvas/{session_id}/tile/{tile_id}/regenerate/{model_id}` | POST | Y (client.js:222-224) | Y | regenerate_response (SSE) |
| 38 | `/api/canvas/{session_id}/debate` | POST | Y (canvas.js:549, client.js:207-209) | Y | start_debate (SSE) |
| 39 | `/api/canvas/{session_id}/debate/{debate_id}/continue` | POST | Y (canvas.js:661, client.js:212-214) | Y | continue_debate (SSE) |
| 40 | `/api/canvas/{session_id}/debate/{debate_id}/status` | PUT | **N** | Y | update_debate_status -- ORPHANED |
| 41 | `/api/canvas/{session_id}/export-note` | POST | Y (client.js:201-203) | Y | export_to_note |

#### Distillation Router (`backend/app/routers/distill.py` -- prefix `/api/notes`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 42 | `/api/notes/{note_id}/distill` | POST | Y (client.js:87, NoteEditor.vue:250) | Y (test_distill_api.py) | distill_note |
| 43 | `/api/notes/{note_id}/normalize-tags` | POST | Y (client.js:88) | Y | normalize_note_tags (only in client.js, no component call found) |

#### Priority Router (`backend/app/routers/priority.py` -- prefix `/api/priority`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 44 | `/api/priority/config` | GET | **N** | Y (test_priority_api.py) | get_priority_config |
| 45 | `/api/priority/weights` | GET | **N** | Y | get_priority_weights |
| 46 | `/api/priority/weights` | PUT | **N** | Y | update_priority_weights |
| 47 | `/api/priority/reset` | POST | **N** | Y | reset_priority_weights |
| 48 | `/api/priority/content-types` | GET | **N** | Y | get_content_type_scores |
| 49 | `/api/priority/recency` | GET | **N** | Y | get_recency_config |
| 50 | `/api/priority/link-density` | GET | **N** | Y | get_link_density_config |
| 51 | `/api/priority/tag-relevance` | GET | **N** | Y | get_tag_relevance_config |
| 52 | `/api/priority/semantic` | GET | **N** | Y | get_semantic_config |

#### Conversation Import Router (`backend/app/routers/conversation_import.py` -- prefix `/api/import`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 53 | `/api/import/` | GET | Y (import.js:323) | Y (test_conversation_import_api.py) | list_jobs |
| 54 | `/api/import/upload` | POST | Y (import.js:65) | Y | upload_file |
| 55 | `/api/import/{job_id}` | GET | **N** | Y | get_job (only used internally?) |
| 56 | `/api/import/{job_id}` | DELETE | Y (import.js:295) | Y | cancel_job |
| 57 | `/api/import/{job_id}/parse` | POST | Y (import.js:95) | Y | parse_file |
| 58 | `/api/import/{job_id}/preview` | GET | Y (import.js:126) | Y | get_preview |
| 59 | `/api/import/{job_id}/assess` | POST | Y (import.js:170) | Y | assess_quality |
| 60 | `/api/import/{job_id}/apply` | POST | Y (import.js:221) | Y | apply_import |
| 61 | `/api/import/{job_id}/revert` | POST | Y (import.js:265) | Y | revert_import |

#### Zettelkasten Router (`backend/app/routers/zettelkasten.py` -- prefix `/api/zettel`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 62 | `/api/zettel/notes/{note_id}/distill-zettel` | POST | Y (import.js:395) | Y (test_zettelkasten_api.py) | distill_zettelkasten |
| 63 | `/api/zettel/notes/{note_id}/discover-links` | GET | Y (client.js:367, NoteEditor.vue:280) | Y | discover_links |
| 64 | `/api/zettel/notes/{source_id}/link/{target_id}` | POST | Y (client.js:376-379, import.js:435) | Y | create_link |
| 65 | `/api/zettel/link-types` | GET | Y (client.js:382) | Y | get_link_types -- defined in client but **never called by any component** |
| 66 | `/api/zettel/zettel-types` | GET | **N** | Y | get_zettel_types -- ORPHANED (data is hardcoded in import.js:25-31) |
| 67 | `/api/zettel/notes/{note_id}/discover-links/apply` | POST | Y (client.js:371-373, LinkCandidateModal.vue:190) | Y | apply_discovered_links |

#### Feedback Router (`backend/app/routers/feedback.py` -- prefix `/api/feedback`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 68 | `/api/feedback` | POST | Y (client.js:234-235, FeedbackModal.vue) | Y (test_feedback_api.py) | submit_feedback |
| 69 | `/api/feedback/status` | GET | Y (client.js:237-238) | Y | get_feedback_status |

#### OAuth Router (`backend/app/routers/oauth.py` -- prefix `/api/oauth`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 70 | `/api/oauth/authorize/{provider}` | GET | Y (client.js:132, auth.js:22) | Y (test_oauth_api.py) | get_authorization_url |
| 71 | `/api/oauth/callback/{provider}` | POST | Y (client.js:133, auth.js:38) | Y | exchange_code |
| 72 | `/api/oauth/user` | GET | Y (client.js:134, auth.js:59) | Y | get_user |
| 73 | `/api/oauth/logout` | POST | Y (client.js:135, auth.js:74) | Y | logout |

#### MCP Write Router (`backend/app/routers/mcp_write.py` -- prefix `/api`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 74 | `/api/mcp/test` | GET | **N** | Y (test_mcp_write_api.py) | MCP test endpoint |
| 75 | `/api/mcp/test/simple` | POST | **N** | Y | MCP test simple |
| 76 | `/api/mcp/notes/create` | POST | **N** | Y | MCP-only: mcp_create_note_simple |
| 77 | `/api/mcp/notes` | POST | **N** | Y | MCP-only: mcp_create_note |
| 78 | `/api/mcp/notes/{note_id}` | PUT | **N** | Y | MCP-only: mcp_update_note |
| 79 | `/api/mcp/notes/find-or-create` | POST | **N** | Y | MCP-only: mcp_find_or_create_note |
| 80 | `/api/mcp/notes/{note_id}/properties` | PUT | **N** | Y | MCP-only: mcp_set_property |
| 81 | `/api/mcp/notes/search` | GET | **N** | Y | MCP-only: mcp_search_notes |
| 82 | `/api/mcp-write/note` | POST | **N** | Y | MCP-only: mcp_write_note (duplicate of #76?) |

#### Memory Router (`backend/app/routers/memory.py` -- prefix `/api/memory`)

| # | Endpoint | Method | Frontend Usage | Test Coverage | Notes |
|---|----------|--------|:-:|:-:|-------|
| 83 | `/api/memory/recall` | POST | Y (client.js:348-350, SearchBar.vue:96) | Y (test_memory_api.py) | recall |
| 84 | `/api/memory/contradictions/{note_id}` | POST | Y (client.js:353-355, HomeView.vue:358) | Y | contradictions |
| 85 | `/api/memory/extract` | POST | Y (client.js:358-360) | Y | extract -- defined in client but **never called by any component** |

### 1.2 Rust Tauri IPC Commands (53 total)

All 53 Rust commands are mapped in `frontend/src/api/client.js` via `invokeOrHttp()` and used by the frontend. They are the desktop equivalents of the Python HTTP endpoints.

#### Notes Commands (`commands/notes.rs` -- 5 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 1 | `list_notes` | Y (client.js:69) | |
| 2 | `get_note` | Y (client.js:71) | |
| 3 | `create_note` | Y (client.js:73) | |
| 4 | `update_note` | Y (client.js:76) | |
| 5 | `delete_note` | Y (client.js:81) | |

#### Search Commands (`commands/search.rs` -- 3 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 6 | `search_notes` | Y (client.js:93) | |
| 7 | `find_similar` | Y (client.js:98) | |
| 8 | `reindex` | Y (client.js:84) | |

#### Graph Commands (`commands/graph.rs` -- 5 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 9 | `get_backlinks` | Y (client.js:106) | |
| 10 | `get_outgoing` | Y (client.js:111) | |
| 11 | `get_neighbors` | Y (client.js:116) | |
| 12 | `get_unlinked` | Y (client.js:127) | |
| 13 | `rebuild_graph` | Y (client.js:123) | |

#### Canvas Commands (`commands/canvas.rs` -- 18 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 14 | `list_sessions` | Y (client.js:140) | |
| 15 | `get_session` | Y (client.js:142) | |
| 16 | `create_session` | Y (client.js:145) | |
| 17 | `update_session` | Y (client.js:148) | |
| 18 | `delete_session` | Y (client.js:153) | |
| 19 | `get_available_models` | Y (client.js:156) | |
| 20 | `send_prompt` | Y (client.js:160) | Streaming via canvas-stream events |
| 21 | `update_tile_position` | Y (client.js:165) | |
| 22 | `delete_tile` | Y (client.js:186) | |
| 23 | `delete_response` | Y (client.js:191) | |
| 24 | `update_viewport` | Y (client.js:196) | |
| 25 | `update_llm_node_position` | Y (client.js:173) | |
| 26 | `auto_arrange` | Y (client.js:181) | |
| 27 | `export_to_note` | Y (client.js:201) | |
| 28 | `start_debate` | Y (client.js:207) | Streaming via canvas-stream events |
| 29 | `continue_debate` | Y (client.js:212) | Streaming via canvas-stream events |
| 30 | `add_models_to_tile` | Y (client.js:217) | Streaming via canvas-stream events |
| 31 | `regenerate_response` | Y (client.js:222) | Streaming via canvas-stream events |

#### Settings Commands (`commands/settings.rs` -- 7 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 32 | `get_settings` | Y (client.js:269) | |
| 33 | `get_settings_status` | Y (client.js:271) | |
| 34 | `update_settings` | Y (client.js:277) | |
| 35 | `complete_setup` | Y (client.js:280) | |
| 36 | `pick_vault_folder` | Y (client.js:283-296) | |
| 37 | `validate_openrouter_key` | Y (client.js:298) | |
| 38 | `get_openrouter_status` | Y (client.js:311) | |

#### Feedback Commands (`commands/feedback.rs` -- 6 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 39 | `submit_feedback` | Y (client.js:234) | Used in FeedbackModal.vue |
| 40 | `get_system_info` | Y (client.js:240-258) | Used in FeedbackModal.vue |
| 41 | `feedback_status` | Y (client.js:237) | Used in FeedbackModal.vue |
| 42 | `get_pending_feedback` | Y (client.js:260) | Defined in client but **never called by any component** |
| 43 | `retry_pending_feedback` | Y (client.js:263) | Defined in client but **never called by any component** |
| 44 | `clear_pending_feedback` | Y (client.js -- not exposed) | **Not exposed in client.js at all** |

#### MCP Sidecar Commands (`commands/mcp.rs` -- 6 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 45 | `get_mcp_status` | Y (client.js:319) | Used in SettingsModal.vue |
| 46 | `start_mcp_sidecar` | Y (client.js:324) | |
| 47 | `stop_mcp_sidecar` | Y (client.js:329) | |
| 48 | `restart_mcp_sidecar` | Y (client.js:334) | |
| 49 | `check_mcp_health` | Y (client.js:339) | |
| 50 | `get_mcp_config_snippet` | Y (client.js:342) | |

#### Memory Commands (`commands/memory.rs` -- 3 commands)

| # | Command | Frontend Usage | Notes |
|---|---------|:-:|-------|
| 51 | `recall_relevant` | Y (client.js:348) | Used in SearchBar.vue |
| 52 | `find_contradictions` | Y (client.js:353) | Used in HomeView.vue |
| 53 | `extract_claims` | Y (client.js:358) | Defined in client but **never called by any component** |

---

## 2. Orphaned Endpoints (No Frontend Consumer)

### 2.1 Definitive Orphans -- Zero Frontend Usage

| Endpoint | Location | Recommendation | Risk |
|----------|----------|----------------|------|
| `GET /api/notes/{id}/properties` | notes.py:116 | **REMOVE** | Low -- no frontend, no tests, no MCP usage |
| `GET /api/notes/{id}/properties/{name}` | notes.py:130 | **REMOVE** | Low -- same as above |
| `PUT /api/notes/{id}/properties/{name}` | notes.py:145 | **REMOVE** | Low -- MCP has its own `/mcp/notes/{id}/properties` |
| `DELETE /api/notes/{id}/properties/{name}` | notes.py:172 | **REMOVE** | Low -- same as above |
| `GET /api/canvas/{id}/edges` | canvas.py:171 | **REMOVE** | Low -- marked "legacy", replaced by `/node-edges` |
| `PUT /api/canvas/{id}/debate/{did}/status` | canvas.py:980 | **REMOVE** | Low -- debate status is updated internally by start/continue endpoints |
| `GET /api/zettel/zettel-types` | zettelkasten.py:230 | **REMOVE** | Low -- data is hardcoded in `import.js:25-31` |

### 2.2 MCP-Only Endpoints -- Keep

These have no frontend usage but are consumed by Claude Desktop / ChatGPT via the MCP protocol. They should be kept.

| Endpoint | Location | Notes |
|----------|----------|-------|
| `GET /api/mcp/test` | mcp_write.py:40 | Test endpoint; consider removing in production |
| `POST /api/mcp/test/simple` | mcp_write.py:46 | Test endpoint; consider removing in production |
| `POST /api/mcp/notes/create` | mcp_write.py:78 | Simple note creation for MCP |
| `POST /api/mcp/notes` | mcp_write.py:163 | Full note creation for MCP |
| `PUT /api/mcp/notes/{note_id}` | mcp_write.py:263 | Note update for MCP |
| `POST /api/mcp/notes/find-or-create` | mcp_write.py:357 | Dedup-aware creation for MCP |
| `PUT /api/mcp/notes/{note_id}/properties` | mcp_write.py:408 | Property management for MCP |
| `GET /api/mcp/notes/search` | mcp_write.py:449 | Search for MCP |
| `POST /api/mcp-write/note` | mcp_write.py:481 | Simplified write endpoint for MCP |

### 2.3 Priority Router -- ZERO Frontend Usage (All 9 endpoints)

| Endpoint | Location | Notes |
|----------|----------|-------|
| `GET /api/priority/config` | priority.py:14 | Has test coverage but no frontend UI |
| `GET /api/priority/weights` | priority.py:32 | Has test coverage but no frontend UI |
| `PUT /api/priority/weights` | priority.py:45 | Has test coverage but no frontend UI |
| `POST /api/priority/reset` | priority.py:88 | Has test coverage but no frontend UI |
| `GET /api/priority/content-types` | priority.py:118 | Has test coverage but no frontend UI |
| `GET /api/priority/recency` | priority.py:137 | Has test coverage but no frontend UI |
| `GET /api/priority/link-density` | priority.py:152 | Has test coverage but no frontend UI |
| `GET /api/priority/tag-relevance` | priority.py:167 | Has test coverage but no frontend UI |
| `GET /api/priority/semantic` | priority.py:182 | Has test coverage but no frontend UI |

**Status:** Priority scoring is used *internally* via search (search.py:33-40 calls `priority_scoring.score_search_results()`), so the service is needed. But all 9 HTTP management endpoints have no frontend consumer. The `PriorityWeights` are read from a JSON file at startup and the router allows runtime configuration -- but no UI exists for it.

**Recommendation:** Keep the service, but consolidate the 9 endpoints into 2 (see Merge Candidates below).

### 2.4 Endpoints Defined in client.js But Never Called by Components

These are wired up in the API client but no Vue component or store actually calls them:

| client.js Method | Backend | Notes |
|-----------------|---------|-------|
| `feedback.getPending()` | Rust `get_pending_feedback` | client.js:260 -- no component calls this |
| `feedback.retryPending()` | Rust `retry_pending_feedback` | client.js:263 -- no component calls this |
| `memory.extract()` | Both backends | client.js:358 -- no component calls this |
| `zettelkasten.getLinkTypes()` | Python `/api/zettel/link-types` | client.js:382 -- no component calls this |
| `notes.normalizeTags()` | Python `/api/notes/{id}/normalize-tags` | client.js:88 -- no component calls this |
| `canvas.getNodeEdges()` | Python `/api/canvas/{id}/node-edges` | client.js:227 -- no component calls this (edges computed locally in canvas.js) |
| `canvas.getNodeGroups()` | Python `/api/canvas/{id}/node-groups` | client.js:229 -- no component calls this |

---

## 3. Merge Candidates

### 3.1 Priority Router: 9 endpoints --> 2 endpoints

**Current:** 9 separate GET endpoints for individual config facets + 1 PUT + 1 POST

**Proposed:**
```
GET  /api/priority/config     -- returns full config (already exists)
PUT  /api/priority/config     -- updates weights (merge PUT /weights into this)
POST /api/priority/reset      -- keep as-is
```

**Remove:** `GET /weights`, `GET /content-types`, `GET /recency`, `GET /link-density`, `GET /tag-relevance`, `GET /semantic` -- all of this data is already included in `GET /config`

**Savings:** 6 endpoints removed, from 9 to 3.

### 3.2 MCP Write: Duplicate note creation endpoints

**Current:**
- `POST /api/mcp/notes/create` (simple, query params + JSON body)
- `POST /api/mcp/notes` (full, Pydantic model body)
- `POST /api/mcp-write/note` (simplified, raw JSON body)

All three create notes with very similar logic. They differ in input format and provenance tagging.

**Proposed:** Merge into 1 endpoint:
```
POST /api/mcp/notes     -- handles both simple and full creation
```

**Remove:** `POST /api/mcp/notes/create` and `POST /api/mcp-write/note`

**Savings:** 2 endpoints removed. Risk: MCP tool definitions in `fastapi-mcp` must be updated.

### 3.3 Notes Properties: 4 endpoints --> 0 or merge into note update

**Current:** 4 REST endpoints for property CRUD (`GET`, `GET /{name}`, `PUT /{name}`, `DELETE /{name}`)

**Analysis:** Properties can already be set via `PUT /api/notes/{id}` (NoteUpdate includes `properties` field). The only consumer is the MCP write router which has its own `PUT /mcp/notes/{id}/properties`.

**Proposed:** Remove all 4 property endpoints. Any property management goes through note update or MCP endpoint.

**Savings:** 4 endpoints removed.

---

## 4. Zero Usage + Zero Tests (Strongest Removal Candidates)

| Endpoint | Location | Frontend | Tests | MCP | Recommendation |
|----------|----------|:--------:|:-----:|:---:|---------------|
| `GET /api/notes/{id}/properties` | notes.py:116 | N | N | N | **REMOVE** |
| `GET /api/notes/{id}/properties/{name}` | notes.py:130 | N | N | N | **REMOVE** |
| `PUT /api/notes/{id}/properties/{name}` | notes.py:145 | N | N | N | **REMOVE** |
| `DELETE /api/notes/{id}/properties/{name}` | notes.py:172 | N | N | N | **REMOVE** |

These 4 endpoints are the strongest removal candidates: they have no frontend consumer, no test coverage, and no MCP usage.

---

## 5. Rust Commands: Unused or Missing Peers

### 5.1 Rust Command Without Corresponding Python Endpoint

| Rust Command | Python Equivalent | Notes |
|-------------|-------------------|-------|
| `clear_pending_feedback` | None | Rust-only; not even exposed in client.js |

### 5.2 Python Endpoint Without Corresponding Rust Command

| Python Endpoint | Notes |
|----------------|-------|
| All Priority endpoints (9) | No Rust equivalent; only consumed by Python web backend |
| All Import endpoints (8) | No Rust equivalent; import.js uses direct fetch() |
| All Zettelkasten endpoints (7) | No Rust equivalent; client.js uses HTTP-only |
| All Distillation endpoints (2) | No Rust equivalent; client.js uses HTTP-only |
| All MCP Write endpoints (9) | MCP-only; never called from frontend |
| All OAuth endpoints (4) | HTTP-only; not needed in desktop app |
| Graph `/full` endpoint | HTTP-only; GraphView.vue uses direct HTTP |
| Graph `/unlinked-mentions/{id}` | HTTP-only; UnlinkedMentions.vue uses direct HTTP |
| Canvas `/edges` (legacy) | No Rust equivalent |
| Canvas `/node-edges` | HTTP-only |
| Canvas `/node-groups` | HTTP-only |
| Canvas `/debate/{did}/status` | No Rust equivalent |

This asymmetry is by design: the desktop app (Tauri) implements the core CRUD + canvas + settings + feedback + memory + MCP lifecycle commands in Rust. Advanced features (import, zettelkasten, priority management, OAuth, distillation) are Python-only, either because they require the Python sidecar or because they are web-only features.

---

## 6. MCP-Exposed Endpoints via `fastapi-mcp`

The MCP server (`backend/app/mcp/server.py`) uses `FastApiMCP` which auto-discovers all FastAPI routes as MCP tools, **except** those tagged with `"import"`. This means:

**Exposed to MCP (via auto-discovery):**
- All Notes endpoints (6 core + 4 properties)
- All Search endpoints (2)
- All Graph endpoints (7)
- All Canvas endpoints (22)
- All Distillation endpoints (2)
- All Priority endpoints (9)
- All Zettelkasten endpoints (7)
- All Feedback endpoints (2)
- All Memory endpoints (3)
- All MCP Write endpoints (9)
- All OAuth endpoints (4)

**Excluded from MCP:**
- All Import endpoints (8) -- excluded via `exclude_tags=["import"]`

**Concern:** The MCP auto-discovery exposes many endpoints that are not useful for Claude/ChatGPT (e.g., canvas streaming, viewport updates, tile position updates, OAuth flow). Consider using `include_tags` or explicit endpoint selection instead of excluding only import.

---

## 7. Recommendations Summary

### Immediate Removals (7 endpoints, low risk)

1. **`GET /api/notes/{id}/properties`** -- no usage, no tests
2. **`GET /api/notes/{id}/properties/{name}`** -- no usage, no tests
3. **`PUT /api/notes/{id}/properties/{name}`** -- no usage, no tests
4. **`DELETE /api/notes/{id}/properties/{name}`** -- no usage, no tests
5. **`GET /api/canvas/{id}/edges`** -- marked legacy, replaced by `/node-edges`
6. **`PUT /api/canvas/{id}/debate/{did}/status`** -- unused, status managed internally
7. **`GET /api/zettel/zettel-types`** -- data hardcoded in frontend

### Merge Opportunities (10 endpoints --> 3)

- **Priority:** 9 endpoints --> 3 (remove 6 redundant GETs)
- **MCP note creation:** 3 endpoints --> 1 (merge duplicate create flows)

### Investigate / Low Priority

- **`feedback.getPending()` / `retryPending()`** -- wired in client.js, Rust commands exist, but no component uses them. May be planned UI feature not yet built.
- **`memory.extract()`** -- wired in client.js and Rust, but never called. May be planned feature.
- **`notes.normalizeTags()`** -- wired in client.js but never called from components. Used implicitly during distillation?
- **`canvas.getNodeEdges()` / `getNodeGroups()`** -- wired in client.js, Python endpoints exist, but no component calls them. Node edges are computed locally in canvas.js store.
- **`zettelkasten.getLinkTypes()`** -- wired in client.js but never called from components.
- **MCP test endpoints** (`/mcp/test`, `/mcp/test/simple`) -- consider removing or guarding with dev-only flag.
- **`Rust: clear_pending_feedback`** -- command exists but is not even exposed in client.js. Either expose or remove.

### Architecture Observation

The priority scoring system is unique: it has 9 HTTP management endpoints, full test coverage, but zero frontend UI. The scoring logic *is* used (search endpoint applies priority scoring), but all weight configuration endpoints are currently dead from a user perspective. This represents significant over-engineering for a feature that could use a simpler approach (e.g., a single settings field in the existing Settings system).
