import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsModal from '@/components/SettingsModal.vue'

const { settingsGet, settingsStatus, settingsUpdate, pickVaultFolder, validateOpenRouterKey, getModels, getMcpStatus, toast, routerPush } = vi.hoisted(() => ({
  settingsGet: vi.fn(),
  settingsStatus: vi.fn(),
  settingsUpdate: vi.fn(),
  pickVaultFolder: vi.fn(),
  validateOpenRouterKey: vi.fn(),
  getModels: vi.fn(),
  getMcpStatus: vi.fn(),
  toast: {
    warning: vi.fn(),
    error: vi.fn()
  },
  routerPush: vi.fn()
}))

vi.mock('@/api/client', () => ({
  settings: {
    get: settingsGet,
    getOpenRouterStatus: settingsStatus,
    update: settingsUpdate,
    pickVaultFolder,
    validateOpenRouterKey
  },
  mcp: {
    getStatus: getMcpStatus
  },
  canvas: {
    getModels
  },
  isDesktopApp: () => true
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => toast
}))

vi.mock('@/stores/theme', () => ({
  useThemeStore: () => ({
    setTheme: vi.fn()
  })
}))

vi.mock('vue-router', () => ({
  useRouter: () => ({
    push: routerPush
  })
}))

describe('SettingsModal', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    settingsGet.mockResolvedValue({
      vault_path: 'C:\\Vault',
      theme: 'system',
      llm_model: 'openai/gpt-4o',
      smart_web_search: true
    })
    settingsStatus.mockResolvedValue({ has_key: true })
    getModels.mockResolvedValue([])
    getMcpStatus.mockResolvedValue({ available: false, config_snippet: '' })
    window.matchMedia = vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn()
    })
  })

  it('labels the toggle as Canvas Web Search and explains the default-on behavior', async () => {
    const wrapper = mount(SettingsModal, {
      props: {
        modelValue: true,
        isSetup: false
      }
    })

    await flushPromises()

    expect(wrapper.text()).toContain('Canvas Web Search')
    expect(wrapper.text()).toContain('Turn live web search on by default for normal Canvas prompts')
    expect(wrapper.text()).toContain('On by default')
    expect(wrapper.text()).not.toContain('Smart Web Search')
  })

  it('shows a masked stored key in the input instead of looking empty', async () => {
    const wrapper = mount(SettingsModal, {
      props: {
        modelValue: true,
        isSetup: false
      }
    })

    await flushPromises()

    const input = wrapper.find('.key-input')
    expect(input.element.value).toBe('sk-or-v1-stored-key')
    expect(wrapper.text()).toContain('An API key is already stored securely')
  })
})
