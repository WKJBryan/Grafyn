/**
 * Unit tests for auth Pinia store
 *
 * Tests cover:
 * - Initial state with localStorage token
 * - Computed properties (isAuthenticated, userName, userEmail)
 * - loginWithProvider() with window redirect
 * - handleOAuthCallback() with token storage and user fetch
 * - fetchUser() action
 * - logout() with cleanup
 * - setToken() with localStorage sync
 * - reset() action
 * - localStorage synchronization
 * - Error handling with logout calls
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useAuthStore } from '@/stores/auth'
import * as apiClient from '@/api/client'

describe('Auth Store', () => {
  let localStorageMock

  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()

    // Mock localStorage
    localStorageMock = {
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
  })

  afterEach(() => {
    localStorageMock.clear()
  })

  // ============================================================================
  // Initial State Tests
  // ============================================================================

  describe('Initial State', () => {
    it('has null user', () => {
      const store = useAuthStore()

      expect(store.user).toBeNull()
    })

    it('loads token from localStorage', () => {
      localStorageMock._storage['auth_token'] = 'stored-token'

      const store = useAuthStore()

      expect(store.token).toBe('stored-token')
    })

    it('has null token when localStorage is empty', () => {
      const store = useAuthStore()

      expect(store.token).toBeNull()
    })

    it('has loading set to false', () => {
      const store = useAuthStore()

      expect(store.loading).toBe(false)
    })

    it('has null error', () => {
      const store = useAuthStore()

      expect(store.error).toBeNull()
    })
  })

  // ============================================================================
  // Computed Properties Tests
  // ============================================================================

  describe('Computed Properties', () => {
    it('isAuthenticated is false when no token', () => {
      const store = useAuthStore()

      expect(store.isAuthenticated).toBe(false)
    })

    it('isAuthenticated is true when token exists', () => {
      localStorageMock._storage['auth_token'] = 'test-token'
      const store = useAuthStore()

      expect(store.isAuthenticated).toBe(true)
    })

    it('userName returns user name when user exists', () => {
      const store = useAuthStore()
      store.user = { name: 'John Doe', email: 'john@example.com' }

      expect(store.userName).toBe('John Doe')
    })

    it('userName returns null when no user', () => {
      const store = useAuthStore()

      expect(store.userName).toBeNull()
    })

    it('userEmail returns user email when user exists', () => {
      const store = useAuthStore()
      store.user = { name: 'John Doe', email: 'john@example.com' }

      expect(store.userEmail).toBe('john@example.com')
    })

    it('userEmail returns null when no user', () => {
      const store = useAuthStore()

      expect(store.userEmail).toBeNull()
    })
  })

  // ============================================================================
  // loginWithProvider() Tests
  // ============================================================================

  describe('loginWithProvider()', () => {
    it('fetches authorization URL from API', async () => {
      const mockAuthUrl = 'https://github.com/login/oauth/authorize?...'
      vi.spyOn(apiClient.oauth, 'getAuthorizationUrl').mockResolvedValue({
        authorization_url: mockAuthUrl,
      })

      // Mock window.location.href
      delete window.location
      window.location = { href: '' }

      const store = useAuthStore()
      await store.loginWithProvider('github')

      expect(apiClient.oauth.getAuthorizationUrl).toHaveBeenCalledWith('github')
    })

    it('redirects to authorization URL', async () => {
      const mockAuthUrl = 'https://github.com/login/oauth/authorize?...'
      vi.spyOn(apiClient.oauth, 'getAuthorizationUrl').mockResolvedValue({
        authorization_url: mockAuthUrl,
      })

      // Mock window.location.href
      delete window.location
      window.location = { href: '' }

      const store = useAuthStore()
      await store.loginWithProvider('github')

      expect(window.location.href).toBe(mockAuthUrl)
    })

    it('sets loading state during request', async () => {
      vi.spyOn(apiClient.oauth, 'getAuthorizationUrl').mockImplementation(
        () => new Promise(() => {}) // Never resolves
      )

      delete window.location
      window.location = { href: '' }

      const store = useAuthStore()
      const promise = store.loginWithProvider('github')

      expect(store.loading).toBe(true)

      await promise.catch(() => {})
    })

    it('handles errors correctly', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'getAuthorizationUrl').mockRejectedValue(
        new Error('Network error')
      )

      delete window.location
      window.location = { href: '' }

      const store = useAuthStore()

      await expect(store.loginWithProvider('github')).rejects.toThrow()

      expect(store.error).toBe('Network error')
    })
  })

  // ============================================================================
  // handleOAuthCallback() Tests
  // ============================================================================

  describe('handleOAuthCallback()', () => {
    it('exchanges code for access token', async () => {
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockResolvedValue({
        access_token: 'new-access-token',
      })
      vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue({
        name: 'John Doe',
      })

      const store = useAuthStore()
      await store.handleOAuthCallback('github', 'auth-code-123')

      expect(apiClient.oauth.exchangeCode).toHaveBeenCalledWith(
        'github',
        'auth-code-123'
      )
    })

    it('sets token in state and localStorage', async () => {
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockResolvedValue({
        access_token: 'new-access-token',
      })
      vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue({
        name: 'John Doe',
      })

      const store = useAuthStore()
      await store.handleOAuthCallback('github', 'auth-code-123')

      expect(store.token).toBe('new-access-token')
      expect(localStorageMock.setItem).toHaveBeenCalledWith(
        'auth_token',
        'new-access-token'
      )
    })

    it('fetches user after token exchange', async () => {
      const getUserSpy = vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue({
        name: 'John Doe',
        email: 'john@example.com',
      })
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockResolvedValue({
        access_token: 'new-token',
      })

      const store = useAuthStore()
      await store.handleOAuthCallback('github', 'auth-code-123')

      expect(getUserSpy).toHaveBeenCalledTimes(1)
      expect(store.user).toEqual({
        name: 'John Doe',
        email: 'john@example.com',
      })
    })

    it('returns true on success', async () => {
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockResolvedValue({
        access_token: 'token',
      })
      vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue({
        name: 'Test User',
      })

      const store = useAuthStore()
      const result = await store.handleOAuthCallback('github', 'code')

      expect(result).toBe(true)
    })

    it('calls logout on error', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockRejectedValue(
        new Error('Invalid code')
      )
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()

      await expect(
        store.handleOAuthCallback('github', 'invalid-code')
      ).rejects.toThrow()

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })

    it('sets error on failure', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'exchangeCode').mockRejectedValue(
        new Error('Exchange failed')
      )
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()

      await expect(
        store.handleOAuthCallback('github', 'code')
      ).rejects.toThrow()

      expect(store.error).toBe('Exchange failed')
    })
  })

  // ============================================================================
  // fetchUser() Tests
  // ============================================================================

  describe('fetchUser()', () => {
    it('fetches user data when token exists', async () => {
      const mockUser = { name: 'Jane Doe', email: 'jane@example.com' }
      vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue(mockUser)

      const store = useAuthStore()
      store.token = 'valid-token'

      await store.fetchUser()

      expect(store.user).toEqual(mockUser)
    })

    it('does not fetch when no token', async () => {
      const getUserSpy = vi.spyOn(apiClient.oauth, 'getUser')

      const store = useAuthStore()
      await store.fetchUser()

      expect(getUserSpy).not.toHaveBeenCalled()
    })

    it('calls logout on error', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'getUser').mockRejectedValue(
        new Error('Unauthorized')
      )
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.token = 'invalid-token'

      await expect(store.fetchUser()).rejects.toThrow()

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })

    it('sets error on failure', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'getUser').mockRejectedValue(
        new Error('Fetch failed')
      )
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.token = 'token'

      await expect(store.fetchUser()).rejects.toThrow()

      expect(store.error).toBe('Fetch failed')
    })
  })

  // ============================================================================
  // logout() Tests
  // ============================================================================

  describe('logout()', () => {
    it('calls logout API', async () => {
      const logoutSpy = vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.token = 'token'

      await store.logout()

      expect(logoutSpy).toHaveBeenCalledTimes(1)
    })

    it('clears user state', async () => {
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.user = { name: 'Test User' }
      store.token = 'token'

      await store.logout()

      expect(store.user).toBeNull()
      expect(store.token).toBeNull()
    })

    it('removes token from localStorage', async () => {
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.token = 'token'
      localStorageMock._storage['auth_token'] = 'token'

      await store.logout()

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })

    it('clears state even if API call fails', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'logout').mockRejectedValue(
        new Error('API error')
      )

      const store = useAuthStore()
      store.user = { name: 'Test User' }
      store.token = 'token'

      await store.logout()

      expect(store.user).toBeNull()
      expect(store.token).toBeNull()
      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })

    it('sets loading to false after completion', async () => {
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()

      await store.logout()

      expect(store.loading).toBe(false)
    })
  })

  // ============================================================================
  // setToken() Tests
  // ============================================================================

  describe('setToken()', () => {
    it('sets token in state', () => {
      const store = useAuthStore()

      store.setToken('new-token')

      expect(store.token).toBe('new-token')
    })

    it('saves token to localStorage', () => {
      const store = useAuthStore()

      store.setToken('new-token')

      expect(localStorageMock.setItem).toHaveBeenCalledWith(
        'auth_token',
        'new-token'
      )
    })

    it('removes token from localStorage when null', () => {
      const store = useAuthStore()
      store.token = 'existing-token'

      store.setToken(null)

      expect(store.token).toBeNull()
      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })

    it('removes token from localStorage when empty string', () => {
      const store = useAuthStore()

      store.setToken('')

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })
  })

  // ============================================================================
  // reset() Tests
  // ============================================================================

  describe('reset()', () => {
    it('resets all state to initial values', () => {
      const store = useAuthStore()
      store.user = { name: 'Test User' }
      store.token = 'token'
      store.loading = true
      store.error = 'Some error'

      store.reset()

      expect(store.user).toBeNull()
      expect(store.token).toBeNull()
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })

    it('removes token from localStorage', () => {
      const store = useAuthStore()
      localStorageMock._storage['auth_token'] = 'token'

      store.reset()

      expect(localStorageMock.removeItem).toHaveBeenCalledWith('auth_token')
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles error without message property', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.oauth, 'getAuthorizationUrl').mockRejectedValue(
        'String error'
      )

      delete window.location
      window.location = { href: '' }

      const store = useAuthStore()

      await expect(store.loginWithProvider('github')).rejects.toBe(
        'String error'
      )

      expect(store.error).toBe('Failed to initiate login')
    })

    it('isAuthenticated updates when token changes', () => {
      const store = useAuthStore()

      expect(store.isAuthenticated).toBe(false)

      store.token = 'new-token'

      expect(store.isAuthenticated).toBe(true)

      store.token = null

      expect(store.isAuthenticated).toBe(false)
    })

    it('handles user object without name', () => {
      const store = useAuthStore()
      store.user = { email: 'test@example.com' }

      expect(store.userName).toBeNull()
    })

    it('handles user object without email', () => {
      const store = useAuthStore()
      store.user = { name: 'Test User' }

      expect(store.userEmail).toBeNull()
    })

    it('handles concurrent operations', async () => {
      vi.spyOn(apiClient.oauth, 'getUser').mockResolvedValue({
        name: 'User',
      })
      vi.spyOn(apiClient.oauth, 'logout').mockResolvedValue()

      const store = useAuthStore()
      store.token = 'token'

      await Promise.all([store.fetchUser(), store.logout()])

      // Logout should win - state should be cleared
      expect(store.user).toBeNull()
      expect(store.token).toBeNull()
    })
  })
})
