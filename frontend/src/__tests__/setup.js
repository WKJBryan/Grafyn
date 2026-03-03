/**
 * Vitest setup file
 *
 * Configures:
 * - MSW (Mock Service Worker) for API mocking
 * - Component mount helpers
 * - Mock localStorage
 * - Global test utilities
 */

import { afterAll, afterEach, beforeAll, vi } from 'vitest'
import { config } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

// ============================================================================
// MSW Setup for API Mocking
// ============================================================================

// Mock localStorage
const localStorageMock = {
  getItem: vi.fn((key) => localStorageMock._storage[key] || null),
  setItem: vi.fn((key, value) => {
    localStorageMock._storage[key] = value
  }),
  removeItem: vi.fn((key) => {
    delete localStorageMock._storage[key]
  }),
  clear: vi.fn(() => {
    localStorageMock._storage = {}
  }),
  _storage: {},
}

global.localStorage = localStorageMock

// Mock window.matchMedia
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
})

// ============================================================================
// Vue Test Utils Configuration
// ============================================================================

// Set global config for all tests
config.global.stubs = {
  teleport: true,
  transition: false,
}

// ============================================================================
// Test Lifecycle Hooks
// ============================================================================

beforeAll(() => {
  // Setup before all tests
})

afterEach(() => {
  // Clear localStorage after each test
  localStorageMock.clear()

  // Clear all mocks
  vi.clearAllMocks()
})

afterAll(() => {
  // Cleanup after all tests
})

// ============================================================================
// Test Utilities
// ============================================================================

/**
 * Create a fresh Pinia instance for testing
 */
export function createTestingPinia(_options = {}) {
  const pinia = createPinia()
  setActivePinia(pinia)
  return pinia
}

/**
 * Wait for next tick and all promises
 */
export async function flushPromises() {
  return new Promise((resolve) => {
    setTimeout(resolve, 0)
  })
}

/**
 * Create mock router
 */
export function createMockRouter(options = {}) {
  const router = {
    push: vi.fn(),
    replace: vi.fn(),
    go: vi.fn(),
    back: vi.fn(),
    forward: vi.fn(),
    currentRoute: {
      value: {
        path: '/',
        params: {},
        query: {},
        ...options.currentRoute,
      },
    },
    ...options,
  }
  return router
}

/**
 * Create mock axios instance
 */
export function createMockAxios() {
  return {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: {
        use: vi.fn(),
        eject: vi.fn(),
      },
      response: {
        use: vi.fn(),
        eject: vi.fn(),
      },
    },
  }
}

// ============================================================================
// Mock Data
// ============================================================================

export const mockNotes = [
  {
    id: 'note-1',
    title: 'Test Note 1',
    content: 'Content of test note 1',
    status: 'draft',
    tags: ['test', 'sample'],
    created_at: '2025-01-01T10:00:00Z',
    updated_at: '2025-01-01T10:00:00Z',
    wikilinks: ['note-2'],
  },
  {
    id: 'note-2',
    title: 'Test Note 2',
    content: 'Content of test note 2 with [[note-1]] link',
    status: 'canonical',
    tags: ['test'],
    created_at: '2025-01-02T10:00:00Z',
    updated_at: '2025-01-02T10:00:00Z',
    wikilinks: ['note-1'],
  },
  {
    id: 'note-3',
    title: 'Test Note 3',
    content: 'Content of test note 3',
    status: 'evidence',
    tags: ['test', 'example'],
    created_at: '2025-01-03T10:00:00Z',
    updated_at: '2025-01-03T10:00:00Z',
    wikilinks: [],
  },
]

export const mockSearchResults = [
  {
    note_id: 'note-1',
    title: 'Test Note 1',
    content: 'Content of test note 1',
    score: 0.95,
  },
  {
    note_id: 'note-2',
    title: 'Test Note 2',
    content: 'Content of test note 2',
    score: 0.85,
  },
]

export const mockBacklinks = [
  {
    note_id: 'note-2',
    title: 'Test Note 2',
    context: 'Content with [[note-1]] link',
  },
]

export const mockUser = {
  id: 'user123',
  login: 'testuser',
  email: 'test@example.com',
  name: 'Test User',
}

export const mockAuthToken = 'mock_auth_token_12345'

// ============================================================================
// API Mock Handlers
// ============================================================================

/**
 * Create mock API responses
 */
export const mockApiHandlers = {
  // Notes API
  getNotes: () => Promise.resolve({ data: mockNotes }),
  getNote: (id) => {
    const note = mockNotes.find((n) => n.id === id)
    return note
      ? Promise.resolve({ data: note })
      : Promise.reject({ response: { status: 404 } })
  },
  createNote: (data) =>
    Promise.resolve({
      data: {
        id: 'new-note-id',
        ...data,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        wikilinks: [],
      },
    }),
  updateNote: (id, data) => {
    const note = mockNotes.find((n) => n.id === id)
    return note
      ? Promise.resolve({ data: { ...note, ...data } })
      : Promise.reject({ response: { status: 404 } })
  },
  deleteNote: (_id) => Promise.resolve({ data: null }),

  // Search API
  search: (_query) => Promise.resolve({ data: mockSearchResults }),

  // Graph API
  getBacklinks: (_id) => Promise.resolve({ data: mockBacklinks }),
  getOutgoing: (_id) => Promise.resolve({ data: [] }),

  // Auth API
  getAuthUrl: (_provider) =>
    Promise.resolve({
      data: {
        authorization_url: `https://github.com/login/oauth/authorize?client_id=test&state=test_state`,
      },
    }),
  exchangeCode: (_provider, _code) =>
    Promise.resolve({
      data: {
        access_token: mockAuthToken,
        user: mockUser,
      },
    }),
  getUser: () => Promise.resolve({ data: mockUser }),
  logout: () => Promise.resolve({ data: null }),
}

// ============================================================================
// Component Mount Helpers
// ============================================================================

/**
 * Mount helper with common dependencies
 */
export function mountWithDependencies(component, options = {}) {
  const { mount } = require('@vue/test-utils')

  const pinia = options.pinia || createTestingPinia()
  const router = options.router || createMockRouter()

  return mount(component, {
    global: {
      plugins: [pinia],
      mocks: {
        $router: router,
        $route: router.currentRoute.value,
      },
      stubs: {
        teleport: true,
        transition: false,
        ...options.stubs,
      },
    },
    ...options,
  })
}

/**
 * Shallow mount helper
 */
export function shallowMountWithDependencies(component, options = {}) {
  const { shallowMount } = require('@vue/test-utils')

  const pinia = options.pinia || createTestingPinia()
  const router = options.router || createMockRouter()

  return shallowMount(component, {
    global: {
      plugins: [pinia],
      mocks: {
        $router: router,
        $route: router.currentRoute.value,
      },
      stubs: {
        teleport: true,
        transition: false,
        ...options.stubs,
      },
    },
    ...options,
  })
}
