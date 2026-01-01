<template>
  <div class="oauth-callback-view">
    <div class="callback-container">
      <div v-if="loading" class="callback-message">
        <div class="spinner"></div>
        <p>Completing sign in...</p>
      </div>
      
      <div v-else-if="error" class="callback-message error">
        <p>Authentication failed</p>
        <p class="error-text">{{ error }}</p>
        <button class="btn btn-primary" @click="handleRetry">
          Try Again
        </button>
      </div>
      
      <div v-else class="callback-message success">
        <p>Successfully signed in!</p>
        <p>Redirecting...</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const loading = ref(true)
const error = ref(null)

onMounted(async () => {
  const provider = route.params.provider
  const code = route.query.code
  
  if (!provider || !code) {
    error.value = 'Invalid OAuth callback parameters'
    loading.value = false
    return
  }
  
  try {
    await authStore.handleOAuthCallback(provider, code)
    
    // Redirect to the page the user was trying to access, or home
    const redirect = route.query.redirect || '/'
    router.push(redirect)
  } catch (err) {
    error.value = err.message || 'Authentication failed'
    loading.value = false
  }
})

function handleRetry() {
  router.push('/login')
}
</script>

<style scoped>
.oauth-callback-view {
  width: 100%;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-primary);
}

.callback-container {
  width: 100%;
  max-width: 400px;
  padding: var(--spacing-lg);
}

.callback-message {
  background: var(--bg-secondary);
  border-radius: var(--radius-lg);
  padding: var(--spacing-xl);
  text-align: center;
}

.callback-message p {
  margin-bottom: var(--spacing-md);
  font-size: 1rem;
  color: var(--text-primary);
}

.callback-message.error p:first-child {
  color: var(--accent-danger);
  font-weight: 600;
}

.error-text {
  color: var(--text-secondary);
  font-size: 0.875rem;
}

.callback-message.success p:first-child {
  color: var(--accent-success);
  font-weight: 600;
}

.spinner {
  width: 40px;
  height: 40px;
  margin: 0 auto var(--spacing-md);
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
