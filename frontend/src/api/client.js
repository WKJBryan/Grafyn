import axios from 'axios'

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

// Notes API
export const notes = {
  list: () => api.get('/notes'),
  get: (id) => api.get(`/notes/${encodeURIComponent(id)}`),
  create: (data) => api.post('/notes', data),
  update: (id, data) => api.put(`/notes/${encodeURIComponent(id)}`, data),
  delete: (id) => api.delete(`/notes/${encodeURIComponent(id)}`),
  reindex: () => api.post('/notes/reindex'),
}

// Search API
export const search = {
  query: (q, { limit = 10, semantic = true } = {}) =>
    api.get('/search', { params: { q, limit, semantic } }),
  similar: (noteId, limit = 5) =>
    api.get(`/search/similar/${encodeURIComponent(noteId)}`, { params: { limit } }),
}

// Graph API
export const graph = {
  backlinks: (id) => api.get(`/graph/backlinks/${encodeURIComponent(id)}`),
  outgoing: (id) => api.get(`/graph/outgoing/${encodeURIComponent(id)}`),
  neighbors: (id, depth = 1) =>
    api.get(`/graph/neighbors/${encodeURIComponent(id)}`, { params: { depth } }),
  unlinkedMentions: (id) => api.get(`/graph/unlinked-mentions/${encodeURIComponent(id)}`),
  rebuild: () => api.post('/graph/rebuild'),
  full: () => api.get('/graph/full'),
}

// OAuth API
export const oauth = {
  getAuthorizationUrl: (provider) =>
    api.get(`/oauth/authorize/${provider}`),
  exchangeCode: (provider, code) =>
    api.post(`/oauth/callback/${provider}`, { code }),
  getUser: () => api.get('/oauth/user'),
  logout: () => api.post('/oauth/logout'),
}

// Canvas API
export const canvas = {
  list: () => api.get('/canvas'),
  get: (id) => api.get(`/canvas/${encodeURIComponent(id)}`),
  create: (data) => api.post('/canvas', data),
  update: (id, data) => api.put(`/canvas/${encodeURIComponent(id)}`, data),
  delete: (id) => api.delete(`/canvas/${encodeURIComponent(id)}`),
  getModels: () => api.get('/canvas/models/available'),
  updateTilePosition: (sessionId, tileId, position) =>
    api.put(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/position`, position),
  deleteTile: (sessionId, tileId) =>
    api.delete(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}`),
  updateViewport: (sessionId, viewport) =>
    api.put(`/canvas/${encodeURIComponent(sessionId)}/viewport`, viewport),
  updateDebateStatus: (sessionId, debateId, status) =>
    api.put(`/canvas/${encodeURIComponent(sessionId)}/debate/${encodeURIComponent(debateId)}/status`, null, { params: { status } }),
  exportToNote: (sessionId) =>
    api.post(`/canvas/${encodeURIComponent(sessionId)}/export-note`),
}

export default api
