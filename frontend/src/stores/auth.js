import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { oauth as oauthApi } from '../api/client'

export const useAuthStore = defineStore('auth', () => {
  // State
  const user = ref(null)
  const token = ref(localStorage.getItem('auth_token') || null)
  const loading = ref(false)
  const error = ref(null)

  // Getters
  const isAuthenticated = computed(() => !!token.value)
  const userName = computed(() => user.value?.name || null)
  const userEmail = computed(() => user.value?.email || null)

  // Actions
  async function loginWithProvider(provider) {
    loading.value = true
    error.value = null
    try {
      const { authorization_url } = await oauthApi.getAuthorizationUrl(provider)
      // Redirect to OAuth provider
      window.location.href = authorization_url
    } catch (err) {
      error.value = err.message || 'Failed to initiate login'
      console.error('Login failed:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function handleOAuthCallback(provider, code) {
    loading.value = true
    error.value = null
    try {
      const { access_token } = await oauthApi.exchangeCode(provider, code)
      token.value = access_token
      localStorage.setItem('auth_token', access_token)
      await fetchUser()
      return true
    } catch (err) {
      error.value = err.message || 'Failed to complete login'
      console.error('OAuth callback failed:', err)
      logout()
      throw err
    } finally {
      loading.value = false
    }
  }

  async function fetchUser() {
    if (!token.value) return
    
    loading.value = true
    error.value = null
    try {
      const userData = await oauthApi.getUser()
      user.value = userData
    } catch (err) {
      error.value = err.message || 'Failed to fetch user'
      console.error('Failed to fetch user:', err)
      logout()
      throw err
    } finally {
      loading.value = false
    }
  }

  async function logout() {
    loading.value = true
    error.value = null
    try {
      await oauthApi.logout()
    } catch (err) {
      console.error('Logout API call failed:', err)
    } finally {
      // Clear local state regardless of API call result
      user.value = null
      token.value = null
      localStorage.removeItem('auth_token')
      loading.value = false
    }
  }

  function setToken(newToken) {
    token.value = newToken
    if (newToken) {
      localStorage.setItem('auth_token', newToken)
    } else {
      localStorage.removeItem('auth_token')
    }
  }

  function reset() {
    user.value = null
    token.value = null
    loading.value = false
    error.value = null
    localStorage.removeItem('auth_token')
  }

  return {
    // State
    user,
    token,
    loading,
    error,
    // Getters
    isAuthenticated,
    userName,
    userEmail,
    // Actions
    loginWithProvider,
    handleOAuthCallback,
    fetchUser,
    logout,
    setToken,
    reset,
  }
})
