/**
 * Unit tests for OAuthCallbackView
 *
 * Tests cover:
 * - Loading state display
 * - OAuth code extraction from URL
 * - Provider extraction from route params
 * - Successful callback handling
 * - Redirect after success
 * - Error state display
 * - Retry button functionality
 * - Missing parameters handling
 * - Error message display
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import OAuthCallbackView from '@/views/OAuthCallbackView.vue'
import { useAuthStore } from '@/stores/auth'

// Setup mock for vue-router
const mockRouterPush = vi.fn()
let mockRoute = {
  params: { provider: 'github' },
  query: { code: 'auth-code-123' },
}

vi.mock('vue-router', () => ({
  useRoute: vi.fn(() => mockRoute),
  useRouter: vi.fn(() => ({
    push: mockRouterPush,
  })),
}))

describe('OAuthCallbackView', () => {
  let wrapper
  let authStore

  beforeEach(() => {
    setActivePinia(createPinia())
    authStore = useAuthStore()
    vi.clearAllMocks()

    // Reset mock route
    mockRoute = {
      params: { provider: 'github' },
      query: { code: 'auth-code-123' },
    }
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
  })

  // ============================================================================
  // Rendering Tests
  // ============================================================================

  describe('Rendering', () => {
    it('renders the component', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockImplementation(
        () => new Promise(() => {}) // Never resolves
      )

      wrapper = mount(OAuthCallbackView)

      expect(wrapper.find('.oauth-callback-view').exists()).toBe(true)
    })

    it('renders callback container', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockImplementation(
        () => new Promise(() => {})
      )

      wrapper = mount(OAuthCallbackView)

      expect(wrapper.find('.callback-container').exists()).toBe(true)
    })
  })

  // ============================================================================
  // Loading State Tests
  // ============================================================================

  describe('Loading State', () => {
    it('shows loading state initially', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockImplementation(
        () => new Promise(() => {})
      )

      wrapper = mount(OAuthCallbackView)

      expect(wrapper.find('.spinner').exists()).toBe(true)
      expect(wrapper.text()).toContain('Completing sign in')
    })

    it('hides loading state after success', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.spinner').exists()).toBe(false)
    })

    it('hides loading state after error', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('Auth failed')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.spinner').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Success State Tests
  // ============================================================================

  describe('Success State', () => {
    it('shows success message after successful callback', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Successfully signed in')
    })

    it('shows redirecting message', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Redirecting')
    })

    it('redirects to home after success', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(mockRouterPush).toHaveBeenCalledWith('/')
    })

    it('redirects to custom URL if provided', async () => {
      mockRoute.query = { code: 'auth-code', redirect: '/notes' }
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(mockRouterPush).toHaveBeenCalledWith('/notes')
    })

    it('calls handleOAuthCallback with correct parameters', async () => {
      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith('github', 'auth-code-123')
    })
  })

  // ============================================================================
  // Error State Tests
  // ============================================================================

  describe('Error State', () => {
    it('shows error message on callback failure', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('Invalid code')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Authentication failed')
    })

    it('displays error message text', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('The authorization code has expired')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.error-text').text()).toBe(
        'The authorization code has expired'
      )
    })

    it('shows retry button on error', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('Failed')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      const retryBtn = wrapper.find('.btn-primary')
      expect(retryBtn.exists()).toBe(true)
      expect(retryBtn.text()).toBe('Try Again')
    })

    it('navigates to login on retry click', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('Failed')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      await wrapper.find('.btn-primary').trigger('click')

      expect(mockRouterPush).toHaveBeenCalledWith('/login')
    })

    it('handles error without message', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        'String error'
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Authentication failed')
    })
  })

  // ============================================================================
  // Missing Parameters Tests
  // ============================================================================

  describe('Missing Parameters', () => {
    it('shows error when provider is missing', async () => {
      mockRoute.params = {}
      mockRoute.query = { code: 'auth-code' }

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Invalid OAuth callback parameters')
    })

    it('shows error when code is missing', async () => {
      mockRoute.params = { provider: 'github' }
      mockRoute.query = {}

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Invalid OAuth callback parameters')
    })

    it('shows error when both are missing', async () => {
      mockRoute.params = {}
      mockRoute.query = {}

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.text()).toContain('Invalid OAuth callback parameters')
    })

    it('does not call handleOAuthCallback with missing params', async () => {
      mockRoute.params = {}
      mockRoute.query = {}

      const callbackSpy = vi.spyOn(authStore, 'handleOAuthCallback')

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).not.toHaveBeenCalled()
    })

    it('shows retry button with missing params', async () => {
      mockRoute.params = {}
      mockRoute.query = {}

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.btn-primary').exists()).toBe(true)
    })
  })

  // ============================================================================
  // Different Providers Tests
  // ============================================================================

  describe('Different Providers', () => {
    it('handles GitHub provider', async () => {
      mockRoute.params = { provider: 'github' }
      mockRoute.query = { code: 'github-code' }

      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith('github', 'github-code')
    })

    it('handles Google provider', async () => {
      mockRoute.params = { provider: 'google' }
      mockRoute.query = { code: 'google-code' }

      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith('google', 'google-code')
    })

    it('handles unknown provider', async () => {
      mockRoute.params = { provider: 'unknown' }
      mockRoute.query = { code: 'code' }

      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith('unknown', 'code')
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles code with special characters', async () => {
      mockRoute.query = { code: 'code+with/special=chars' }

      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith(
        'github',
        'code+with/special=chars'
      )
    })

    it('handles very long authorization code', async () => {
      const longCode = 'a'.repeat(500)
      mockRoute.query = { code: longCode }

      const callbackSpy = vi
        .spyOn(authStore, 'handleOAuthCallback')
        .mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(callbackSpy).toHaveBeenCalledWith('github', longCode)
    })

    it('handles redirect with query parameters', async () => {
      mockRoute.query = { code: 'code', redirect: '/notes?filter=draft' }

      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(mockRouterPush).toHaveBeenCalledWith('/notes?filter=draft')
    })

    it('applies error styling class', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockRejectedValue(
        new Error('Failed')
      )

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.callback-message.error').exists()).toBe(true)
    })

    it('applies success styling class', async () => {
      vi.spyOn(authStore, 'handleOAuthCallback').mockResolvedValue(true)

      wrapper = mount(OAuthCallbackView)
      await flushPromises()

      expect(wrapper.find('.callback-message.success').exists()).toBe(true)
    })
  })
})
