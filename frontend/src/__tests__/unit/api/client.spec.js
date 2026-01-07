/**
 * Unit tests for API client
 *
 * Tests cover:
 * - Axios instance configuration
 * - Request interceptor (adds auth token from localStorage)
 * - Response interceptor (extracts data, handles 401)
 * - Notes API methods (list, get, create, update, delete, reindex)
 * - Search API methods (query, similar)
 * - Graph API methods (backlinks, outgoing, neighbors, unlinkedMentions, rebuild)
 * - OAuth API methods (getAuthorizationUrl, exchangeCode, getUser, logout)
 * - URL encoding for note IDs with special characters
 * - Error handling
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import axios from 'axios'

// Mock axios before importing the client
vi.mock('axios', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: {
        use: vi.fn(),
      },
      response: {
        use: vi.fn(),
      },
    },
  }

  return {
    default: {
      create: vi.fn(() => mockAxiosInstance),
    },
  }
})

describe('API Client', () => {
  let mockAxiosInstance
  let requestInterceptor
  let responseSuccessInterceptor
  let responseErrorInterceptor
  let localStorageMock

  beforeEach(async () => {
    vi.clearAllMocks()

    // Setup localStorage mock
    localStorageMock = {
      getItem: vi.fn((key) => localStorageMock._storage[key] || null),
      setItem: vi.fn((key, value) => {
        localStorageMock._storage[key] = value
      }),
      removeItem: vi.fn((key) => {
        delete localStorageMock._storage[key]
      }),
      _storage: {},
    }
    global.localStorage = localStorageMock

    // Mock window.location
    delete window.location
    window.location = { href: '' }

    // Get the mock axios instance
    mockAxiosInstance = axios.create()

    // Capture interceptors
    mockAxiosInstance.interceptors.request.use.mockImplementation((fn) => {
      requestInterceptor = fn
    })
    mockAxiosInstance.interceptors.response.use.mockImplementation(
      (successFn, errorFn) => {
        responseSuccessInterceptor = successFn
        responseErrorInterceptor = errorFn
      }
    )

    // Re-import the module to trigger interceptor setup
    vi.resetModules()
    await import('@/api/client')
  })

  afterEach(() => {
    vi.resetModules()
  })

  // ============================================================================
  // Axios Instance Configuration Tests
  // ============================================================================

  describe('Axios Instance Configuration', () => {
    it('creates axios instance with correct base configuration', async () => {
      vi.resetModules()
      await import('@/api/client')

      expect(axios.create).toHaveBeenCalledWith({
        baseURL: '/api',
        headers: {
          'Content-Type': 'application/json',
        },
      })
    })
  })

  // ============================================================================
  // Request Interceptor Tests
  // ============================================================================

  describe('Request Interceptor', () => {
    it('adds Authorization header when token exists', async () => {
      localStorageMock._storage['auth_token'] = 'test-bearer-token'

      const config = { headers: {} }
      const result = requestInterceptor(config)

      expect(result.headers.Authorization).toBe('Bearer test-bearer-token')
    })

    it('does not add Authorization header when no token', async () => {
      const config = { headers: {} }
      const result = requestInterceptor(config)

      expect(result.headers.Authorization).toBeUndefined()
    })

    it('preserves existing config properties', async () => {
      localStorageMock._storage['auth_token'] = 'token'

      const config = {
        headers: { 'X-Custom': 'value' },
        params: { key: 'value' },
        data: { foo: 'bar' },
      }
      const result = requestInterceptor(config)

      expect(result.headers['X-Custom']).toBe('value')
      expect(result.params).toEqual({ key: 'value' })
      expect(result.data).toEqual({ foo: 'bar' })
    })
  })

  // ============================================================================
  // Response Interceptor Tests
  // ============================================================================

  describe('Response Interceptor', () => {
    it('extracts data from successful response', () => {
      const response = {
        data: { notes: [{ id: '1', title: 'Test' }] },
        status: 200,
      }

      const result = responseSuccessInterceptor(response)

      expect(result).toEqual({ notes: [{ id: '1', title: 'Test' }] })
    })

    it('handles 401 error by clearing token and redirecting', async () => {
      localStorageMock._storage['auth_token'] = 'invalid-token'

      const error = {
        response: { status: 401 },
      }

      await expect(responseErrorInterceptor(error)).rejects.toEqual(error)

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
      expect(window.location.href).toBe('/login')
    })

    it('passes through non-401 errors', async () => {
      const error = {
        response: { status: 500, data: { message: 'Server error' } },
      }

      await expect(responseErrorInterceptor(error)).rejects.toEqual(error)

      expect(localStorageMock.removeItem).not.toHaveBeenCalled()
      expect(window.location.href).toBe('')
    })

    it('handles network errors without response', async () => {
      const error = new Error('Network Error')

      await expect(responseErrorInterceptor(error)).rejects.toEqual(error)

      expect(localStorageMock.removeItem).not.toHaveBeenCalled()
    })
  })

  // ============================================================================
  // Notes API Tests
  // ============================================================================

  describe('Notes API', () => {
    let notes

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      notes = client.notes
    })

    it('list() calls GET /notes', async () => {
      mockAxiosInstance.get.mockResolvedValue([{ id: '1', title: 'Test' }])

      await notes.list()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/notes')
    })

    it('get() calls GET /notes/:id with encoded ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({ id: 'note-1', title: 'Test' })

      await notes.get('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/notes/note-1')
    })

    it('get() encodes special characters in ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('note with spaces')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/notes/note%20with%20spaces'
      )
    })

    it('create() calls POST /notes with data', async () => {
      const noteData = { title: 'New Note', content: 'Content' }
      mockAxiosInstance.post.mockResolvedValue({ id: 'new-id', ...noteData })

      await notes.create(noteData)

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/notes', noteData)
    })

    it('update() calls PUT /notes/:id with data', async () => {
      const updateData = { title: 'Updated Title' }
      mockAxiosInstance.put.mockResolvedValue({ id: 'note-1', ...updateData })

      await notes.update('note-1', updateData)

      expect(mockAxiosInstance.put).toHaveBeenCalledWith(
        '/notes/note-1',
        updateData
      )
    })

    it('update() encodes special characters in ID', async () => {
      mockAxiosInstance.put.mockResolvedValue({})

      await notes.update('special/note?id=1', { title: 'Test' })

      expect(mockAxiosInstance.put).toHaveBeenCalledWith(
        '/notes/special%2Fnote%3Fid%3D1',
        { title: 'Test' }
      )
    })

    it('delete() calls DELETE /notes/:id', async () => {
      mockAxiosInstance.delete.mockResolvedValue()

      await notes.delete('note-1')

      expect(mockAxiosInstance.delete).toHaveBeenCalledWith('/notes/note-1')
    })

    it('delete() encodes special characters in ID', async () => {
      mockAxiosInstance.delete.mockResolvedValue()

      await notes.delete('note#1')

      expect(mockAxiosInstance.delete).toHaveBeenCalledWith('/notes/note%231')
    })

    it('reindex() calls POST /notes/reindex', async () => {
      mockAxiosInstance.post.mockResolvedValue({ status: 'success' })

      await notes.reindex()

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/notes/reindex')
    })
  })

  // ============================================================================
  // Search API Tests
  // ============================================================================

  describe('Search API', () => {
    let search

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      search = client.search
    })

    it('query() calls GET /search with query params', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.query('test query')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/search', {
        params: { q: 'test query', limit: 10, semantic: true },
      })
    })

    it('query() uses custom limit', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.query('test', { limit: 5 })

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/search', {
        params: { q: 'test', limit: 5, semantic: true },
      })
    })

    it('query() allows disabling semantic search', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.query('test', { semantic: false })

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/search', {
        params: { q: 'test', limit: 10, semantic: false },
      })
    })

    it('similar() calls GET /search/similar/:id with limit', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.similar('note-1', 3)

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/search/similar/note-1',
        { params: { limit: 3 } }
      )
    })

    it('similar() uses default limit of 5', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.similar('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/search/similar/note-1',
        { params: { limit: 5 } }
      )
    })

    it('similar() encodes special characters in ID', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.similar('note with spaces')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/search/similar/note%20with%20spaces',
        { params: { limit: 5 } }
      )
    })
  })

  // ============================================================================
  // Graph API Tests
  // ============================================================================

  describe('Graph API', () => {
    let graph

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      graph = client.graph
    })

    it('backlinks() calls GET /graph/backlinks/:id', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await graph.backlinks('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/backlinks/note-1'
      )
    })

    it('backlinks() encodes special characters in ID', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await graph.backlinks('special&note')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/backlinks/special%26note'
      )
    })

    it('outgoing() calls GET /graph/outgoing/:id', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await graph.outgoing('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/outgoing/note-1'
      )
    })

    it('neighbors() calls GET /graph/neighbors/:id with depth', async () => {
      mockAxiosInstance.get.mockResolvedValue({ nodes: [], edges: [] })

      await graph.neighbors('note-1', 2)

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/neighbors/note-1',
        { params: { depth: 2 } }
      )
    })

    it('neighbors() uses default depth of 1', async () => {
      mockAxiosInstance.get.mockResolvedValue({ nodes: [], edges: [] })

      await graph.neighbors('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/neighbors/note-1',
        { params: { depth: 1 } }
      )
    })

    it('unlinkedMentions() calls GET /graph/unlinked-mentions/:id', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await graph.unlinkedMentions('note-1')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/graph/unlinked-mentions/note-1'
      )
    })

    it('rebuild() calls POST /graph/rebuild', async () => {
      mockAxiosInstance.post.mockResolvedValue({ status: 'success' })

      await graph.rebuild()

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/graph/rebuild')
    })
  })

  // ============================================================================
  // OAuth API Tests
  // ============================================================================

  describe('OAuth API', () => {
    let oauth

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      oauth = client.oauth
    })

    it('getAuthorizationUrl() calls GET /oauth/authorize/:provider', async () => {
      mockAxiosInstance.get.mockResolvedValue({
        authorization_url: 'https://github.com/...',
      })

      await oauth.getAuthorizationUrl('github')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/oauth/authorize/github'
      )
    })

    it('exchangeCode() calls POST /oauth/callback/:provider with code', async () => {
      mockAxiosInstance.post.mockResolvedValue({ access_token: 'token' })

      await oauth.exchangeCode('github', 'auth-code-123')

      expect(mockAxiosInstance.post).toHaveBeenCalledWith(
        '/oauth/callback/github',
        { code: 'auth-code-123' }
      )
    })

    it('getUser() calls GET /oauth/user', async () => {
      mockAxiosInstance.get.mockResolvedValue({ name: 'Test User' })

      await oauth.getUser()

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/oauth/user')
    })

    it('logout() calls POST /oauth/logout', async () => {
      mockAxiosInstance.post.mockResolvedValue({})

      await oauth.logout()

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/oauth/logout')
    })
  })

  // ============================================================================
  // URL Encoding Tests
  // ============================================================================

  describe('URL Encoding', () => {
    let notes, search, graph

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      notes = client.notes
      search = client.search
      graph = client.graph
    })

    it('encodes slashes in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('folder/note')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/notes/folder%2Fnote'
      )
    })

    it('encodes question marks in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('what?')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/notes/what%3F')
    })

    it('encodes hash in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('note#heading')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/notes/note%23heading'
      )
    })

    it('encodes ampersand in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('A&B')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/notes/A%26B')
    })

    it('encodes unicode characters in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('日本語')

      // encodeURIComponent encodes each UTF-8 byte
      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/notes/%E6%97%A5%E6%9C%AC%E8%AA%9E'
      )
    })

    it('encodes emojis in note ID', async () => {
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get('note🚀')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        '/notes/note%F0%9F%9A%80'
      )
    })
  })

  // ============================================================================
  // Error Handling Tests
  // ============================================================================

  describe('Error Handling', () => {
    let notes

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      notes = client.notes
    })

    it('propagates network errors', async () => {
      const networkError = new Error('Network Error')
      mockAxiosInstance.get.mockRejectedValue(networkError)

      await expect(notes.list()).rejects.toThrow('Network Error')
    })

    it('propagates server errors', async () => {
      const serverError = {
        response: { status: 500, data: { message: 'Internal Server Error' } },
      }
      mockAxiosInstance.get.mockRejectedValue(serverError)

      await expect(notes.list()).rejects.toEqual(serverError)
    })

    it('propagates validation errors', async () => {
      const validationError = {
        response: { status: 422, data: { detail: 'Title is required' } },
      }
      mockAxiosInstance.post.mockRejectedValue(validationError)

      await expect(notes.create({})).rejects.toEqual(validationError)
    })

    it('propagates 404 errors', async () => {
      const notFoundError = {
        response: { status: 404, data: { detail: 'Note not found' } },
      }
      mockAxiosInstance.get.mockRejectedValue(notFoundError)

      await expect(notes.get('nonexistent')).rejects.toEqual(notFoundError)
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    let notes, search

    beforeEach(async () => {
      vi.resetModules()
      const client = await import('@/api/client')
      notes = client.notes
      search = client.search
    })

    it('handles empty response data', async () => {
      mockAxiosInstance.get.mockResolvedValue(null)

      const result = await notes.list()

      expect(result).toBeNull()
    })

    it('handles empty array response', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      const result = await notes.list()

      expect(result).toEqual([])
    })

    it('handles very long note IDs', async () => {
      const longId = 'a'.repeat(500)
      mockAxiosInstance.get.mockResolvedValue({})

      await notes.get(longId)

      expect(mockAxiosInstance.get).toHaveBeenCalledWith(
        `/notes/${longId}`
      )
    })

    it('handles empty query string', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.query('')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/search', {
        params: { q: '', limit: 10, semantic: true },
      })
    })

    it('handles query with special characters', async () => {
      mockAxiosInstance.get.mockResolvedValue([])

      await search.query('[[wikilink]] & "quotes"')

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/search', {
        params: { q: '[[wikilink]] & "quotes"', limit: 10, semantic: true },
      })
    })

    it('returns undefined when delete succeeds', async () => {
      mockAxiosInstance.delete.mockResolvedValue(undefined)

      const result = await notes.delete('note-1')

      expect(result).toBeUndefined()
    })
  })
})
