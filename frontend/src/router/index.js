import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const routes = [
  {
    path: '/',
    name: 'home',
    component: () => import('../views/HomeView.vue'),
    meta: { requiresAuth: false }
  },
  {
    path: '/login',
    name: 'login',
    component: () => import('../views/LoginView.vue'),
    meta: { requiresAuth: false }
  },
  {
    path: '/oauth/callback/:provider',
    name: 'oauth-callback',
    component: () => import('../views/OAuthCallbackView.vue'),
    meta: { requiresAuth: false }
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'not-found',
    component: () => import('../views/NotFoundView.vue'),
    meta: { requiresAuth: false }
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

// Navigation guard for authentication
router.beforeEach(async (to, from, next) => {
  const authStore = useAuthStore()
  
  // If route doesn't require auth, proceed
  if (!to.meta.requiresAuth) {
    next()
    return
  }
  
  // Check if user is authenticated
  if (authStore.isAuthenticated) {
    // If we have a token but no user data, fetch it
    if (!authStore.user) {
      try {
        await authStore.fetchUser()
      } catch (error) {
        // Token might be invalid, redirect to login
        next({ name: 'login', query: { redirect: to.fullPath } })
        return
      }
    }
    next()
  } else {
    // Not authenticated, redirect to login
    next({ name: 'login', query: { redirect: to.fullPath } })
  }
})

export default router
