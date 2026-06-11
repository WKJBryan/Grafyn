import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CanvasContainer from '@/components/canvas/CanvasContainer.vue'

const { store, getOpenRouterStatus, getStatus, getSettings, updateSettings, listOllamaModels, getConstitutionSetup, saveConstitutionSetup, toastSuccess } = vi.hoisted(() => ({
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
    updatePinnedNotes: vi.fn().mockResolvedValue(),
    recordPreferenceFeedback: vi.fn().mockResolvedValue(),
    recordCorrectionFeedback: vi.fn().mockResolvedValue(),
    recordSelectionRanking: vi.fn().mockResolvedValue(),
    captureInsight: vi.fn().mockResolvedValue(),
    listMemoryDigest: vi.fn().mockResolvedValue([]),
    reviewMemoryDigestItem: vi.fn().mockResolvedValue(),
    listDecisionEpisodes: vi.fn().mockResolvedValue([]),
    updateDecisionOutcome: vi.fn().mockResolvedValue(),
    exportTwinData: vi.fn().mockResolvedValue({
      approved_user_records: { count: 1 },
      candidate_user_records: { count: 0 },
      rejected_user_records: { count: 0 }
    })
  },
  getOpenRouterStatus: vi.fn(),
  getStatus: vi.fn(),
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  listOllamaModels: vi.fn(),
  getConstitutionSetup: vi.fn(),
  saveConstitutionSetup: vi.fn(),
  toastSuccess: vi.fn()
}))

vi.mock('@/stores/canvas', () => ({
  useCanvasStore: () => store
}))

vi.mock('@/api/client', () => ({
  settings: {
    get: getSettings,
    getOpenRouterStatus,
    getStatus,
    update: updateSettings,
    listOllamaModels
  },
  twin: {
    getConstitutionSetup,
    saveConstitutionSetup
  },
  isDesktopApp: () => true
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    success: toastSuccess,
    error: vi.fn(),
    warning: vi.fn(),
    info: vi.fn(),
    remove: vi.fn(),
    toasts: []
  })
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
          template: `
            <div>
              <button class="llm-node-stub" :data-tile-id="tileId" :data-model-id="modelId" :data-web-search="String(webSearch)" @click="$emit('select', { tileId, modelId })" />
              <button class="branch-stub" @click="$emit('branch', tileId, modelId)" />
            </div>
          `
        },
        DebateNode: { template: '<div />' },
        AddModelDialog: { template: '<div />' },
        PinnedNotesPanel: { template: '<div />' },
        PromptDialog: {
          props: ['models', 'presets', 'branchContext', 'smartWebSearch', 'openRouterConfigured', 'twinLlmProvider', 'ollamaModel', 'twinIdentity'],
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
              <button class="decision-submit-stub" @click="$emit('submit', {
                prompt: 'Should we build Decision Mirror?',
                promptType: 'decision',
                decisionMetadata: {
                  decision: 'Should we build Decision Mirror?',
                  options: ['Decision Mirror', 'Topology'],
                  stakes: 'Product direction',
                  initial_leaning: 'Decision Mirror',
                  review_date: '2026-05-15'
                },
                models: ['openai/gpt-4o'],
                systemPrompt: null,
                temperature: 0.4,
                contextMode: 'twin',
                twinAnswerMode: 'simulation',
                twinLlmProvider: 'ollama',
                webSearch: false
              })" />
              <button class="identity-submit-stub" @click="$emit('submit', {
                prompt: 'Simulate my likely response',
                promptType: 'standard',
                models: ['llama3.1:8b'],
                systemPrompt: null,
                temperature: 0.4,
                contextMode: 'twin',
                twinAnswerMode: 'simulation',
                twinLlmProvider: 'ollama',
                webSearch: false,
                twinIdentitySetup: {
                  twin_name: 'Alex Chen',
                  twin_role: 'founder deciding from product evidence'
                }
              })" />
            </div>
          `
        }
      }
    }
  })
}

function mountContainerWithPresetPromptStub() {
  return mount(CanvasContainer, {
    props: {
      sessionId: 'session-1'
    },
    global: {
      stubs: {
        PromptNode: { template: '<div />' },
        LLMNode: { template: '<div />' },
        DebateNode: { template: '<div />' },
        AddModelDialog: { template: '<div />' },
        PinnedNotesPanel: { template: '<div />' },
        PromptDialog: {
          props: ['presets'],
          template: `
            <div class="prompt-dialog-stub">
              <div class="preset-count">{{ presets.length }}</div>
              <button class="create-preset-stub" @click="$emit('create-preset', { name: 'Fast trio', modelIds: ['openai/gpt-4o'] })" />
              <button class="update-preset-stub" @click="$emit('update-preset', { id: 'preset-1', modelIds: ['openai/gpt-4o'] })" />
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
          template: `
            <div class="add-model-dialog-stub">
              <span class="existing-models">{{ existingModelIds.join(',') }}</span>
              <span class="available-models">{{ models.map(model => model.id).join(',') }}</span>
              <button class="add-local-model-submit" @click="$emit('submit', ['qwen3:14b'])" />
            </div>
          `
        },
        PinnedNotesPanel: { template: '<div />' },
        PromptDialog: { template: '<div />' }
      }
    }
  })
}

async function openTwinActions(wrapper) {
  await wrapper.findAll('button').find(button => button.text().includes('Twin')).trigger('click')
  await flushPromises()
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
    store.listMemoryDigest.mockResolvedValue([])
    getSettings.mockResolvedValue({ smart_web_search: true, canvas_model_presets: [] })
    getStatus.mockResolvedValue({ smart_web_search: true })
    updateSettings.mockResolvedValue({})
    listOllamaModels.mockResolvedValue([
      { id: 'llama3.1:8b', name: 'llama3.1:8b', provider: 'Ollama' },
      { id: 'qwen3:14b', name: 'qwen3:14b', provider: 'Ollama' }
    ])
    getConstitutionSetup.mockResolvedValue({
      twin_name: 'Alex Chen',
      twin_role: 'founder deciding from product evidence'
    })
    saveConstitutionSetup.mockResolvedValue({})
    toastSuccess.mockReset()
  })

  it('opens the prompt dialog without an API key so users can choose a local twin runtime', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: false, is_configured: false })

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('.prompt-dialog-stub').exists()).toBe(true)
    expect(wrapper.text()).not.toContain('OpenRouter API Key Required')
  })

  it('saves inline Twin Identity before submitting a simulation prompt', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    getConstitutionSetup.mockResolvedValueOnce({
      values: ['evidence-backed work'],
      tastes: [],
      constraints: [],
      somatic_cues: [],
      action_tendencies: []
    }).mockResolvedValue({
      twin_name: 'Alex Chen',
      twin_role: 'founder deciding from product evidence',
      values: ['evidence-backed work'],
      tastes: [],
      constraints: [],
      somatic_cues: [],
      action_tendencies: []
    })
    const wrapper = mountContainer()
    await flushPromises()

    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.identity-submit-stub').trigger('click')
    await flushPromises()

    expect(saveConstitutionSetup).toHaveBeenCalledWith(expect.objectContaining({
      twin_name: 'Alex Chen',
      twin_role: 'founder deciding from product evidence',
      values: ['evidence-backed work']
    }))
    expect(store.sendPrompt).toHaveBeenCalledWith(
      'Simulate my likely response',
      ['llama3.1:8b'],
      null,
      0.4,
      null,
      null,
      null,
      'twin',
      'simulation',
      false,
      undefined,
      'standard',
      null,
      'none',
      'ollama'
    )
  })

  it('blocks API submit if OpenRouter becomes unavailable after the dialog opens', async () => {
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

  it('allows a local twin submit without an OpenRouter key', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: false, is_configured: false })
    getStatus.mockResolvedValue({ smart_web_search: true, twin_llm_provider: 'ollama', ollama_model: 'llama3.1:8b' })

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.decision-submit-stub').trigger('click')
    await flushPromises()

    expect(store.sendPrompt).toHaveBeenCalled()
    expect(wrapper.text()).not.toContain('OpenRouter API Key Required')
  })

  it('passes Decision Mirror metadata through the Canvas submit path', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.decision-submit-stub').trigger('click')
    await flushPromises()

    expect(store.sendPrompt).toHaveBeenCalledWith(
      'Should we build Decision Mirror?',
      ['openai/gpt-4o'],
      null,
      0.4,
      null,
      null,
      null,
      'twin',
      'simulation',
      false,
      undefined,
      'decision',
      {
        decision: 'Should we build Decision Mirror?',
        options: ['Decision Mirror', 'Topology'],
        stakes: 'Product direction',
        initial_leaning: 'Decision Mirror',
        review_date: '2026-05-15'
      },
      'none',
      'ollama'
    )
  })

  it('opens a new prompt as a root even after a branch dialog was opened', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.getParentResponseContent.mockReturnValue({
      prompt: 'Parent prompt',
      model: 'openai/gpt-4o',
      content: 'Parent answer'
    })
    store.promptTiles = [
      {
        id: 'parent-tile',
        prompt: 'Parent prompt',
        web_search: false,
        responses: {
          'openai/gpt-4o': {
            status: 'completed',
            content: 'Parent answer',
            model_name: 'GPT-4o',
            color: '#7c5cff',
            position: { x: 320, y: 40, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('.branch-stub').trigger('click')
    await flushPromises()
    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.submit-stub').trigger('click')
    await flushPromises()

    expect(store.branchFromResponse).not.toHaveBeenCalled()
    expect(store.sendPrompt).toHaveBeenCalledWith(
      'hello',
      ['openai/gpt-4o'],
      null,
      0.7,
      null,
      null,
      null,
      'knowledge_search',
      'simulation',
      false,
      undefined,
      'standard',
      null,
      'none',
      null
    )
  })

  it('submits an explicit branch with the selected parent response ids', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.getParentResponseContent.mockReturnValue({
      prompt: 'Parent prompt',
      model: 'openai/gpt-4o',
      content: 'Parent answer'
    })
    store.promptTiles = [
      {
        id: 'parent-tile',
        prompt: 'Parent prompt',
        web_search: false,
        responses: {
          'openai/gpt-4o': {
            status: 'completed',
            content: 'Parent answer',
            model_name: 'GPT-4o',
            color: '#7c5cff',
            position: { x: 320, y: 40, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('.branch-stub').trigger('click')
    await flushPromises()
    await wrapper.find('.submit-stub').trigger('click')
    await flushPromises()

    expect(store.sendPrompt).not.toHaveBeenCalled()
    expect(store.branchFromResponse).toHaveBeenCalledWith(
      'parent-tile',
      'openai/gpt-4o',
      'hello',
      ['openai/gpt-4o'],
      null,
      0.7,
      null,
      'knowledge_search',
      'simulation',
      false,
      undefined,
      'standard',
      null,
      'none',
      null
    )
  })

  it('does not surface memory digest review inside Canvas', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.listMemoryDigest.mockResolvedValue([
      {
        id: 'digest-record-1',
        pattern: 'User benefits from hard go/no-go gates before scaling.',
        evidence_count: 3,
        confidence: 0.82,
        trigger_reason: '3+ evidence points support this durable pattern'
      }
    ])

    const wrapper = mountContainer()
    await flushPromises()

    expect(wrapper.text()).not.toContain('Review 1')
    expect(wrapper.text()).not.toContain('User benefits from hard go/no-go gates before scaling.')
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
    expect(wrapper.find('.existing-models').text()).toContain('openai/gpt-4o')
    expect(wrapper.find('.existing-models').text()).toContain('anthropic/claude-3.5-sonnet')
    expect(wrapper.find('.available-models').text()).toContain('openai/gpt-4o')
  })

  it('uses installed Ollama models when adding a model to a local twin tile', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: false, is_configured: false })
    store.promptTiles = [
      {
        id: 'tile-1',
        prompt: 'Use my twin records',
        prompt_type: 'decision',
        context_mode: 'twin',
        twin_llm_provider: 'ollama',
        web_search: false,
        responses: {
          'llama3.1:8b': {
            status: 'completed',
            content: 'Answer',
            model_name: 'llama3.1:8b',
            position: { x: 320, y: 40, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]

    const wrapper = mountContainerWithAddModelPromptStub()
    await flushPromises()
    await wrapper.find('.prompt-add-model-stub').trigger('click')
    await flushPromises()

    expect(listOllamaModels).toHaveBeenCalled()
    expect(wrapper.find('.existing-models').text()).toContain('llama3.1:8b')
    expect(wrapper.find('.available-models').text()).toContain('qwen3:14b')
    expect(wrapper.find('.available-models').text()).not.toContain('openai/gpt-4o')

    await wrapper.find('.add-local-model-submit').trigger('click')
    await flushPromises()

    expect(store.addModelToTile).toHaveBeenCalledWith('tile-1', ['qwen3:14b'])
  })

  it('loads saved presets from settings and passes them into the prompt dialog', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    getSettings.mockResolvedValue({
      smart_web_search: true,
      canvas_model_presets: [
        { id: 'preset-1', name: 'Quality', model_ids: ['openai/gpt-4o'] }
      ]
    })

    const wrapper = mountContainerWithPresetPromptStub()
    await flushPromises()

    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('.preset-count').text()).toBe('1')
  })

  it('persists a created preset through settings updates', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })

    const wrapper = mountContainerWithPresetPromptStub()
    await flushPromises()

    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.create-preset-stub').trigger('click')
    await flushPromises()

    expect(updateSettings).toHaveBeenCalledWith({
      canvas_model_presets: [
        expect.objectContaining({
          name: 'Fast trio',
          model_ids: ['openai/gpt-4o']
        })
      ]
    })
    expect(wrapper.text()).toContain('Saved preset "Fast trio"')
  })

  it('persists preset model updates through settings updates', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    getSettings.mockResolvedValue({
      smart_web_search: true,
      canvas_model_presets: [
        { id: 'preset-1', name: 'Quality', model_ids: ['anthropic/claude-3.5-sonnet'] }
      ]
    })

    const wrapper = mountContainerWithPresetPromptStub()
    await flushPromises()

    await wrapper.find('[data-guide="canvas-prompt-btn"]').trigger('click')
    await flushPromises()
    await wrapper.find('.update-preset-stub').trigger('click')
    await flushPromises()

    expect(updateSettings).toHaveBeenCalledWith({
      canvas_model_presets: [
        {
          id: 'preset-1',
          name: 'Quality',
          model_ids: ['openai/gpt-4o']
        }
      ]
    })
    expect(toastSuccess).toHaveBeenCalledWith('Updated preset "Quality"', 2500)
    expect(wrapper.text()).toContain('Updated preset "Quality"')
  })

  it('records accept and reject feedback for a completed selected response', async () => {
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
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]
    const promptSpy = vi.spyOn(globalThis, 'prompt').mockReturnValue('fits my workflow')

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('.llm-node-stub').trigger('click')
    await openTwinActions(wrapper)
    await wrapper.findAll('button').find(button => button.text() === 'Matches Me').trigger('click')
    await flushPromises()

    expect(store.recordPreferenceFeedback).toHaveBeenCalledWith(
      'tile-1',
      'openai/gpt-4o',
      'accept',
      'fits my workflow'
    )

    await wrapper.find('.llm-node-stub').trigger('click')
    await openTwinActions(wrapper)
    await wrapper.findAll('button').find(button => button.text() === 'Not Me').trigger('click')
    await flushPromises()

    expect(store.recordPreferenceFeedback).toHaveBeenCalledWith(
      'tile-1',
      'openai/gpt-4o',
      'reject',
      'fits my workflow'
    )

    promptSpy.mockRestore()
  })

  it('records correction feedback for a completed selected response', async () => {
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
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]
    const promptSpy = vi
      .spyOn(globalThis, 'prompt')
      .mockReturnValueOnce('The correct value is local only.')
      .mockReturnValueOnce('The answer assumed cloud state.')

    const wrapper = mountContainer()
    await flushPromises()
    await wrapper.find('.llm-node-stub').trigger('click')
    await openTwinActions(wrapper)
    await wrapper.findAll('button').find(button => button.text() === 'Correct').trigger('click')
    await flushPromises()

    expect(store.recordCorrectionFeedback).toHaveBeenCalledWith(
      'tile-1',
      'openai/gpt-4o',
      'The correct value is local only.',
      'The answer assumed cloud state.'
    )

    promptSpy.mockRestore()
  })

  it('records rank feedback for completed selected responses', async () => {
    getOpenRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    store.promptTiles = [
      {
        id: 'tile-1',
        prompt: 'Prompt',
        web_search: false,
        responses: {
          'openai/gpt-4o': {
            status: 'completed',
            content: 'Answer A',
            model_name: 'GPT-4o',
            color: '#7c5cff',
            position: { x: 320, y: 40, width: 280, height: 200 }
          },
          'anthropic/claude-3.5-sonnet': {
            status: 'completed',
            content: 'Answer B',
            model_name: 'Claude',
            color: '#00a37f',
            position: { x: 320, y: 260, width: 280, height: 200 }
          }
        },
        position: { x: 40, y: 40, width: 200, height: 120 }
      }
    ]
    const promptSpy = vi.spyOn(globalThis, 'prompt').mockReturnValue('A is closer to my thinking')

    const wrapper = mountContainer()
    await flushPromises()
    const nodes = wrapper.findAll('.llm-node-stub')
    await nodes[0].trigger('click')
    await nodes[1].trigger('click')
    await openTwinActions(wrapper)
    await wrapper.findAll('button').find(button => button.text() === 'Rank Selection').trigger('click')
    await flushPromises()

    expect(store.recordSelectionRanking).toHaveBeenCalledWith(
      [
        { tile_id: 'tile-1', model_id: 'openai/gpt-4o' },
        { tile_id: 'tile-1', model_id: 'anthropic/claude-3.5-sonnet' }
      ],
      'A is closer to my thinking'
    )

    promptSpy.mockRestore()
  })
})
