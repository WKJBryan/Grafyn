import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CanvasView from '@/views/CanvasView.vue'

const push = vi.fn()
const loadSessions = vi.fn().mockResolvedValue()
const loadSession = vi.fn().mockResolvedValue()
const loadModels = vi.fn().mockResolvedValue()

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
    toggleTheme: vi.fn()
  })
}))

vi.mock('@/api/client', () => ({
  isDesktopApp: () => true
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
})
