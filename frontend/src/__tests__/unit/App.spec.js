import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import App from '@/App.vue'

const tauriPlugins = vi.hoisted(() => ({
  openUrl: vi.fn(),
  confirm: vi.fn(),
  check: vi.fn(),
  downloadAndInstall: vi.fn(),
  relaunch: vi.fn(),
  runtime: {
    isTauri: true,
    isDesktop: true,
    platform: 'windows',
  },
}))

vi.mock('@/api/client', () => ({
  isTauriApp: () => tauriPlugins.runtime.isTauri,
  isDesktopApp: () => tauriPlugins.runtime.isDesktop,
}))

vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: tauriPlugins.openUrl,
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: tauriPlugins.confirm,
}))

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: tauriPlugins.check,
}))

vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: tauriPlugins.relaunch,
}))

vi.mock('@tauri-apps/plugin-os', () => ({
  platform: () => tauriPlugins.runtime.platform,
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ path: '/' }),
}))

vi.mock('@/stores/boot', () => ({
  useBootStore: () => ({
    isVisible: false,
    status: null,
    initialize: vi.fn(),
    cleanup: vi.fn(),
    dismissSplash: vi.fn(),
  }),
}))

vi.mock('@/composables/useGuide', () => ({
  useGuide: () => ({
    setCurrentRoute: vi.fn(),
    checkNewFeatures: vi.fn(),
    showTipForRoute: vi.fn(),
  }),
}))

function mountApp() {
  return mount(App, {
    global: {
      stubs: {
        RouterView: true,
        ToastNotification: true,
        GuidePanel: true,
        GuideTip: true,
        StartupSplash: true,
      },
    },
  })
}

describe('desktop Tauri shell', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    tauriPlugins.runtime.isTauri = true
    tauriPlugins.runtime.isDesktop = true
    tauriPlugins.runtime.platform = 'windows'
    tauriPlugins.check.mockResolvedValue(null)
  })

  it('opens external links with the opener plugin', async () => {
    const wrapper = mountApp()
    const link = document.createElement('a')
    link.href = 'https://grafyn.app/docs'
    link.addEventListener('click', (event) => event.preventDefault())
    document.body.appendChild(link)

    link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    await flushPromises()

    expect(tauriPlugins.openUrl).toHaveBeenCalledWith('https://grafyn.app/docs')
    wrapper.unmount()
    link.remove()
  })

  it('offers and installs an available desktop update explicitly', async () => {
    tauriPlugins.check.mockResolvedValue({
      version: '0.4.0',
      body: 'Desktop improvements',
      downloadAndInstall: tauriPlugins.downloadAndInstall,
    })
    tauriPlugins.confirm.mockResolvedValue(true)

    const wrapper = mountApp()
    await flushPromises()

    expect(tauriPlugins.confirm).toHaveBeenCalledWith(
      expect.stringContaining('0.4.0'),
      expect.objectContaining({ title: 'Grafyn update' }),
    )
    expect(tauriPlugins.downloadAndInstall).toHaveBeenCalledOnce()
    expect(tauriPlugins.relaunch).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it.each(['macos', 'linux'])('relaunches %s after a successful update install', async (platform) => {
    tauriPlugins.runtime.platform = platform
    tauriPlugins.check.mockResolvedValue({
      version: '0.4.0',
      downloadAndInstall: tauriPlugins.downloadAndInstall,
    })
    tauriPlugins.confirm.mockResolvedValue(true)

    const wrapper = mountApp()
    await flushPromises()

    expect(tauriPlugins.downloadAndInstall).toHaveBeenCalledOnce()
    expect(tauriPlugins.relaunch).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('does not initialize the desktop updater on mobile', async () => {
    tauriPlugins.runtime.isDesktop = false

    const wrapper = mountApp()
    await flushPromises()

    expect(tauriPlugins.check).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
