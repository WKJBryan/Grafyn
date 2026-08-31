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
})
