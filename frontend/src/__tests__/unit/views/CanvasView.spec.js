import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CanvasView from '@/views/CanvasView.vue'

const { push, loadSessions, loadSession, loadModels, updateSettings, setTheme } = vi.hoisted(() => ({
  push: vi.fn(),
  loadSessions: vi.fn().mockResolvedValue(),
  loadSession: vi.fn().mockResolvedValue(),
  loadModels: vi.fn().mockResolvedValue(),
  updateSettings: vi.fn().mockResolvedValue({}),
  setTheme: vi.fn()
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({
    params: { id: 'session-1' }
  }),
  useRouter: () => ({
    push
  }),
  RouterLink: {
    props: ['to'],
    template: '<a><slot /></a>'
  }
}))

vi.mock('@/stores/canvas', () => ({
  useCanvasStore: () => ({
    sessions: [],
    loading: false,
    loadSessions,
    loadSession,
    loadModels,
    createSession: vi.fn(),
    deleteSession: vi.fn()
  })
}))

vi.mock('@/stores/theme', () => ({
  useThemeStore: () => ({
    theme: 'dark',
    setTheme
  })
}))

vi.mock('@/api/client', () => ({
  isDesktopApp: () => true,
  settings: {
    update: updateSettings
  }
}))

vi.mock('@/composables/useGuide', () => ({
  useGuide: () => ({
    togglePanel: vi.fn()
  })
}))

describe('CanvasView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows a settings button in the canvas sidebar header', async () => {
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: { template: '<div class="settings-modal-stub" />' },
          ConfirmDialog: { template: '<div />' }
        }
      }
    })

    await flushPromises()

    expect(wrapper.find('[data-guide="canvas-settings-btn"]').exists()).toBe(true)
  })

  it('opens the existing settings modal when the canvas settings button is clicked', async () => {
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: {
            props: ['modelValue', 'isSetup'],
            template: '<div class="settings-modal-stub">{{ modelValue }}</div>'
          },
          ConfirmDialog: { template: '<div />' }
        }
      }
    })

    await flushPromises()
    await wrapper.find('[data-guide="canvas-settings-btn"]').trigger('click')

    expect(wrapper.find('.settings-modal-stub').text()).toContain('true')
  })

  it('does not reload models after unrelated settings saves', async () => {
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: {
            emits: ['saved'],
            template: '<button class="settings-modal-stub" @click="$emit(\'saved\', { modelSourceChanged: false })" />'
          },
          ConfirmDialog: { template: '<div />' }
        }
      }
    })

    await flushPromises()
    loadModels.mockClear()
    await wrapper.find('.settings-modal-stub').trigger('click')

    expect(loadModels).not.toHaveBeenCalled()
  })

  it('reloads models after settings saves that change model source', async () => {
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: {
            emits: ['saved'],
            template: '<button class="settings-modal-stub" @click="$emit(\'saved\', { modelSourceChanged: true })" />'
          },
          ConfirmDialog: { template: '<div />' }
        }
      }
    })

    await flushPromises()
    loadModels.mockClear()
    await wrapper.find('.settings-modal-stub').trigger('click')

    expect(loadModels).toHaveBeenCalledTimes(1)
  })

  it('persists the canvas theme toggle into settings', async () => {
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: { template: '<div class="settings-modal-stub" />' },
          ConfirmDialog: { template: '<div />' }
        }
      }
    })

    await flushPromises()
    await wrapper.find('[title="Toggle Theme"]').trigger('click')
    await flushPromises()

    expect(setTheme).toHaveBeenCalledWith('light')
    expect(updateSettings).toHaveBeenCalledWith({ theme: 'light' })
  })
})
