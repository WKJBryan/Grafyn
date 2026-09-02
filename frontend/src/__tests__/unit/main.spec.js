import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'

const shell = vi.hoisted(() => ({
  showMainWindow: vi.fn().mockResolvedValue(undefined),
  mount: vi.fn(),
  use: vi.fn().mockReturnThis(),
  setTheme: vi.fn(),
  setRuntimeStatus: vi.fn(),
  runtimeGetStatus: vi.fn(),
  settingsGet: vi.fn(),
  order: [],
  installedStatus: null,
  mountedTarget: null,
}))

vi.mock('vue', () => ({
  createApp: () => {
    shell.order.push('create-app')
    return {
      use: shell.use,
      mount: (target) => {
        shell.mountedTarget = target
        return shell.mount(target)
      },
    }
  },
}))

vi.mock('pinia', () => ({
  createPinia: () => ({}),
}))

vi.mock('@/App.vue', () => ({ default: {} }))
vi.mock('@/router', () => ({ default: {} }))
vi.mock('@/api/client', () => ({
  runtime: { getStatus: shell.runtimeGetStatus },
  settings: { get: shell.settingsGet },
}))
vi.mock('@/api/transport', () => ({
  getRuntimeProfile: () => ({ name: 'desktop-wide', isTauri: true, nativePlugins: true }),
  getTransport: () => ({ showMainWindow: shell.showMainWindow }),
  setRuntimeStatus: shell.setRuntimeStatus,
}))
vi.mock('@/platform/capabilities', () => ({
  normalizeRuntimeStatus: (status) => {
    shell.order.push('normalize-runtime')
    return status
  },
}))
vi.mock('@/stores/theme', () => ({
  resolveThemePreference: () => 'system',
  useThemeStore: () => ({ setTheme: shell.setTheme }),
}))

describe('frontend bootstrap', () => {
  beforeAll(async () => {
    shell.runtimeGetStatus.mockImplementation(async () => {
      shell.order.push('runtime-status')
      return { schemaVersion: 1, runtime: 'desktop' }
    })
    shell.settingsGet.mockImplementation(async () => {
      shell.order.push('settings')
      return { theme: 'system' }
    })
    shell.setRuntimeStatus.mockImplementation((status) => {
      shell.installedStatus = status
      shell.order.push('install-runtime')
    })
    window.__TAURI__ = true
    vi.stubGlobal('requestAnimationFrame', (callback) => callback())
    await import('@/main.js')
  })

  afterAll(() => {
    vi.unstubAllGlobals()
  })

  it('shows the initially hidden current webview after the app mounts', async () => {
    window.dispatchEvent(new Event('grafyn-app-mounted'))
    await vi.waitFor(() => expect(shell.showMainWindow).toHaveBeenCalledOnce())
  })

  it('installs backend runtime health before loading settings or mounting the router', async () => {
    expect(shell.installedStatus).toEqual({ schemaVersion: 1, runtime: 'desktop' })
    expect(shell.order).toEqual([
      'runtime-status',
      'normalize-runtime',
      'install-runtime',
      'settings',
      'create-app',
    ])
    expect(shell.mountedTarget).toBe('#app')
  })
})
