import { createRouter, createWebHistory } from 'vue-router'

const routes = [
  {
    path: '/',
    name: 'home',
    component: () => import('../views/HomeView.vue'),
  },
  {
    path: '/canvas',
    name: 'canvas',
    component: () => import('../views/CanvasView.vue'),
  },
  {
    path: '/canvas/:id',
    name: 'canvas-session',
    component: () => import('../views/CanvasView.vue'),
  },
  {
    path: '/import',
    name: 'import',
    component: () => import('../views/ImportView.vue'),
  },
  {
    path: '/chat',
    name: 'chat',
    component: () => import('../views/ChatView.vue'),
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'not-found',
    component: () => import('../views/NotFoundView.vue'),
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

export default router
