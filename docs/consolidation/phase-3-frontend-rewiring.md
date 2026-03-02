# Phase 3: Frontend Rewiring

## Goal
Rewrite `client.js` to use HTTP for **all** business logic (via the Python sidecar), removing the `invokeOrHttp` dual-path pattern. Delete Tauri event streaming from `canvas.js` (use HTTP SSE everywhere). The Rust backend retains only shell commands (settings, sidecar, MCP, folder picker).

## Why This Is the Big One
This phase touches the most files but is conceptually simple: every `invokeOrHttp('rust_command', ...)` becomes `api.verb('/endpoint')`. The canvas store goes from 1217 lines (with duplicated Tauri/HTTP streaming) to ~400 lines (HTTP SSE only).

---

## Task 1: Rewrite `client.js`

### Current Pattern (remove)
```javascript
const invokeOrHttp = async (command, params, httpFallback) => {
  if (isTauri()) {
    const invoke = await getTauriInvoke()
    if (invoke) return await invoke(command, params)
  }
  return httpFallback()
}
```

### New Pattern
```javascript
// In desktop mode, all API calls go to the Python sidecar via HTTP.
// The sidecar URL is resolved on startup from Tauri.
let _sidecarBaseUrl = null

async function getSidecarUrl() {
  if (_sidecarBaseUrl) return _sidecarBaseUrl
  if (isTauri()) {
    const { invoke } = await import('@tauri-apps/api/tauri')
    _sidecarBaseUrl = await invoke('get_sidecar_url')
    return _sidecarBaseUrl
  }
  return '' // Web mode — relative URLs
}

// Axios instance dynamically configured for sidecar or web
const api = axios.create({
  headers: { 'Content-Type': 'application/json' },
})

// On first request, set base URL
api.interceptors.request.use(async (config) => {
  if (!config.baseURL) {
    const base = await getSidecarUrl()
    config.baseURL = base ? `${base}/api` : '/api'
    api.defaults.baseURL = config.baseURL  // Cache for future requests
  }
  return config
})
```

### API Rewrite Summary

Every export changes from dual-path to HTTP-only:

```javascript
// BEFORE:
export const notes = {
  list: () => invokeOrHttp('list_notes', {}, () => api.get('/notes')),
  // ...
}

// AFTER:
export const notes = {
  list: () => api.get('/notes'),
  get: (id) => api.get(`/notes/${encodeURIComponent(id)}`),
  create: (data) => api.post('/notes', data),
  update: (id, data) => api.put(`/notes/${encodeURIComponent(id)}`, data),
  delete: (id) => api.delete(`/notes/${encodeURIComponent(id)}`),
  reindex: () => api.post('/notes/reindex'),
  distill: (id, request) => api.post(`/notes/${encodeURIComponent(id)}/distill`, request),
  normalizeTags: (id) => api.post(`/notes/${encodeURIComponent(id)}/normalize-tags`),
}
```

Full mapping of all API groups:

| Group | Methods | Changes |
|-------|---------|---------|
| `notes` | 8 | Remove all `invokeOrHttp`, use `api.*` |
| `search` | 2 | Remove all `invokeOrHttp` |
| `graph` | 7 | Remove all `invokeOrHttp` |
| `canvas` | 18 | Remove all `invokeOrHttp` — streaming handled in store |
| `feedback` | 5 | Remove `invokeOrHttp`, keep `getSystemInfo` as Tauri-only |
| `settings` | 7 | **Keep as Tauri IPC** — folder picker needs native dialog |
| `mcp` | 2 | **Keep as Tauri IPC** — binary path is local |
| `memory` | 3 | Remove all `invokeOrHttp` |
| `zettelkasten` | 4 | Already HTTP-only, no changes |
| `oauth` | 4 | Already HTTP-only, no changes |

### Commands That Stay as Tauri IPC
These commands need native OS access and can't go through HTTP:

```javascript
// These remain as Tauri invoke() calls:
export const settings = {
  pickVaultFolder: async () => {
    // Native file dialog — must stay as IPC
    const { invoke } = await import('@tauri-apps/api/tauri')
    return await invoke('pick_vault_folder')
  },
  // All other settings methods now go through HTTP:
  get: () => api.get('/settings'),
  getStatus: () => api.get('/settings/status'),
  update: (data) => api.put('/settings', data),
  // ...
}

export const mcp = {
  // MCP status checks local binary — stays as IPC
  getStatus: () => invokeOrHttp('get_mcp_status', {}, ...),
  getConfigSnippet: () => invokeOrHttp('get_mcp_config_snippet', {}, ...),
}

// Sidecar management — IPC only
export const sidecar = {
  getStatus: () => invoke('get_sidecar_status'),
  getUrl: () => invoke('get_sidecar_url'),
}
```

---

## Task 2: Simplify Canvas Store (Streaming)

The canvas store has **massive duplication** between Tauri event streaming and HTTP SSE. After this phase, only HTTP SSE remains.

### Delete entirely:
- `setupTauriStreamListener()` function
- `waitForModelsComplete()` function
- All `if (isDesktopApp()) { ... Tauri path ... } else { ... Web path ... }` branches

### Keep only:
- HTTP SSE stream processing (`processSSEStream`, `processDebateSSEStream`)
- The SSE URL needs to use the sidecar base URL:

```javascript
async function sendPrompt(prompt, models, ...) {
  const sessionId = currentSession.value.id
  const baseUrl = await getSidecarUrl()

  const response = await fetch(`${baseUrl}/api/canvas/${sessionId}/prompt`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prompt, models, ... })
  })

  if (!response.ok) throw new Error(`HTTP error: ${response.status}`)
  return await processSSEStream(response, sessionId, models)
}
```

### Functions simplified (line count reduction):
| Function | Before | After | Savings |
|----------|--------|-------|---------|
| `sendPrompt` | 82 lines | 25 lines | -57 |
| `startDebate` | 115 lines | 40 lines | -75 |
| `continueDebate` | 115 lines | 55 lines | -60 |
| `addModelToTile` | 90 lines | 40 lines | -50 |
| `regenerateResponse` | 90 lines | 40 lines | -50 |
| **Total** | **~492** | **~200** | **~292 lines** |

Also delete:
- `import { isDesktopApp } from '@/api/client'` from canvas store (no longer needed)
- `@tauri-apps/api/event` import (no longer needed in canvas store)

---

## Task 3: Update `getSidecarUrl` for SSE

The `fetch()` calls for SSE streaming need the full base URL, not just the API prefix. Create a helper:

```javascript
// In client.js
export async function getApiBaseUrl() {
  const base = await getSidecarUrl()
  return base ? `${base}/api` : '/api'
}
```

The canvas store imports this and uses it for `fetch()` calls:

```javascript
import { getApiBaseUrl } from '@/api/client'

// In sendPrompt:
const apiBase = await getApiBaseUrl()
const response = await fetch(`${apiBase}/canvas/${sessionId}/prompt`, { ... })
```

---

## Task 4: Feedback `getSystemInfo` Special Case

`getSystemInfo` collects desktop-specific info (app version, platform from Tauri). This stays as IPC with HTTP fallback:

```javascript
export const feedback = {
  submit: (data) => api.post('/feedback', data),
  status: () => api.get('/feedback/status'),
  getSystemInfo: async (currentPage = null) => {
    if (isTauri()) {
      const { invoke } = await import('@tauri-apps/api/tauri')
      try {
        return await invoke('get_system_info', { currentPage })
      } catch (e) {
        console.error('Failed to get system info:', e)
      }
    }
    return {
      platform: navigator.platform || 'Unknown',
      app_version: '1.0.0',
      runtime: 'web-browser',
      current_page: currentPage || window.location.pathname,
    }
  },
}
```

---

## Files Modified
| File | Action |
|------|--------|
| `frontend/src/api/client.js` | **Rewrite** — remove invokeOrHttp, add sidecar URL resolution |
| `frontend/src/stores/canvas.js` | **Major edit** — delete all Tauri streaming, keep SSE only |
| `frontend/src/stores/notes.js` | **Minor edit** — remove any `isDesktopApp` references |
| `frontend/src/components/SettingsModal.vue` | **Minor edit** — update API calls if needed |
| Any component importing `isDesktopApp` | **Edit** — remove or update usage |

## Validation
- Web mode (`npm run dev`) works exactly as before — no regressions
- Desktop mode (`npm run tauri:dev`) routes all API calls through sidecar
- Canvas streaming works in both modes (SSE only)
- Settings folder picker still works (native dialog)
- MCP status still works (reads binary path)
- No `@tauri-apps/api/tauri` imports remain in business logic code (only in settings/mcp/sidecar)
- Console has no Tauri invoke errors

## Risk Mitigation
- **Feature flag approach**: Add `VITE_USE_SIDECAR=true` env var. When false, fall back to old `invokeOrHttp` behavior. Remove the flag after Phase 4 validation.
- **Incremental rollout**: Can rewire one API group at a time (notes first, then search, then canvas, etc.)
