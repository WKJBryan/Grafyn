import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CanvasView from '@/views/CanvasView.vue'

const {
  push,
  replace,
  routeState,
  canvasState,
  loadModels,
  updateSettings,
  setTheme,
} = vi.hoisted(() => {
  const loadSessions = vi.fn().mockResolvedValue()
  const loadSession = vi.fn().mockResolvedValue()
  const loadModels = vi.fn().mockResolvedValue()
  return {
    push: vi.fn(),
    replace: vi.fn(),
    routeState: { params: { id: 'session-1' } },
    canvasState: {
      sessions: [],
      currentSession: null,
      loading: false,
      loadSessions,
      loadSession,
      loadModels,
      createSession: vi.fn(),
      deleteSession: vi.fn(),
      clearSession: vi.fn(),
    },
    loadModels,
    updateSettings: vi.fn().mockResolvedValue({}),
    setTheme: vi.fn(),
  }
})

vi.mock('vue-router', () => ({
  useRoute: () => routeState,
  useRouter: () => ({
    push,
    replace,
  }),
  RouterLink: {
    props: ['to'],
    template: '<a><slot /></a>'
  }
}))

vi.mock('@/stores/canvas', () => ({
  useCanvasStore: () => canvasState,
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
    routeState.params.id = 'session-1'
    canvasState.sessions = []
    canvasState.currentSession = null
    canvasState.loading = false
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

  it('hides companion Twin chat sessions from the wide Canvas session list', async () => {
    canvasState.sessions = [
      { id: 'session-1', title: 'Ordinary Canvas', tile_count: 1, updated_at: '2026-09-01T00:00:00Z', tags: [] },
      { id: 'twin-chat', title: 'Private Twin chat', tile_count: 2, updated_at: '2026-09-01T01:00:00Z', tags: ['companion-twin-chat'] },
    ]
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: { template: '<div />' },
          ConfirmDialog: { template: '<div />' },
        },
      },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Ordinary Canvas')
    expect(wrapper.text()).not.toContain('Private Twin chat')
  })

  it('clears and rejects a tagged Twin chat deep link in wide Canvas', async () => {
    routeState.params.id = 'twin-chat'
    canvasState.currentSession = {
      id: 'twin-chat',
      title: 'Private Twin chat',
      tags: ['companion-twin-chat'],
      prompt_tiles: [],
      debates: [],
    }
    const wrapper = mount(CanvasView, {
      global: {
        stubs: {
          RouterLink: { template: '<a><slot /></a>' },
          CanvasContainer: { template: '<div class="canvas-container-stub" />' },
          SettingsModal: { template: '<div />' },
          ConfirmDialog: { template: '<div />' },
        },
      },
    })
    await flushPromises()

    expect(canvasState.clearSession).toHaveBeenCalledOnce()
    expect(replace).toHaveBeenCalledWith('/canvas')
    expect(wrapper.find('.canvas-container-stub').exists()).toBe(false)
  })
})
