import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'

const shell = vi.hoisted(() => ({
  showMainWindow: vi.fn().mockResolvedValue(undefined),
  runtimeStatus: vi.fn().mockResolvedValue({ schemaVersion: 1, runtime: 'desktop' }),
  settings: vi.fn().mockResolvedValue({ theme: 'system' }),
}))

vi.mock('vue', () => ({
  createApp: () => ({ use: vi.fn().mockReturnThis(), mount: vi.fn() }),
}))
vi.mock('pinia', () => ({ createPinia: () => ({}) }))
vi.mock('@/App.vue', () => ({ default: {} }))
vi.mock('@/router', () => ({ default: {} }))
vi.mock('@/api/client', () => ({
  runtime: { getStatus: shell.runtimeStatus },
  settings: { get: shell.settings },
}))
vi.mock('@/api/transport', () => ({
  getRuntimeProfile: () => ({
    name: 'desktop-wide',
    isTauri: true,
    nativePlugins: false,
  }),
  getTransport: () => ({ showMainWindow: shell.showMainWindow }),
  setRuntimeStatus: vi.fn(),
}))
vi.mock('@/platform/capabilities', () => ({
  normalizeRuntimeStatus: status => status,
}))
vi.mock('@/stores/theme', () => ({
  resolveThemePreference: () => 'system',
  useThemeStore: () => ({ setTheme: vi.fn() }),
}))

describe('browser-backed desktop bootstrap', () => {
  beforeAll(async () => {
    vi.stubGlobal('requestAnimationFrame', callback => callback())
    await import('@/main.js')
  })

  afterAll(() => {
    vi.unstubAllGlobals()
  })

  it('does not show a native webview without native plugin authority', async () => {
    window.dispatchEvent(new Event('grafyn-app-mounted'))
    await Promise.resolve()

    expect(shell.showMainWindow).not.toHaveBeenCalled()
  })
})
