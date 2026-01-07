/**
 * Unit tests for LoginView
 *
 * Tests cover:
 * - Component rendering
 * - Login provider buttons
 * - GitHub login flow
 * - Google login flow
 * - Error handling
 * - Alert on failure
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import LoginView from '@/views/LoginView.vue'
import { useAuthStore } from '@/stores/auth'
import * as apiClient from '@/api/client'

// Mock vue-router
vi.mock('vue-router', () => ({
  useRoute: vi.fn(() => ({
    query: {},
  })),
  useRouter: vi.fn(() => ({
    push: vi.fn(),
  })),
}))

describe('LoginView', () => {
  let wrapper
  let authStore

  beforeEach(() => {
    setActivePinia(createPinia())
    authStore = useAuthStore()
    vi.clearAllMocks()

    // Mock window.alert
    vi.spyOn(window, 'alert').mockImplementation(() => {})
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
    it('renders the component', () => {
      wrapper = mount(LoginView)

      expect(wrapper.find('.login-view').exists()).toBe(true)
    })

    it('renders login card', () => {
      wrapper = mount(LoginView)

      expect(wrapper.find('.login-card').exists()).toBe(true)
    })

    it('displays welcome title', () => {
      wrapper = mount(LoginView)

      expect(wrapper.find('.login-title').text()).toBe('Welcome to Seedream')
    })

    it('displays subtitle', () => {
      wrapper = mount(LoginView)

      expect(wrapper.find('.login-subtitle').text()).toBe(
        'Your personal knowledge management system'
      )
    })

    it('displays login note', () => {
      wrapper = mount(LoginView)

      expect(wrapper.find('.login-note').text()).toContain(
        'Sign in to access your notes'
      )
    })
  })

  // ============================================================================
  // Provider Buttons Tests
  // ============================================================================

  describe('Provider Buttons', () => {
    it('renders GitHub login button', () => {
      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))
      expect(githubBtn).toBeTruthy()
    })

    it('renders Google login button', () => {
      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const googleBtn = buttons.find((btn) => btn.text().includes('Google'))
      expect(googleBtn).toBeTruthy()
    })

    it('GitHub button has primary style', () => {
      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons[0]
      expect(githubBtn.classes()).toContain('btn-primary')
    })

    it('Google button has secondary style', () => {
      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const googleBtn = buttons[1]
      expect(googleBtn.classes()).toContain('btn-secondary')
    })

    it('buttons have large style', () => {
      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      buttons.forEach((btn) => {
        expect(btn.classes()).toContain('btn-large')
      })
    })
  })

  // ============================================================================
  // GitHub Login Tests
  // ============================================================================

  describe('GitHub Login', () => {
    it('calls authStore.loginWithProvider with github', async () => {
      const loginSpy = vi
        .spyOn(authStore, 'loginWithProvider')
        .mockResolvedValue()

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))
      await githubBtn.trigger('click')
      await flushPromises()

      expect(loginSpy).toHaveBeenCalledWith('github')
    })

    it('handles GitHub login error', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(authStore, 'loginWithProvider').mockRejectedValue(
        new Error('OAuth error')
      )

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))
      await githubBtn.trigger('click')
      await flushPromises()

      expect(consoleError).toHaveBeenCalledWith(
        'Login failed:',
        expect.any(Error)
      )
    })

    it('shows alert on GitHub login error', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(authStore, 'loginWithProvider').mockRejectedValue(
        new Error('OAuth error')
      )

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))
      await githubBtn.trigger('click')
      await flushPromises()

      expect(window.alert).toHaveBeenCalledWith(
        'Failed to initiate login. Please try again.'
      )
    })
  })

  // ============================================================================
  // Google Login Tests
  // ============================================================================

  describe('Google Login', () => {
    it('calls authStore.loginWithProvider with google', async () => {
      const loginSpy = vi
        .spyOn(authStore, 'loginWithProvider')
        .mockResolvedValue()

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const googleBtn = buttons.find((btn) => btn.text().includes('Google'))
      await googleBtn.trigger('click')
      await flushPromises()

      expect(loginSpy).toHaveBeenCalledWith('google')
    })

    it('handles Google login error', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(authStore, 'loginWithProvider').mockRejectedValue(
        new Error('OAuth error')
      )

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const googleBtn = buttons.find((btn) => btn.text().includes('Google'))
      await googleBtn.trigger('click')
      await flushPromises()

      expect(consoleError).toHaveBeenCalledWith(
        'Login failed:',
        expect.any(Error)
      )
    })

    it('shows alert on Google login error', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(authStore, 'loginWithProvider').mockRejectedValue(
        new Error('OAuth error')
      )

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const googleBtn = buttons.find((btn) => btn.text().includes('Google'))
      await googleBtn.trigger('click')
      await flushPromises()

      expect(window.alert).toHaveBeenCalledWith(
        'Failed to initiate login. Please try again.'
      )
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles multiple rapid login clicks', async () => {
      const loginSpy = vi
        .spyOn(authStore, 'loginWithProvider')
        .mockResolvedValue()

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))

      // Click multiple times rapidly
      await githubBtn.trigger('click')
      await githubBtn.trigger('click')
      await githubBtn.trigger('click')
      await flushPromises()

      expect(loginSpy).toHaveBeenCalledTimes(3)
    })

    it('handles login when store has existing state', async () => {
      authStore.token = 'existing-token'
      authStore.user = { name: 'Test User' }

      const loginSpy = vi
        .spyOn(authStore, 'loginWithProvider')
        .mockResolvedValue()

      wrapper = mount(LoginView)

      const buttons = wrapper.findAll('.login-providers .btn')
      const githubBtn = buttons.find((btn) => btn.text().includes('GitHub'))
      await githubBtn.trigger('click')
      await flushPromises()

      expect(loginSpy).toHaveBeenCalledWith('github')
    })
  })
})
