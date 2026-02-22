import axios from 'axios'

// Check if running in Tauri
const isTauri = () => typeof window !== 'undefined' && window.__TAURI__ !== undefined

// Lazy load Tauri API to avoid errors in web environment
let tauriInvoke = null
const getTauriInvoke = async () => {
  if (!isTauri()) return null
  if (tauriInvoke) return tauriInvoke
  const { invoke } = await import('@tauri-apps/api/tauri')
  tauriInvoke = invoke
  return invoke
}

// Create axios instance with base configuration
const api = axios.create({
  baseURL: '/api',
  headers: {
    'Content-Type': 'application/json',
  },
})

// Request interceptor to add auth token if available
api.interceptors.request.use(
  (config) => {
    const token = localStorage.getItem('auth_token')
    if (token) {
      config.headers.Authorization = `Bearer ${token}`
    }
    return config
  },
  (error) => {
    return Promise.reject(error)
  }
)

// Response interceptor for error handling
api.interceptors.response.use(
  (response) => response.data,
  (error) => {
    if (error.response?.status === 401) {
      // Unauthorized - clear token and redirect to login
      localStorage.removeItem('auth_token')
      window.location.href = '/login'
    }
    return Promise.reject(error)
  }
)

// Helper to call Tauri command or fallback to HTTP
const invokeOrHttp = async (command, params, httpFallback) => {
  if (isTauri()) {
    const invoke = await getTauriInvoke()
    if (invoke) {
      try {
        return await invoke(command, params)
      } catch (e) {
        console.error(`Tauri invoke failed for ${command}:`, e)
        throw e
      }
    }
  }
  return httpFallback()
}

// Notes API
export const notes = {
  list: () => invokeOrHttp('list_notes', {}, () => api.get('/notes')),

  get: (id) => invokeOrHttp('get_note', { id }, () => api.get(`/notes/${encodeURIComponent(id)}`)),

  create: (data) =>
    invokeOrHttp('create_note', { note: data }, () => api.post('/notes', data)),

  update: (id, data) =>
    invokeOrHttp('update_note', { id, update: data }, () =>
      api.put(`/notes/${encodeURIComponent(id)}`, data)
    ),

  delete: (id) =>
    invokeOrHttp('delete_note', { id }, () => api.delete(`/notes/${encodeURIComponent(id)}`)),

  reindex: () => invokeOrHttp('reindex', {}, () => api.post('/notes/reindex')),

  // Distillation API (HTTP only for now)
  distill: (id, request) => api.post(`/notes/${encodeURIComponent(id)}/distill`, request),
  normalizeTags: (id) => api.post(`/notes/${encodeURIComponent(id)}/normalize-tags`),
}

// Search API
export const search = {
  query: (q, { limit = 10, semantic = true } = {}) =>
    invokeOrHttp('search_notes', { query: q, limit }, () =>
      api.get('/search', { params: { q, limit, semantic } })
    ),

  similar: (noteId, limit = 5) =>
    invokeOrHttp('find_similar', { noteId, limit }, () =>
      api.get(`/search/similar/${encodeURIComponent(noteId)}`, { params: { limit } })
    ),
}

// Graph API
export const graph = {
  backlinks: (id) =>
    invokeOrHttp('get_backlinks', { noteId: id }, () =>
      api.get(`/graph/backlinks/${encodeURIComponent(id)}`)
    ),

  outgoing: (id) =>
    invokeOrHttp('get_outgoing', { noteId: id }, () =>
      api.get(`/graph/outgoing/${encodeURIComponent(id)}`)
    ),

  neighbors: (id, depth = 1) =>
    invokeOrHttp('get_neighbors', { noteId: id }, () =>
      api.get(`/graph/neighbors/${encodeURIComponent(id)}`, { params: { depth } })
    ),

  unlinkedMentions: (id) => api.get(`/graph/unlinked-mentions/${encodeURIComponent(id)}`),

  rebuild: () => invokeOrHttp('rebuild_graph', {}, () => api.post('/graph/rebuild')),

  full: () => invokeOrHttp('get_full_graph', {}, () => api.get('/graph/full')),

  unlinked: () => invokeOrHttp('get_unlinked', {}, () => api.get('/graph/unlinked')),
}

// OAuth API (HTTP only - not needed in desktop app)
export const oauth = {
  getAuthorizationUrl: (provider) => api.get(`/oauth/authorize/${provider}`),
  exchangeCode: (provider, code) => api.post(`/oauth/callback/${provider}`, { code }),
  getUser: () => api.get('/oauth/user'),
  logout: () => api.post('/oauth/logout'),
}

// Canvas API
export const canvas = {
  list: () => invokeOrHttp('list_sessions', {}, () => api.get('/canvas')),

  get: (id) =>
    invokeOrHttp('get_session', { id }, () => api.get(`/canvas/${encodeURIComponent(id)}`)),

  create: (data) =>
    invokeOrHttp('create_session', { session: data }, () => api.post('/canvas', data)),

  update: (id, data) =>
    invokeOrHttp('update_session', { id, update: data }, () =>
      api.put(`/canvas/${encodeURIComponent(id)}`, data)
    ),

  delete: (id) =>
    invokeOrHttp('delete_session', { id }, () => api.delete(`/canvas/${encodeURIComponent(id)}`)),

  getModels: () =>
    invokeOrHttp('get_available_models', {}, () => api.get('/canvas/models/available')),

  // Send prompt to models (Tauri: returns tile_id, streams via events)
  sendPrompt: (sessionId, request) =>
    invokeOrHttp('send_prompt', { sessionId, request }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/prompt`, request)
    ),

  updateTilePosition: (sessionId, tileId, position) =>
    invokeOrHttp('update_tile_position', { sessionId, tileId, position }, () =>
      api.put(
        `/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/position`,
        position
      )
    ),

  updateLLMNodePosition: (sessionId, tileId, modelId, position) =>
    invokeOrHttp('update_llm_node_position', { sessionId, tileId, modelId, position }, () =>
      api.put(
        `/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/responses/${encodeURIComponent(modelId)}/position`,
        position
      )
    ),

  autoArrange: (sessionId, positions) =>
    invokeOrHttp('auto_arrange', { sessionId, positions }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/arrange`, { positions })
    ),

  deleteTile: (sessionId, tileId) =>
    invokeOrHttp('delete_tile', { sessionId, tileId }, () =>
      api.delete(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}`)
    ),

  deleteResponse: (sessionId, tileId, modelId) =>
    invokeOrHttp('delete_response', { sessionId, tileId, modelId }, () =>
      api.delete(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/responses/${encodeURIComponent(modelId)}`)
    ),

  updateViewport: (sessionId, viewport) =>
    invokeOrHttp('update_viewport', { sessionId, viewport }, () =>
      api.put(`/canvas/${encodeURIComponent(sessionId)}/viewport`, viewport)
    ),

  exportToNote: (sessionId) =>
    invokeOrHttp('export_to_note', { sessionId }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/export-note`)
    ),

  // Debate commands
  startDebate: (sessionId, request) =>
    invokeOrHttp('start_debate', { sessionId, request }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/debate`, request)
    ),

  continueDebate: (sessionId, debateId, request) =>
    invokeOrHttp('continue_debate', { sessionId, debateId, request }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/debate/${encodeURIComponent(debateId)}/continue`, request)
    ),

  addModelsToTile: (sessionId, tileId, request) =>
    invokeOrHttp('add_models_to_tile', { sessionId, tileId, request }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/tile/${encodeURIComponent(tileId)}/add-models`, request)
    ),

  regenerateResponse: (sessionId, tileId, modelId) =>
    invokeOrHttp('regenerate_response', { sessionId, tileId, modelId }, () =>
      api.post(`/canvas/${encodeURIComponent(sessionId)}/tile/${encodeURIComponent(tileId)}/regenerate/${encodeURIComponent(modelId)}`)
    ),

  getNodeEdges: (sessionId) => api.get(`/canvas/${encodeURIComponent(sessionId)}/node-edges`),

  getNodeGroups: (sessionId) => api.get(`/canvas/${encodeURIComponent(sessionId)}/node-groups`),
}

// Feedback API
export const feedback = {
  submit: (data) =>
    invokeOrHttp('submit_feedback', { feedback: data }, () => api.post('/feedback', data)),

  status: () =>
    invokeOrHttp('feedback_status', {}, () => api.get('/feedback/status')),

  getSystemInfo: async (currentPage = null) => {
    if (isTauri()) {
      const invoke = await getTauriInvoke()
      if (invoke) {
        try {
          return await invoke('get_system_info', { currentPage })
        } catch (e) {
          console.error('Failed to get system info from Tauri:', e)
        }
      }
    }
    // Fallback for web mode
    return {
      platform: navigator.platform || 'Unknown',
      app_version: '1.0.0',
      runtime: 'web-browser',
      current_page: currentPage || window.location.pathname,
    }
  },

  getPending: () =>
    invokeOrHttp('get_pending_feedback', {}, () => api.get('/feedback/pending')),

  retryPending: () =>
    invokeOrHttp('retry_pending_feedback', {}, () => api.post('/feedback/retry')),
}

// Settings API (Desktop only)
export const settings = {
  get: () => invokeOrHttp('get_settings', {}, () => Promise.resolve(null)),

  getStatus: () =>
    invokeOrHttp('get_settings_status', {}, () =>
      // Web mode doesn't need setup
      Promise.resolve({ needs_setup: false, has_vault_path: true, has_openrouter_key: false })
    ),

  update: (data) =>
    invokeOrHttp('update_settings', { update: data }, () => Promise.resolve(data)),

  completeSetup: () =>
    invokeOrHttp('complete_setup', {}, () => Promise.resolve()),

  pickVaultFolder: async () => {
    if (isTauri()) {
      const invoke = await getTauriInvoke()
      if (invoke) {
        try {
          return await invoke('pick_vault_folder')
        } catch (e) {
          console.error('Failed to pick vault folder:', e)
          throw e
        }
      }
    }
    return null
  },

  validateOpenRouterKey: (apiKey) =>
    invokeOrHttp('validate_openrouter_key', { apiKey }, async () => {
      // Web mode: validate via OpenRouter API directly
      try {
        const response = await fetch('https://openrouter.ai/api/v1/models', {
          headers: { Authorization: `Bearer ${apiKey}` },
        })
        return response.ok
      } catch {
        return false
      }
    }),

  getOpenRouterStatus: () =>
    invokeOrHttp('get_openrouter_status', {}, () =>
      Promise.resolve({ has_key: false, is_configured: false })
    ),
}

// MCP API (Desktop only — native Rust MCP server)
export const mcp = {
  getStatus: () =>
    invokeOrHttp('get_mcp_status', {}, () =>
      Promise.resolve({ available: false, binary_path: null, config_snippet: '' })
    ),

  getConfigSnippet: () =>
    invokeOrHttp('get_mcp_config_snippet', {}, () => Promise.resolve('')),
}

// Memory API
export const memory = {
  recall: (query, contextNoteIds = [], limit = 5) =>
    invokeOrHttp('recall_relevant', { request: { query, context_note_ids: contextNoteIds, limit } }, () =>
      api.post('/memory/recall', { query, context_note_ids: contextNoteIds, limit })
    ),

  contradictions: (noteId) =>
    invokeOrHttp('find_contradictions', { noteId }, () =>
      api.post(`/memory/contradictions/${encodeURIComponent(noteId)}`)
    ),

  extract: (messages) =>
    invokeOrHttp('extract_claims', { request: { messages } }, () =>
      api.post('/memory/extract', { messages })
    ),
}

// Zettelkasten Link Discovery API (HTTP only — uses Python backend)
export const zettelkasten = {
  discoverLinks: (noteId, mode = 'suggested', maxLinks = 10) =>
    api.get(`/zettel/notes/${encodeURIComponent(noteId)}/discover-links`, {
      params: { mode, max_links: maxLinks },
    }),

  applyLinks: (noteId, linkIds) =>
    api.post(`/zettel/notes/${encodeURIComponent(noteId)}/discover-links/apply`, {
      link_ids: linkIds,
    }),

  createLink: (sourceId, targetId, linkType = 'related') =>
    api.post(
      `/zettel/notes/${encodeURIComponent(sourceId)}/link/${encodeURIComponent(targetId)}`,
      { link_type: linkType }
    ),

  getLinkTypes: () => api.get('/zettel/link-types'),
}

// Utility function to check if we're in Tauri environment
export const isDesktopApp = isTauri

export default api
