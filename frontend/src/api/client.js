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

  full: () => api.get('/graph/full'),

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

  // Send prompt to models
  sendPrompt: (sessionId, request) =>
    invokeOrHttp('send_prompt', { sessionId, request }, () =>
      // For HTTP, this is typically done via SSE - simplified here
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
    api.put(
      `/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/responses/${encodeURIComponent(modelId)}/position`,
      position
    ),

  autoArrange: (sessionId, positions) =>
    api.post(`/canvas/${encodeURIComponent(sessionId)}/arrange`, { positions }),

  deleteTile: (sessionId, tileId) =>
    api.delete(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}`),

  updateViewport: (sessionId, viewport) =>
    api.put(`/canvas/${encodeURIComponent(sessionId)}/viewport`, viewport),

  updateDebateStatus: (sessionId, debateId, status) =>
    api.put(`/canvas/${encodeURIComponent(sessionId)}/debate/${encodeURIComponent(debateId)}/status`, null, {
      params: { status },
    }),

  exportToNote: (sessionId) =>
    api.post(`/canvas/${encodeURIComponent(sessionId)}/export-note`),

  getNodeEdges: (sessionId) => api.get(`/canvas/${encodeURIComponent(sessionId)}/node-edges`),

  getNodeGroups: (sessionId) => api.get(`/canvas/${encodeURIComponent(sessionId)}/node-groups`),
}

// Utility function to check if we're in Tauri environment
export const isDesktopApp = isTauri

export default api
