import { createMemoryHistory } from 'vue-router'
import { afterEach, describe, expect, it } from 'vitest'
import { createGrafynRouter } from '@/router'
import { setRuntimeStatus } from '@/api/transport'
import { CAPABILITY_NAMES, normalizeRuntimeStatus } from '@/platform/capabilities'
import { RUNTIME_PROFILES } from '@/platform/runtime'

function routerFor(name) {
  setRuntimeStatus(name === RUNTIME_PROFILES.DESKTOP_WIDE
    ? normalizeRuntimeStatus({
        schemaVersion: 1,
        runtime: 'desktop',
        capabilities: Object.fromEntries(CAPABILITY_NAMES.map(capability => [capability, true])),
        vault: { kind: 'user_selected', available: true },
        secureSecrets: { status: 'ready', code: null, message: null },
        nativeImageShare: {
          status: 'unavailable',
          code: 'desktop_save_as',
          message: 'Desktop uses Save As.',
        },
        diagnostics: [],
      })
    : null)
  return createGrafynRouter({
    history: createMemoryHistory(),
    getProfile: () => ({ name }),
  })
}

function revokeDesktopRuntime() {
  setRuntimeStatus(normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'desktop',
    capabilities: Object.fromEntries(CAPABILITY_NAMES.map(capability => [capability, false])),
    vault: { kind: 'user_selected', available: false },
    secureSecrets: {
      status: 'unavailable',
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    },
    nativeImageShare: {
      status: 'unavailable',
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    },
    diagnostics: [{
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    }],
  }))
}

describe('companion routes', () => {
  afterEach(() => setRuntimeStatus(null))

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

  it.each([
    ['/recall', 'recall'],
    ['/canvas', 'linearCanvas'],
    ['/canvas/session-a', 'linearCanvas'],
    ['/twin', 'twinReview'],
  ])(
    're-evaluates the active %s route when %s authority is revoked',
    async (path, capability) => {
      const router = routerFor(RUNTIME_PROFILES.DESKTOP_WIDE)
      await router.push(path)

      revokeDesktopRuntime()

      await expect.poll(() => router.currentRoute.value.name)
        .toBe('capability-unavailable')
      expect(router.currentRoute.value.query.capability).toBe(capability)
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
