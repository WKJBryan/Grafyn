import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CanvasContainer from '@/components/canvas/CanvasContainer.vue'

const { store, getOpenRouterStatus, getStatus } = vi.hoisted(() => ({
  store: {
    currentSession: {
      id: 'session-1',
      title: 'Canvas',
      prompt_tiles: [],
      debates: [],
      viewport: { x: 0, y: 0, zoom: 1 }
    },
    promptTiles: [],
    debates: [],
    availableModels: [{ id: 'openai/gpt-4o', name: 'GPT-4o', context_length: 128000 }],
    loading: false,
    streamingModels: new Set(),
    debateStreamingContent: {},
    loadSession: vi.fn().mockResolvedValue(),
    loadModels: vi.fn().mockResolvedValue(),
    branchFromResponse: vi.fn().mockResolvedValue(),
    sendPrompt: vi.fn().mockResolvedValue(),
    updateTilePosition: vi.fn(),
    updateLLMNodePosition: vi.fn(),
    autoArrange: vi.fn(),
    deleteTile: vi.fn(),
    deleteResponse: vi.fn(),
    updateViewport: vi.fn(),
    startDebate: vi.fn().mockResolvedValue(),
    continueDebate: vi.fn().mockResolvedValue(),
    saveAsNote: vi.fn().mockResolvedValue(),
    clearError: vi.fn(),
    clearSession: vi.fn(),
    reset: vi.fn(),
    getParentResponseContent: vi.fn(),
    thinkHarderFromResponse: vi.fn().mockResolvedValue(),
    addModelToTile: vi.fn().mockResolvedValue(),
    regenerateResponse: vi.fn().mockResolvedValue(),
    updatePinnedNotes: vi.fn().mockResolvedValue()
  },
  getOpenRouterStatus: vi.fn(),
  getStatus: vi.fn()
}))

vi.mock('@/stores/canvas', () => ({
  useCanvasStore: () => store
}))

vi.mock('@/api/client', () => ({
  settings: {
    getOpenRouterStatus,
    getStatus
  },
  isDesktopApp: () => true
}))

vi.mock('d3-selection', () => ({
  select: () => ({
    call() { return this },
    transition() { return this },
    duration() { return this }
  })
}))

vi.mock('d3-zoom', () => {
  const zoomFn = () => zoomFn
  zoomFn.scaleExtent = () => zoomFn
  zoomFn.filter = () => zoomFn
  zoomFn.on = () => zoomFn

  const zoomIdentity = {
    translate() { return this },
    scale() { return this }
  }

  return {
    zoom: () => zoomFn,
    zoomIdentity
  }
})

vi.mock('d3-force', () => ({
  forceSimulation: () => ({
    force() { return this },
    stop() { return this },
    tick() { return this }
  }),
  forceLink: () => ({
    id() { return this },
    distance() { return this }
  }),
  forceManyBody: () => ({
    strength() { return this }
  }),
  forceCenter: () => ({}),
  forceCollide: () => ({
    radius() { return this }
  })
}))

vi.mock('d3-transition', () => ({}))

function mountContainer() {
  return mount(CanvasContainer, {
    props: {
      sessionId: 'session-1'
    },
    global: {
      stubs: {
        PromptNode: { template: '<div />' },
        LLMNode: {
          props: ['tileId', 'modelId', 'response', 'webSearch'],
          template: '<div class="llm-node-stub" :data-tile-id="tileId" :data-model-id="modelId" :data-web-search="String(webSearch)" />'
        },
        DebateNode: { template: '<div />' },
        AddModelDialog: { template: '<div />' },
        PinnedNotesPanel: { template: '<div />' },
        PromptDialog: {
          props: ['models', 'branchContext', 'smartWebSearch', 'openRouterConfigured'],
          template: `
            <div class="prompt-dialog-stub">
              <button class="submit-stub" @click="$emit('submit', {
                prompt: 'hello',
                models: ['openai/gpt-4o'],
                systemPrompt: null,
                temperature: 0.7,
                contextMode: 'knowledge_search',
                webSearch: false
              })" />
            </div>
          `
        }
      }
    }
  })
}

function mountContainerWithAddModelPromptStub() {
  return mount(CanvasContainer, {
    props: {
      sessionId: 'session-1'
    },
    global: {
      stubs: {
        PromptNode: {
          template: '<button class="prompt-add-model-stub" @click="$emit(\'show-add-model-dialog\', { tileId: \'tile-1\' })">Add model</button>'
        },
        LLMNode: { template: '<div />' },
        DebateNode: { template: '<div />' },
        AddModelDialog: {
          props: ['models', 'existingModelIds'],
          template: '<div class="add-model-dialog-stub">{{ existingModelIds.join(\',\') }}</div>'
        },
        PinnedNotesPanel: { template: '<div />' },
        PromptDialog: { template: '<div />' }
      }
    }
  })
}

describe('CanvasContainer', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    store.currentSession = {
      id: 'session-1',
      title: 'Canvas',
      prompt_tiles: [],
      debates: [],
      viewport: { x: 0, y: 0, zoom: 1 }
    }
    store.promptTiles = []
    store.debates = []
    store.availableModels = [{ id: 'openai/gpt-4o', name: 'GPT-4o', context_length: 128000 }]
    getStatus.mockResolvedValue({ smart_web_search: true })
  })

  it('opens the API key required dialog when the prompt button is clicked without a configured key', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: false, is_configured: false })

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('OpenRouter API Key Required')
    expect(wrapper.find('.prompt-dialog-stub').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Open Settings')
    expect(wrapper.text()).toContain('Close')
  })

  it('blocks submit if OpenRouter becomes unavailable after the dialog opens', async () => {
    getOpenRouterStatus
      .mockResolvedValueOnce({ has_key: true, is_configured: true })
      .mockResolvedValueOnce({ has_key: true, is_configured: true })
      .mockResolvedValueOnce({ has_key: false, is_configured: false })

    const wrapper = mountContainer()
    await flushPromises()

    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('.prompt-dialog-stub').exists()).toBe(true)

    await wrapper.find('.submit-stub').trigger('click')
    await flushPromises()

    expect(store.sendPrompt).not.toHaveBeenCalled()
    expect(store.branchFromResponse).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('OpenRouter API Key Required')
  })

  it('passes prompt tile web-search state through to response nodes', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.promptTiles = [
      {
        id: 'tile-web',
        prompt: 'Search the web',
        web_search: true,
        responses: {
          'openai/gpt-4o': {
            status: 'completed',
            content: 'Web answer',
            model_name: 'GPT-4o',
            color: '#7c5cff',
            position: { x: 320, y: 40, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      },
      {
        id: 'tile-notes',
        prompt: 'Search my notes',
        web_search: false,
        responses: {
          'anthropic/claude-3.5-sonnet': {
            status: 'completed',
            content: 'Note answer',
            model_name: 'Claude',
            color: '#00a37f',
            position: { x: 320, y: 260, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 260, width: 200, height: 120 }
      }
    ]

    const wrapper = mountContainer()
    await flushPromises()

    const llmNodes = wrapper.findAll('.llm-node-stub')
    expect(llmNodes).toHaveLength(2)
    expect(llmNodes[0].attributes('data-web-search')).toBe('true')
    expect(llmNodes[1].attributes('data-web-search')).toBe('false')
  })

  it('opens the add-model dialog from a prompt tile and passes existing model ids', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.promptTiles = [
      {
        id: 'tile-1',
        prompt: 'Prompt',
        web_search: false,
        responses: {
          'openai/gpt-4o': {
            status: 'completed',
            content: 'Answer',
            model_name: 'GPT-4o',
            color: '#7c5cff',
            position: { x: 320, y: 40, width: 280, height: 200 }
          },
          'anthropic/claude-3.5-sonnet': {
            status: 'completed',
            content: 'Answer',
            model_name: 'Claude',
            color: '#00a37f',
            position: { x: 320, y: 260, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]

    const wrapper = mountContainerWithAddModelPromptStub()
    await flushPromises()

    await wrapper.find('.prompt-add-model-stub').trigger('click')
    await flushPromises()

    const dialog = wrapper.find('.add-model-dialog-stub')
    expect(dialog.exists()).toBe(true)
    expect(dialog.text()).toContain('openai/gpt-4o')
    expect(dialog.text()).toContain('anthropic/claude-3.5-sonnet')
  })
})
