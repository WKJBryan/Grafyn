<template>
  <div class="not-found-view">
    <div class="not-found-container">
      <h1 class="not-found-title">404</h1>
      <p class="not-found-message">Page not found</p>
      <p class="not-found-description">
        The page you're looking for doesn't exist or has been moved.
      </p>
      <button class="btn btn-primary" @click="goHome">
        Go Home
      </button>
    </div>
  </div>
</template>

<script setup>
import { useRouter } from 'vue-router'

const router = useRouter()

function goHome() {
  router.push('/')
}
</script>

<style scoped>
.not-found-view {
  width: 100%;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-primary);
}

.not-found-container {
  text-align: center;
  padding: var(--spacing-xl);
}

.not-found-title {
  font-size: 6rem;
  font-weight: 700;
  color: var(--accent-primary);
  margin: 0 0 var(--spacing-md) 0;
  line-height: 1;
}

.not-found-message {
  font-size: 1.5rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 var(--spacing-sm) 0;
}

.not-found-description {
  font-size: 1rem;
  color: var(--text-secondary);
  margin: 0 0 var(--spacing-xl) 0;
}
</style>
