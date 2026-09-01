import { createMemoryHistory } from 'vue-router'
import { describe, expect, it } from 'vitest'
import { createGrafynRouter } from '@/router'
import { RUNTIME_PROFILES } from '@/platform/runtime'

function routerFor(name) {
  return createGrafynRouter({
    history: createMemoryHistory(),
    getProfile: () => ({ name }),
  })
}

describe('companion routes', () => {
  it('registers responsive Capture and Recall destinations', () => {
    const router = routerFor(RUNTIME_PROFILES.DESKTOP_WIDE)
    expect(router.resolve('/').name).toBe('home')
    expect(router.resolve('/recall').name).toBe('recall')
  })

  it('routes Twin and both Canvas paths through responsive view boundaries', async () => {
    const router = routerFor(RUNTIME_PROFILES.DESKTOP_WIDE)
    const twinRoute = router.getRoutes().find(route => route.name === 'twin-review')
    const canvasRoute = router.getRoutes().find(route => route.name === 'canvas')
    const sessionRoute = router.getRoutes().find(route => route.name === 'canvas-session')

    expect(twinRoute.meta.capability).toBe('twinReview')
    expect(canvasRoute.meta.capability).toBe('linearCanvas')
    expect(sessionRoute.meta.capability).toBe('linearCanvas')

    const [twinModule, canvasModule, sessionModule] = await Promise.all([
      twinRoute.components.default(),
      canvasRoute.components.default(),
      sessionRoute.components.default(),
    ])
    expect(twinModule.default.__file).toMatch(/ResponsiveTwinView\.vue$/)
    expect(canvasModule.default.__file).toMatch(/ResponsiveCanvasView\.vue$/)
    expect(sessionModule.default.__file).toMatch(/ResponsiveCanvasView\.vue$/)
  })

  it.each(['/canvas', '/canvas/session-a'])(
    'allows linear Canvas on a desktop runtime at %s',
    async (path) => {
      const router = routerFor(RUNTIME_PROFILES.DESKTOP_WIDE)
      await router.push(path)

      expect(router.currentRoute.value.path).toBe(path)
      expect(router.currentRoute.value.meta.capability).toBe('linearCanvas')
    },
  )

  it('allows path import on desktop', async () => {
    const router = routerFor(RUNTIME_PROFILES.DESKTOP_WIDE)
    await router.push('/import')
    expect(router.currentRoute.value.name).toBe('import')
  })

  it.each(['/import', '/recall', '/twin', '/canvas'])(
    'fails closed for unavailable Android route %s',
    async (path) => {
      const router = routerFor(RUNTIME_PROFILES.ANDROID_COMPACT)
      await router.push(path)
      expect(router.currentRoute.value.name).toBe('capability-unavailable')
      expect(router.currentRoute.value.query.capability).toBeTruthy()
    },
  )

  it.each(['/canvas', '/canvas/session-a'])(
    'identifies linear Canvas exactly when Android blocks %s',
    async (path) => {
      const router = routerFor(RUNTIME_PROFILES.ANDROID_COMPACT)
      await router.push(path)

      expect(router.currentRoute.value.name).toBe('capability-unavailable')
      expect(router.currentRoute.value.query.capability).toBe('linearCanvas')
    },
  )
})
