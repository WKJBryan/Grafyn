# Backend Consolidation: Python Sidecar Architecture

## The Problem
Grafyn has two backends with duplicated business logic:
- **Python/FastAPI** (web mode): 14 services, 83 HTTP endpoints, semantic search via LanceDB
- **Rust/Tauri** (desktop mode): 8 services, 52 IPC commands, full-text search via Tantivy

The frontend uses an `invokeOrHttp()` dual-path pattern in `client.js` — every API call has both a Tauri IPC path and an HTTP fallback. This means every feature must be implemented twice.

## The Solution
Bundle the Python backend as a compiled sidecar (`grafyn-server`) inside the Tauri app. The desktop app launches it on startup and routes all business logic through HTTP. Rust is stripped to a thin shell (settings, sidecar management, native dialogs).

```
Before:                          After:
┌─────────────┐                  ┌─────────────┐
│  Vue Frontend│                  │  Vue Frontend│
├──────┬──────┤                  └──────┬──────┘
│Tauri │ HTTP │                         │ HTTP only
│ IPC  │      │                  ┌──────▼──────┐
├──────┤      │                  │ Tauri Shell  │ (settings, sidecar, dialogs)
│ Rust │Python│                  └──────┬──────┘
│Backend│Backend│                       │ localhost
└──────┴──────┘                  ┌──────▼──────┐
                                 │grafyn-server │ (PyInstaller binary)
                                 │ Python/FastAPI│
                                 └─────────────┘
```

## Phases

| Phase | Description | Risk | Reversible? |
|-------|-------------|------|-------------|
| [Phase 1](phase-1-python-settings-chat.md) | Python Settings + Chat API | Low | Yes |
| [Phase 2](phase-2-sidecar-infrastructure.md) | Sidecar Infrastructure (PyInstaller + Tauri) | Medium | Yes |
| [Phase 3](phase-3-frontend-rewiring.md) | Frontend Rewiring (remove invokeOrHttp) | Medium | Feature flag |
| [Phase 4](phase-4-rust-cleanup.md) | Rust Cleanup (delete ~5000 lines) | Low | Git revert |
| [Phase 5](phase-5-mcp-rewrite.md) | MCP Binary Rewrite (HTTP proxy) | Low | Yes |
| [Phase 6](phase-6-cicd-update.md) | CI/CD Update (PyInstaller in pipeline) | Medium | Yes |
| [Phase 7](phase-7-chat-with-notes.md) | Chat with Notes Feature (RAG) | Low | New feature |

## Execution Order

Phases 1-6 are sequential (each depends on the previous). Phase 7 can start after Phase 1 (only needs the Python backend).

```
Phase 1 ──→ Phase 2 ──→ Phase 3 ──→ Phase 4 ──→ Phase 5 ──→ Phase 6
   │
   └──→ Phase 7 (parallel)
```

## What Gets Deleted

| Category | Files | Lines |
|----------|-------|-------|
| Rust commands | 7 modules | ~1,850 |
| Rust services | 7 modules | ~2,450 |
| Rust models | 4 modules | ~700 |
| Cargo dependencies | 12 crates | N/A |
| Frontend dual-path code | canvas.js streaming | ~300 |
| **Total** | **~18 files** | **~5,300 lines** |

## What Gets Added

| Category | Files | Lines |
|----------|-------|-------|
| Python settings service | 1 | ~120 |
| Python settings router | 1 | ~60 |
| Python chat router | 1 | ~150 |
| Rust sidecar manager | 1 | ~100 |
| Rust sidecar commands | 1 | ~30 |
| PyInstaller spec | 1 | ~50 |
| Chat frontend | 2 | ~250 |
| **Total** | **~8 files** | **~760 lines** |

**Net reduction: ~4,500 lines**

## Trade-offs

### Pros
- Single backend to maintain (Python)
- Semantic search in desktop mode (LanceDB, not just Tantivy keyword search)
- Chat-with-notes feature unlocked (RAG over knowledge base)
- Faster Rust compilation (stripped dependencies)
- Smaller Rust binary (~15MB → ~5MB for the shell)

### Cons
- Larger installer (~30MB → ~350-500MB with PyTorch/sentence-transformers)
- Sidecar startup adds ~2-5s cold start
- Extra process running (Python alongside Tauri)
- More complex CI (Python + Rust builds per platform)

### Mitigations
- Bundle size: Switch to ONNX runtime later (~80MB instead of ~400MB)
- Cold start: Show loading indicator, app UI is immediately usable
- CI complexity: Pip caching keeps build times reasonable
