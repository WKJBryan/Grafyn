import { createRouter, createWebHistory } from 'vue-router'
import { getRuntimeProfile } from '@/api/transport'
import { hasCapability } from '@/platform/capabilities'

const routes = [
  {
    path: '/',
    name: 'home',
    component: () => import('../views/ResponsiveHomeView.vue'),
  },
  {
    path: '/recall',
    name: 'recall',
    component: () => import('../views/companion/RecallView.vue'),
    meta: { capability: 'recall' },
  },
  {
    path: '/canvas',
    name: 'canvas',
    component: () => import('../views/CanvasView.vue'),
    meta: { capability: 'spatialCanvas' },
  },
  {
    path: '/canvas/:id',
    name: 'canvas-session',
    component: () => import('../views/CanvasView.vue'),
    meta: { capability: 'spatialCanvas' },
  },
  {
    path: '/import',
    name: 'import',
    component: () => import('../views/ImportView.vue'),
    meta: { capability: 'importByPath' },
  },
  {
    path: '/twin',
    name: 'twin-review',
    component: () => import('../views/TwinReviewView.vue'),
    meta: { capability: 'twinReview' },
  },
  {
    path: '/unavailable',
    name: 'capability-unavailable',
    component: () => import('../components/companion/CapabilityUnavailable.vue'),
    props: route => ({ capability: route.query.capability || '' }),
  },
  {
    path: '/:pathMatch(.*)*',
    name: 'not-found',
    component: () => import('../views/NotFoundView.vue'),
  },
]

export function createGrafynRouter({
  history = createWebHistory(),
  getProfile = getRuntimeProfile,
} = {}) {
  const router = createRouter({ history, routes })

  router.beforeEach(to => {
    const capability = to.meta.capability
    if (!capability || hasCapability(getProfile(), capability)) return true
    return {
      name: 'capability-unavailable',
      query: { capability },
    }
  })

  return router
}

const router = createGrafynRouter()
export default router
