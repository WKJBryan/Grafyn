import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import App from '@/App.vue'

const tauriPlugins = vi.hoisted(() => ({
  openExternal: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn(),
  unlisten: vi.fn(),
  toastWarning: vi.fn(),
  confirm: vi.fn(),
  check: vi.fn(),
  downloadAndInstall: vi.fn(),
  relaunch: vi.fn(),
  runtime: {
    isTauri: true,
    isDesktop: true,
    nativePlugins: true,
    compact: false,
    platform: 'windows',
  },
}))

vi.mock('@/api/client', () => ({
  isTauriApp: () => tauriPlugins.runtime.isTauri,
  isDesktopApp: () => tauriPlugins.runtime.isDesktop,
}))

vi.mock('@/api/transport', () => ({
  getRuntimeProfile: () => ({
    platform: tauriPlugins.runtime.platform,
    nativePlugins: tauriPlugins.runtime.nativePlugins,
  }),
  getTransport: () => ({
    listen: tauriPlugins.listen,
    openExternal: tauriPlugins.openExternal,
  }),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ warning: tauriPlugins.toastWarning }),
}))

vi.mock('@/composables/useCompanionLayout', () => ({
  useCompanionLayout: () => ({ isCompact: tauriPlugins.runtime.compact }),
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
        CompanionShell: {
          name: 'CompanionShell',
          template: '<div class="companion-shell-test"><slot /></div>',
        },
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
    tauriPlugins.runtime.nativePlugins = true
    tauriPlugins.runtime.compact = false
    tauriPlugins.runtime.platform = 'windows'
    tauriPlugins.check.mockResolvedValue(null)
    tauriPlugins.listen.mockResolvedValue(tauriPlugins.unlisten)
  })

  it('registers one committed-warning listener for the app lifetime', async () => {
    const wrapper = mountApp()
    await flushPromises()

    expect(tauriPlugins.listen).toHaveBeenCalledOnce()
    expect(tauriPlugins.listen).toHaveBeenCalledWith(
      'grafyn://committed-warning',
      expect.any(Function),
    )

    wrapper.unmount()
  })

  it('deduplicates the current warning DTO and distinguishes optional operations', async () => {
    const wrapper = mountApp()
    await flushPromises()
    const handleWarning = tauriPlugins.listen.mock.calls[0][1]
    const warning = {
      code: 'derived_state_unavailable',
      message: 'Your change was saved, but derived views are temporarily unavailable.',
    }

    handleWarning({ payload: warning })
    handleWarning({ payload: { ...warning } })
    handleWarning({ payload: { ...warning, message: 'different backend detail' } })
    handleWarning({ payload: { ...warning, operation: 'update_note' } })

    expect(tauriPlugins.toastWarning).toHaveBeenCalledTimes(2)
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      1,
      'Your change was saved, but derived views are temporarily unavailable.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      2,
      'Your change was saved, but derived views are temporarily unavailable.',
    )

    wrapper.unmount()
  })

  it('never displays text supplied by the warning payload', async () => {
    const wrapper = mountApp()
    await flushPromises()
    const handleWarning = tauriPlugins.listen.mock.calls[0][1]

    handleWarning({
      payload: {
        code: 'derived_state_unavailable',
        message: 'internal backend detail',
      },
    })

    expect(tauriPlugins.toastWarning).toHaveBeenCalledWith(
      'Your change was saved, but derived views are temporarily unavailable.',
    )
    expect(tauriPlugins.toastWarning).not.toHaveBeenCalledWith('internal backend detail')

    wrapper.unmount()
  })

  it('maps optimizer warning codes and unknown codes to safe local copy', async () => {
    const wrapper = mountApp()
    await flushPromises()
    const handleWarning = tauriPlugins.listen.mock.calls[0][1]

    handleWarning({
      payload: {
        code: 'optimizer_publication_pending',
        message: 'private publication detail',
      },
    })
    handleWarning({
      payload: {
        code: 'optimizer_rollback_recovery_pending',
        message: 'private recovery detail',
      },
    })
    handleWarning({
      payload: {
        code: 'optimizer_rollback_not_applied',
        message: 'private rollback detail',
      },
    })
    handleWarning({
      payload: {
        code: 'future_warning_code',
        message: 'private future detail',
      },
    })
    handleWarning({
      payload: {
        code: 'constructor',
        message: 'private prototype detail',
      },
    })
    handleWarning({
      payload: {
        code: '__proto__',
        message: 'private prototype detail',
      },
    })

    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      1,
      'Your change was saved, but its optimizer audit publication is still pending.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      2,
      'The rollback is accepted, but restoring the target bytes is still pending. Do not retry.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      3,
      'The rollback did not restore target bytes after authority advanced.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      4,
      'Your change was saved, but follow-up work is temporarily unavailable.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      5,
      'Your change was saved, but follow-up work is temporarily unavailable.',
    )
    expect(tauriPlugins.toastWarning).toHaveBeenNthCalledWith(
      6,
      'Your change was saved, but follow-up work is temporarily unavailable.',
    )
    expect(tauriPlugins.toastWarning).not.toHaveBeenCalledWith(
      expect.stringContaining('private'),
    )

    wrapper.unmount()
  })

  it('unlistens when the app is torn down', async () => {
    const wrapper = mountApp()
    await flushPromises()

    wrapper.unmount()

    expect(tauriPlugins.unlisten).toHaveBeenCalledOnce()
  })

  it('unlistens if teardown wins the asynchronous registration race', async () => {
    let resolveListen
    tauriPlugins.listen.mockReturnValue(new Promise((resolve) => {
      resolveListen = resolve
    }))
    const wrapper = mountApp()

    expect(tauriPlugins.listen).toHaveBeenCalledOnce()
    wrapper.unmount()
    resolveListen(tauriPlugins.unlisten)
    await flushPromises()

    expect(tauriPlugins.unlisten).toHaveBeenCalledOnce()
  })

  it('opens external links with the opener plugin', async () => {
    const wrapper = mountApp()
    const link = document.createElement('a')
    link.href = 'https://grafyn.app/docs'
    link.addEventListener('click', (event) => event.preventDefault())
    document.body.appendChild(link)

    link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    await flushPromises()

    expect(tauriPlugins.openExternal).toHaveBeenCalledWith({
      type: 'url',
      url: 'https://grafyn.app/docs',
    })
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

  it('does not intercept links or import updater plugins without native plugin authority', async () => {
    tauriPlugins.runtime.nativePlugins = false
    const wrapper = mountApp()
    const link = document.createElement('a')
    link.href = 'https://grafyn.app/docs'
    link.addEventListener('click', event => event.preventDefault())
    document.body.appendChild(link)

    link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    await flushPromises()

    expect(tauriPlugins.openExternal).not.toHaveBeenCalled()
    expect(tauriPlugins.check).not.toHaveBeenCalled()
    expect(tauriPlugins.listen).toHaveBeenCalledOnce()
    wrapper.unmount()
    link.remove()
  })

  it('wraps only compact routes in the companion shell and hides desktop guide chrome', () => {
    tauriPlugins.runtime.compact = true

    const wrapper = mountApp()

    expect(wrapper.find('.companion-shell-test').exists()).toBe(true)
    expect(wrapper.find('router-view-stub').exists()).toBe(true)
    expect(wrapper.find('guide-panel-stub').exists()).toBe(false)
    expect(wrapper.find('guide-tip-stub').exists()).toBe(false)
    wrapper.unmount()
  })

  it('leaves the wide router view outside the companion shell', () => {
    const wrapper = mountApp()

    expect(wrapper.find('.companion-shell-test').exists()).toBe(false)
    expect(wrapper.find('router-view-stub').exists()).toBe(true)
    expect(wrapper.find('guide-panel-stub').exists()).toBe(true)
    wrapper.unmount()
  })
})
