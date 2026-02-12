# Grafyn Cleanup Action Plan

**Date:** 2026-02-11
**Sources:** abstractions-report.md, duplication-report.md, api-surface-report.md

---

## Priority Legend

| Priority | Criteria |
|----------|----------|
| **P0 -- Critical** | Data corruption risk, security vulnerability, or cross-platform incompatibility |
| **P1 -- High Impact** | Large line reduction (100+), dead code removal, or significant simplification |
| **P2 -- Medium** | Moderate cleanup (30-100 lines), improved consistency |
| **P3 -- Low** | Cosmetic, convention alignment, or future-proofing |

---

## P0 -- Critical: Cross-Platform Compatibility & Security

These issues cause data corruption when users switch between web and desktop modes, or represent security gaps.

### 1. Unify timestamp field names
- **Problem:** Python uses `created`/`modified`, Rust uses `created_at`/`updated_at`. Notes saved by one backend have broken timestamps in the other.
- **Fix:** Standardize on `created_at`/`updated_at` in both backends.
- **Files:** `backend/app/models/note.py`, `backend/app/services/knowledge_store.py`, `frontend/src-tauri/src/models/note.rs`, `frontend/src-tauri/src/services/knowledge_store.rs`
- **Risk:** Medium -- requires migration of existing note frontmatter. Existing notes with `created`/`modified` need backwards-compat reading.
- **Impact:** Fixes data portability between web and desktop.

### 2. Unify note ID generation
- **Problem:** Python produces `My_Note` (underscores, preserves case), Rust produces `my-note` (lowercase hyphens with dedup). A note created in web mode cannot be found by ID in desktop mode.
- **Fix:** Pick one algorithm and implement in both. Recommend Rust's approach (lowercase hyphens) -- it's URL-friendly and collision-resistant.
- **Files:** `backend/app/services/knowledge_store.py` (L60-61), `frontend/src-tauri/src/services/knowledge_store.rs` (L127-153)
- **Risk:** High -- existing notes have Python-style IDs. Needs migration or backwards-compat ID resolution.
- **Impact:** Fixes cross-platform note lookup.

### 3. Add path traversal protection to Rust KnowledgeStore
- **Problem:** Python sanitizes note IDs and validates paths with `relative_to()` (L42-56). Rust does `vault_path.join(format!("{}.md", id))` with zero sanitization (L156-158).
- **Fix:** Port Python's sanitization logic to Rust: strip `..`, validate resolved path is within vault.
- **Files:** `frontend/src-tauri/src/services/knowledge_store.rs` (L156-158)
- **Risk:** Low -- purely additive security fix.
- **Impact:** Closes security vulnerability in desktop app.

### 4. Unify debate round schema
- **Problem:** Python stores `List[Dict[str, str]]`, Rust stores structured `Vec<DebateRound>`. Debate sessions are not portable between platforms.
- **Fix:** Adopt the Rust structured format in both (it's richer). Update Python's `CanvasSession` model and store.
- **Files:** `backend/app/models/canvas.py`, `backend/app/services/canvas_store.py`, `frontend/src-tauri/src/models/canvas.rs`
- **Risk:** Medium -- existing canvas session JSON files with Python format need migration.
- **Impact:** Fixes canvas portability.

### 5. Unify default tile dimensions
- **Problem:** Python defaults to 280x200, Rust to 400x300. Canvas sessions render differently.
- **Fix:** Pick one (recommend 400x300 -- Rust's default gives more space for LLM responses).
- **Files:** `backend/app/models/canvas.py`, `frontend/src-tauri/src/models/canvas.rs`
- **Risk:** Low -- only affects new tiles.
- **Impact:** Visual consistency.

---

## P1 -- High Impact: Dead Code & Over-Engineering Removal

### 6. Remove 7 orphaned Python endpoints
- **Endpoints:**
  1. `GET /api/notes/{id}/properties` (notes.py:116) -- zero usage, zero tests
  2. `GET /api/notes/{id}/properties/{name}` (notes.py:130) -- zero usage, zero tests
  3. `PUT /api/notes/{id}/properties/{name}` (notes.py:145) -- zero usage, zero tests
  4. `DELETE /api/notes/{id}/properties/{name}` (notes.py:172) -- zero usage, zero tests
  5. `GET /api/canvas/{id}/edges` (canvas.py:171) -- legacy, replaced by `/node-edges`
  6. `PUT /api/canvas/{id}/debate/{did}/status` (canvas.py:980) -- managed internally
  7. `GET /api/zettel/zettel-types` (zettelkasten.py:230) -- hardcoded in frontend
- **Lines saved:** ~120
- **Risk:** Low -- verified zero frontend usage, zero tests (properties), no MCP consumers.

### 7. Collapse Priority Scoring system (2 services + 9 endpoints --> 1 service + 3 endpoints)
- **Problem:** `PriorityScoringService` + `PrioritySettingsService` + `PrioritySettings` model + 9 HTTP endpoints. Zero frontend UI exists for any of them. The scoring itself IS used internally by search, but the HTTP management surface is dead weight.
- **Fix:**
  1. Merge `PrioritySettingsService` into `PriorityScoringService` (add `save()`/`load()` directly)
  2. Delete `PrioritySettings` wrapper model
  3. Reduce router from 9 endpoints to 3: `GET /config`, `PUT /config`, `POST /reset`
  4. Remove duplicate Pydantic validation in router (already enforced by model)
- **Files:** `backend/app/services/priority_scoring.py`, `backend/app/services/priority_settings.py` (DELETE), `backend/app/routers/priority.py`
- **Lines saved:** ~200
- **Risk:** Low -- no frontend consumers. Only risk: MCP auto-exposes these, so any MCP client using them (unlikely) would break.

### 8. Flatten dependency helpers
- **Problem:** 12 one-line functions in `utils/dependencies.py` that just return `request.app.state.X`. Not using FastAPI `Depends()` -- called manually in handler bodies.
- **Fix:** Either:
  - (A) Convert to proper `Depends()` callables (idiomatic FastAPI), or
  - (B) Delete file, inline `request.app.state.X` in each router
- **Recommendation:** Option (A) -- proper DI makes testing easier and is more Pythonic.
- **Files:** `backend/app/utils/dependencies.py` (90 lines), `backend/app/utils/__init__.py` (32 lines), all routers that import helpers
- **Lines saved:** ~50-120 depending on approach
- **Risk:** Low -- mechanical refactor, no behavior change.

### 9. Deduplicate ContentType/NoteType/PropertyType enums
- **Problem:** `ContentType` defined identically in 3 files: `models/note.py:17-25`, `services/priority_scoring.py:14-21`, `mcp/write_tools.py:7-14`. Same for `NoteType` and `PropertyType`.
- **Fix:** Import from `models/note.py` in both other files. Delete duplicate definitions.
- **Lines saved:** ~30
- **Risk:** Very low -- import change only.

---

## P2 -- Medium: Consistency & Cleanup

### 10. Merge 3 MCP note creation endpoints into 1
- **Problem:** Three endpoints with near-identical logic:
  - `POST /api/mcp/notes/create` (simple)
  - `POST /api/mcp/notes` (full)
  - `POST /api/mcp-write/note` (simplified)
- **Fix:** Consolidate into `POST /api/mcp/notes` with optional fields.
- **Files:** `backend/app/routers/mcp_write.py`
- **Lines saved:** ~80
- **Risk:** Medium -- MCP tool definitions for Claude/ChatGPT would need updating. Existing MCP clients may break.

### 11. Move memory router inline models to models/
- **Problem:** 8 Pydantic models defined inline in `backend/app/routers/memory.py:17-68`, breaking the convention that models live in `backend/app/models/`.
- **Fix:** Move to `backend/app/models/memory.py`. Drop single-field wrapper models (`RecallResponse`, `ContradictionsResponse`, `ExtractResponse`) -- return `List[X]` directly.
- **Lines saved:** ~20
- **Risk:** Low -- internal refactor.

### 12. Restrict MCP auto-discovery scope
- **Problem:** `fastapi-mcp` auto-exposes ALL routes to Claude/ChatGPT (except import). This includes canvas viewport updates, tile positions, OAuth flow, and priority config -- none of which are useful for AI assistants.
- **Fix:** Switch from `exclude_tags=["import"]` to `include_tags=["notes", "search", "graph", "mcp-write", "memory"]` (explicit allowlist).
- **Files:** `backend/app/mcp/server.py`
- **Risk:** Medium -- must ensure all desired MCP tools are tagged correctly. Test with Claude Desktop after change.

### 13. Clean up orphaned client.js wiring
- **Problem:** Several methods defined in `client.js` but never called by any component:
  - `feedback.getPending()`, `feedback.retryPending()` -- UI not built yet
  - `memory.extract()` -- UI not built yet
  - `notes.normalizeTags()` -- no component calls it
  - `canvas.getNodeEdges()`, `canvas.getNodeGroups()` -- edges computed locally
  - `zettelkasten.getLinkTypes()` -- no component calls it
- **Fix:** Either build the UI (if planned) or remove from client.js. Add `// TODO: not yet wired to UI` comments at minimum.
- **Risk:** Low -- removing unused JS functions.

### 14. Remove Rust `clear_pending_feedback` or expose in client.js
- **Problem:** Command exists in `commands/feedback.rs` but is not exposed in `client.js` at all.
- **Fix:** Either add to client.js (if planned) or remove the command.
- **Risk:** Very low.

---

## P3 -- Low: Future Improvements

### 15. Unify case sensitivity in graph wikilink resolution
- **Problem:** Python resolves wikilinks case-sensitively; Rust lowercases all titles. `[[My Note]]` and `[[my note]]` behave differently across backends.
- **Fix:** Make Python case-insensitive to match Rust (more user-friendly).
- **Files:** `backend/app/services/graph_index.py` (L57)
- **Risk:** Low -- may surface new links that were previously unresolved.

### 16. Extract Zettelkasten templates from DistillationService
- **Problem:** `distillation.py` is 1,611 lines. The 6 `_render_*_note` functions and `ZETTELKASTEN_EXTRACTION_PROMPT` are data-like and could live separately.
- **Fix:** Move to `backend/app/services/zettelkasten_templates.py`.
- **Lines moved:** ~300 (not saved, just relocated)
- **Risk:** Very low -- no behavior change.

### 17. Remove MCP test endpoints in production
- **Problem:** `GET /api/mcp/test` and `POST /api/mcp/test/simple` exist primarily for development.
- **Fix:** Guard with `if settings.environment == "development"` or remove entirely.
- **Risk:** Very low.

### 18. Consider feature parity for desktop
- **Problem:** 9 Python-only services (distillation, import, link discovery, priority scoring, parsers, etc.) have no Rust equivalent. Desktop users cannot use these features.
- **Not a cleanup item** -- this is a feature roadmap decision. Document the gap and decide if/when to port.

---

## Impact Summary

| Category | Items | Lines Removed/Simplified | Risk |
|----------|-------|--------------------------|------|
| P0 Critical compat fixes | 5 items | ~50 lines changed (mostly alignment) | Medium-High (needs migration) |
| P1 Dead code & over-engineering | 4 items | ~370-470 lines | Low |
| P2 Consistency & cleanup | 5 items | ~100-130 lines | Low-Medium |
| P3 Future improvements | 4 items | ~300 lines relocated | Very Low |
| **Total** | **18 items** | **~520-650 net lines reduced** | |

---

## Recommended Execution Order

**Phase 1 -- Quick wins (low risk, immediate value):**
- Item 9: Deduplicate enums (5 min)
- Item 6: Remove 7 orphaned endpoints (15 min)
- Item 3: Add Rust path traversal protection (15 min)
- Item 5: Unify tile dimensions (5 min)

**Phase 2 -- Priority system overhaul:**
- Item 7: Collapse priority scoring (merge services, reduce endpoints)
- Item 8: Flatten dependency helpers

**Phase 3 -- Cross-platform compat (needs migration strategy):**
- Item 1: Unify timestamp fields
- Item 2: Unify note ID generation
- Item 4: Unify debate round schema
- Item 15: Unify case sensitivity

**Phase 4 -- Polish:**
- Items 10-14, 16-17: MCP cleanup, client.js cleanup, template extraction

---

## Files to Delete (After Cleanup)

| File | Reason |
|------|--------|
| `backend/app/services/priority_settings.py` | Merged into `priority_scoring.py` |

## Endpoints to Remove (Total: 13)

| Endpoint | Reason |
|----------|--------|
| `GET /api/notes/{id}/properties` | Zero usage + zero tests |
| `GET /api/notes/{id}/properties/{name}` | Zero usage + zero tests |
| `PUT /api/notes/{id}/properties/{name}` | Zero usage + zero tests |
| `DELETE /api/notes/{id}/properties/{name}` | Zero usage + zero tests |
| `GET /api/canvas/{id}/edges` | Legacy, replaced by `/node-edges` |
| `PUT /api/canvas/{id}/debate/{did}/status` | Managed internally |
| `GET /api/zettel/zettel-types` | Data hardcoded in frontend |
| `GET /api/priority/weights` | Subset of `/config` |
| `GET /api/priority/content-types` | Subset of `/config` |
| `GET /api/priority/recency` | Subset of `/config` |
| `GET /api/priority/link-density` | Subset of `/config` |
| `GET /api/priority/tag-relevance` | Subset of `/config` |
| `GET /api/priority/semantic` | Subset of `/config` |
