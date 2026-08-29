import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'

const shell = vi.hoisted(() => ({
  show: vi.fn().mockResolvedValue(undefined),
  mount: vi.fn(),
  use: vi.fn().mockReturnThis(),
  setTheme: vi.fn(),
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ show: shell.show }),
}))

vi.mock('vue', () => ({
  createApp: () => ({ use: shell.use, mount: shell.mount }),
}))

vi.mock('pinia', () => ({
  createPinia: () => ({}),
}))

vi.mock('@/App.vue', () => ({ default: {} }))
vi.mock('@/router', () => ({ default: {} }))
vi.mock('@/api/client', () => ({
  settings: { get: vi.fn().mockResolvedValue({ theme: 'system' }) },
}))
vi.mock('@/api/transport', () => ({
  getRuntimeProfile: () => ({ name: 'desktop-wide' }),
  getTransport: () => ({ show: shell.show }),
}))
vi.mock('@/stores/theme', () => ({
  resolveThemePreference: () => 'system',
  useThemeStore: () => ({ setTheme: shell.setTheme }),
}))

describe('frontend bootstrap', () => {
  beforeAll(async () => {
    window.__TAURI__ = true
    vi.stubGlobal('requestAnimationFrame', (callback) => callback())
    await import('@/main.js')
  })

  afterAll(() => {
    vi.unstubAllGlobals()
  })

  it('shows the initially hidden current webview after the app mounts', async () => {
    window.dispatchEvent(new Event('grafyn-app-mounted'))
    await vi.waitFor(() => expect(shell.show).toHaveBeenCalledOnce())
  })
})
