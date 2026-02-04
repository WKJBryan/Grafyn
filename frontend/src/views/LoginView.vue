<template>
  <div class="login-view">
    <div class="login-container">
      <div class="login-card">
        <h1 class="login-title">
          Welcome to Seedream
        </h1>
        <p class="login-subtitle">
          Your personal knowledge management system
        </p>
        
        <div class="login-providers">
          <button
            class="btn btn-primary btn-large"
            @click="handleLogin('github')"
          >
            <span class="provider-icon">GitHub</span>
            Continue with GitHub
          </button>
          
          <button
            class="btn btn-secondary btn-large"
            @click="handleLogin('google')"
          >
            <span class="provider-icon">Google</span>
            Continue with Google
          </button>
        </div>
        
        <p class="login-note">
          Sign in to access your notes and manage your knowledge base
        </p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

async function handleLogin(provider) {
  try {
    await authStore.loginWithProvider(provider)
  } catch (error) {
    console.error('Login failed:', error)
    alert('Failed to initiate login. Please try again.')
  }
}
</script>

<style scoped>
.login-view {
  width: 100%;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-primary);
}

.login-container {
  width: 100%;
  max-width: 400px;
  padding: var(--spacing-lg);
}

.login-card {
  background: var(--bg-secondary);
  border-radius: var(--radius-lg);
  padding: var(--spacing-xl);
  text-align: center;
}

.login-title {
  font-size: 1.75rem;
  font-weight: 700;
  color: var(--text-primary);
  margin-bottom: var(--spacing-sm);
}

.login-subtitle {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin-bottom: var(--spacing-xl);
}

.login-providers {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-lg);
}

.btn-large {
  width: 100%;
  padding: var(--spacing-md);
  font-size: 1rem;
  justify-content: center;
}

.provider-icon {
  font-weight: 600;
}

.login-note {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin: 0;
}
</style>
