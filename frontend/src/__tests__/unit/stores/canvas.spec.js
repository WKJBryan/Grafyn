import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useCanvasStore, THINK_HARDER_PROMPT, THINK_HARDER_WEB_SEARCH_MAX_RESULTS } from '@/stores/canvas'
import * as apiClient from '@/api/client'

const { listenMock, unlistenMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

function mockCompletedSendPrompt(tileId, modelId) {
  let streamHandler
  listenMock.mockImplementation(async (_eventName, handler) => {
    streamHandler = handler
    return unlistenMock
  })
  return vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
    streamHandler({
      payload: {
        session_id: 'session-1',
        type: 'complete',
        tile_id: tileId,
        model_id: modelId
      }
    })
    return tileId
  })
}

describe('Canvas Store', () => {
  it('keeps the provider-reported cost when a streamed response completes', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })
    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({ payload: { session_id: 'session-1', type: 'tile_created', tile: {
        id: 'tile-1', responses: { 'openai/gpt-4': { content: '', status: 'pending' } }
      } } })
      streamHandler({ payload: { session_id: 'session-1', type: 'complete', tile_id: 'tile-1', model_id: 'openai/gpt-4', cost_usd: 0.002341 } })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }
    await store.sendPrompt('Hello', ['openai/gpt-4'])

    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4'].cost_usd).toBe(0.002341)
  })

  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    listenMock.mockResolvedValue(unlistenMock)
  })

  it('sendCompanionPrompt defaults to a single-model Twin-history Advisor request', async () => {
    const sendPromptSpy = mockCompletedSendPrompt('tile-companion', 'openai/gpt-4')
    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

    const tileId = await store.sendCompanionPrompt({
      prompt: 'What should I do next?',
      modelId: 'openai/gpt-4'
    })

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      prompt: 'What should I do next?',
      models: ['openai/gpt-4'],
      context_mode: 'twin_history',
      twin_answer_mode: 'advisor',
      twin_relationship_variant: { relationships: [] }
    }))
    expect(sendPromptSpy.mock.calls[0][1]).not.toHaveProperty('position')
    expect(tileId).toBe('tile-companion')
    expect(store.isStreaming).toBe(false)
    expect(store.error).toBeNull()
  })

  it('sendCompanionPrompt forwards an explicit Simulation answer mode for Twin history', async () => {
    const sendPromptSpy = mockCompletedSendPrompt('tile-simulation', 'openai/gpt-4')
    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

    await store.sendCompanionPrompt({
      prompt: 'How would I respond?',
      modelId: 'openai/gpt-4',
      mode: 'twin',
      answerMode: 'simulation',
      relationshipVariant: {
        relationships: [{
          subject_id: 'owner',
          predicate: 'with',
          object_id: 'person-alex',
          direction: 'directed'
        }]
      }
    })

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      context_mode: 'twin_history',
      twin_answer_mode: 'simulation',
      twin_relationship_variant: {
        relationships: [{
          subject_id: 'owner',
          predicate: 'with',
          object_id: 'person-alex',
          direction: 'directed'
        }]
      }
    }))
  })

  it.each([
    ['knowledge', 'simulation'],
    ['twin', 'impersonation']
  ])(
    'sendCompanionPrompt rejects mode %s with answer mode %s before starting a stream',
    async (mode, answerMode) => {
      const sendPromptSpy = mockCompletedSendPrompt('tile-invalid-answer-mode', 'openai/gpt-4')
      const store = useCanvasStore()
      store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

      await expect(store.sendCompanionPrompt({
        prompt: 'Continue',
        modelId: 'openai/gpt-4',
        mode,
        answerMode
      })).rejects.toThrow(/answer mode|Simulation requires Twin/i)

      expect(sendPromptSpy).not.toHaveBeenCalled()
      expect(listenMock).not.toHaveBeenCalled()
    }
  )

  it.each([
    ['plain', 'none'],
    ['knowledge', 'knowledge_search'],
    ['twin', 'twin_history']
  ])('sendCompanionPrompt maps %s mode to %s context', async (mode, contextMode) => {
    const sendPromptSpy = mockCompletedSendPrompt(`tile-${mode}`, 'openai/gpt-4')
    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

    await store.sendCompanionPrompt({
      prompt: 'Continue',
      modelId: 'openai/gpt-4',
      mode
    })

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      context_mode: contextMode
    }))
  })

  it.each(['semantic', 'constructor'])(
    'sendCompanionPrompt rejects unknown mode %s before starting a stream',
    async (mode) => {
      const sendPromptSpy = mockCompletedSendPrompt('tile-invalid', 'openai/gpt-4')
      const store = useCanvasStore()
      store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

      await expect(store.sendCompanionPrompt({
        prompt: 'Continue',
        modelId: 'openai/gpt-4',
        mode
      })).rejects.toThrow(/companion mode/i)

      expect(sendPromptSpy).not.toHaveBeenCalled()
      expect(listenMock).not.toHaveBeenCalled()
      expect(store.isStreaming).toBe(false)
    }
  )

  it('sendCompanionPrompt forwards parent IDs and provider without changing them', async () => {
    const sendPromptSpy = mockCompletedSendPrompt('tile-child', 'llama3.2')
    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

    await store.sendCompanionPrompt({
      prompt: 'Continue from that answer',
      modelId: 'llama3.2',
      mode: 'knowledge',
      parentTileId: 'tile-parent',
      parentModelId: 'openai/gpt-4',
      provider: 'ollama'
    })

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      models: ['llama3.2'],
      context_mode: 'knowledge_search',
      parent_tile_id: 'tile-parent',
      parent_model_id: 'openai/gpt-4',
      twin_llm_provider: 'ollama'
    }))
  })

  it('thinkHarderFromResponse creates a same-model full-history request with deeper defaults and no max token cap', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'child-tile',
          model_id: 'openai/gpt-4'
        }
      })
      return 'child-tile'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'tile-1',
          prompt: 'Fast answer prompt',
          reasoning_effort: 'minimal',
          responses: {
            'openai/gpt-4': {
              content: 'Fast answer',
              status: 'completed',
              position: { x: 10, y: 20, width: 280, height: 200 }
            }
          }
        }
      ],
      debates: []
    }

    await store.thinkHarderFromResponse('tile-1', 'openai/gpt-4', { webSearch: true })

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      prompt: THINK_HARDER_PROMPT,
      models: ['openai/gpt-4'],
      parent_tile_id: 'tile-1',
      parent_model_id: 'openai/gpt-4',
      context_mode: 'full_history',
      temperature: 0.3,
      reasoning_effort: 'high',
      web_search: true,
      web_search_max_results: THINK_HARDER_WEB_SEARCH_MAX_RESULTS,
      system_prompt: expect.stringContaining('Verify factual claims')
    }))
    expect(sendPromptSpy.mock.calls[0][1]).not.toHaveProperty('max_tokens')
  })

  it('loadModels surfaces a failure via store.error instead of failing silently', async () => {
    vi.spyOn(apiClient.canvas, 'getModels').mockRejectedValue(new Error('OpenRouter unreachable'))

    const store = useCanvasStore()
    await store.loadModels()

    expect(store.error).toBe('OpenRouter unreachable')
    expect(store.availableModels).toEqual([])
  })

  it('loadModels clears a previous error on success', async () => {
    const store = useCanvasStore()
    store.error = 'stale error'
    vi.spyOn(apiClient.canvas, 'getModels').mockResolvedValue([{ id: 'openai/gpt-4o', name: 'GPT-4o' }])

    await store.loadModels()

    expect(store.error).toBe(null)
    expect(store.availableModels).toEqual([{ id: 'openai/gpt-4o', name: 'GPT-4o' }])
  })

  it('sendPrompt includes reasoning effort and omits max_tokens for normal prompts', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async (_sessionId, _request) => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4'
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    await store.sendPrompt(
      'Hello',
      ['openai/gpt-4'],
      null,
      0.7,
      null,
      null,
      null,
      'knowledge_search',
      'advisor',
      false,
      5,
      'standard',
      null,
      'medium'
    )

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      prompt: 'Hello',
      models: ['openai/gpt-4'],
      temperature: 0.7,
      reasoning_effort: 'medium',
      context_mode: 'knowledge_search',
      web_search: false,
      web_search_max_results: 5
    }))
    expect(sendPromptSpy.mock.calls[0][1]).not.toHaveProperty('max_tokens')
  })

  it('startDebate inherits the highest source tile reasoning effort', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })
    vi.spyOn(apiClient.canvas, 'get').mockResolvedValue({
      id: 'session-1',
      prompt_tiles: [],
      debates: [{ id: 'debate-1', reasoning_effort: 'high' }]
    })

    const startDebateSpy = vi.spyOn(apiClient.canvas, 'startDebate').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'debate_complete',
          debate_id: 'debate-1'
        }
      })
      return 'debate-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        { id: 'tile-low', reasoning_effort: 'low', responses: {} },
        { id: 'tile-high', reasoning_effort: 'high', responses: {} }
      ],
      debates: []
    }

    await store.startDebate(['tile-low', 'tile-high'], ['openai/gpt-4'], 'auto', 3)

    expect(startDebateSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      source_tile_ids: ['tile-low', 'tile-high'],
      reasoning_effort: 'high'
    }))
  })

  it('continueDebate reuses stored debate reasoning effort', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })
    vi.spyOn(apiClient.canvas, 'get').mockResolvedValue({
      id: 'session-1',
      prompt_tiles: [],
      debates: [{ id: 'debate-1', reasoning_effort: 'xhigh' }]
    })

    const continueDebateSpy = vi.spyOn(apiClient.canvas, 'continueDebate').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'debate_complete',
          debate_id: 'debate-1'
        }
      })
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: [
        {
          id: 'debate-1',
          participating_models: ['openai/gpt-4'],
          reasoning_effort: 'xhigh'
        }
      ]
    }

    await store.continueDebate('debate-1', 'Continue')

    expect(continueDebateSpy).toHaveBeenCalledWith('session-1', 'debate-1', {
      prompt: 'Continue',
      reasoning_effort: 'xhigh'
    })
  })

  it('continueDebate rejects a blank prompt with a user-visible error and never calls the API', async () => {
    const continueDebateSpy = vi.spyOn(apiClient.canvas, 'continueDebate')

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: [
        { id: 'debate-1', participating_models: ['openai/gpt-4'], reasoning_effort: 'high' }
      ]
    }

    await expect(store.continueDebate('debate-1', '   ')).rejects.toThrow(/prompt/i)
    expect(continueDebateSpy).not.toHaveBeenCalled()
    expect(store.error).toMatch(/prompt/i)

    await expect(store.continueDebate('debate-1', undefined)).rejects.toThrow(/prompt/i)
    expect(continueDebateSpy).not.toHaveBeenCalled()
  })

  it('sendPrompt includes twin answer mode and context policy for Twin Mode', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4'
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    await store.sendPrompt(
      'What would my twin consider?',
      ['openai/gpt-4'],
      null,
      0.7,
      null,
      null,
      null,
      'twin',
      'simulation'
    )

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      context_mode: 'twin',
      twin_answer_mode: 'simulation',
      twin_context_policy: 'approved_plus_relevant_candidates',
      twin_llm_provider: null
    }))
  })

  it('sendPrompt includes prompt-level twin provider override', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4'
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    await store.sendPrompt(
      'What would my twin consider?',
      ['openai/gpt-4'],
      null,
      0.7,
      null,
      null,
      null,
      'twin',
      'advisor',
      false,
      5,
      'standard',
      null,
      'none',
      'ollama'
    )

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      context_mode: 'twin',
      twin_llm_provider: 'ollama'
    }))
  })

  it('sendPrompt includes Decision Mirror type and metadata', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4'
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    const decisionMetadata = {
      decision: 'Should we build Decision Mirror?',
      options: ['Decision Mirror', 'Topology'],
      stakes: 'Product direction'
    }

    await store.sendPrompt(
      'Should we build Decision Mirror?',
      ['openai/gpt-4'],
      null,
      0.4,
      null,
      null,
      null,
      'twin',
      'advisor',
      false,
      5,
      'decision',
      decisionMetadata
    )

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      prompt_type: 'decision',
      decision_metadata: decisionMetadata,
      context_mode: 'twin',
      twin_answer_mode: 'advisor'
    }))
  })

  it('wraps Decision Mirror digest and outcome twin APIs', async () => {
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([{ id: 'digest-1' }])
    vi.spyOn(apiClient.twin, 'reviewMemoryDigestItem').mockResolvedValue({ id: 'digest-1', state: 'kept' })
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([{ id: 'decision-1' }])
    vi.spyOn(apiClient.twin, 'updateDecisionOutcome').mockResolvedValue({ id: 'decision-1', outcome: 'shipped' })
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({ preset: 'balanced' })
    vi.spyOn(apiClient.twin, 'updateDecisionMirrorConfig').mockResolvedValue({ preset: 'evidence_strict' })
    vi.spyOn(apiClient.twin, 'resetDecisionMirrorConfig').mockResolvedValue({ preset: 'balanced' })

    const store = useCanvasStore()

    await expect(store.listMemoryDigest()).resolves.toEqual([{ id: 'digest-1' }])
    await store.reviewMemoryDigestItem('digest-1', 'keep')
    expect(apiClient.twin.reviewMemoryDigestItem).toHaveBeenCalledWith('digest-1', {
      action: 'keep',
      rationale: null
    })

    await expect(store.listDecisionEpisodes()).resolves.toEqual([{ id: 'decision-1' }])
    await store.updateDecisionOutcome('decision-1', { outcome: 'shipped' })
    expect(apiClient.twin.updateDecisionOutcome).toHaveBeenCalledWith('decision-1', {
      outcome: 'shipped'
    })

    await expect(store.getDecisionMirrorConfig()).resolves.toEqual({ preset: 'balanced' })
    await store.updateDecisionMirrorConfig({ preset: 'evidence_strict' })
    expect(apiClient.twin.updateDecisionMirrorConfig).toHaveBeenCalledWith({
      preset: 'evidence_strict'
    })
    await expect(store.resetDecisionMirrorConfig()).resolves.toEqual({ preset: 'balanced' })
  })

  it('regenerateResponse stores the backend error text on response.error_message', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const regenerateSpy = vi.spyOn(apiClient.canvas, 'regenerateResponse').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'error',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4',
          error: 'OpenRouter request failed: rate limit exceeded'
        }
      })
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'tile-1',
          prompt: 'Hello',
          responses: {
            'openai/gpt-4': {
              status: 'completed',
              content: 'Previous response',
              position: { x: 0, y: 0, width: 280, height: 200 }
            }
          }
        }
      ],
      debates: []
    }

    await store.regenerateResponse('tile-1', 'openai/gpt-4')

    expect(regenerateSpy).toHaveBeenCalledWith('session-1', 'tile-1', 'openai/gpt-4')
    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4']).toMatchObject({
      status: 'error',
      content: '',
      error_message: 'OpenRouter request failed: rate limit exceeded'
    })
  })

  it('regenerateResponse restores the exact completed response when replay is rejected before streaming', async () => {
    vi.spyOn(apiClient.canvas, 'regenerateResponse').mockRejectedValue(
      new Error('Twin History persisted prompt context can no longer be reproduced')
    )
    const previousResponse = {
      id: 'response-1',
      model_id: 'openai/gpt-4',
      model_name: 'GPT-4',
      status: 'completed',
      content: 'Durable prior answer',
      error: null,
      error_message: null,
      cost_usd: 0.003,
      provider: 'openrouter',
      provenance: 'canvas_openrouter',
      position: { x: 0, y: 0, width: 280, height: 200 }
    }
    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [{
        id: 'tile-1',
        prompt: 'Hello',
        responses: { 'openai/gpt-4': { ...previousResponse, position: { ...previousResponse.position } } }
      }],
      debates: []
    }

    await expect(store.regenerateResponse('tile-1', 'openai/gpt-4')).rejects.toThrow(
      'can no longer be reproduced'
    )

    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4']).toEqual(previousResponse)
    expect(store.streamingModels.size).toBe(0)
  })

  it('regenerateResponse rejection does not restore into a different current session', async () => {
    const store = useCanvasStore()
    vi.spyOn(apiClient.canvas, 'regenerateResponse').mockImplementation(async () => {
      store.currentSession = {
        id: 'session-B',
        prompt_tiles: [{
          id: 'tile-1',
          responses: {
            'openai/gpt-4': {
              status: 'completed',
              content: 'Session B answer',
              position: { x: 10, y: 10, width: 280, height: 200 }
            }
          }
        }],
        debates: []
      }
      throw new Error('Twin History replay rejected')
    })
    store.currentSession = {
      id: 'session-A',
      prompt_tiles: [{
        id: 'tile-1',
        responses: {
          'openai/gpt-4': {
            status: 'completed',
            content: 'Session A answer',
            position: { x: 0, y: 0, width: 280, height: 200 }
          }
        }
      }],
      debates: []
    }

    await expect(store.regenerateResponse('tile-1', 'openai/gpt-4')).rejects.toThrow(
      'replay rejected'
    )

    expect(store.currentSession.id).toBe('session-B')
    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4']).toMatchObject({
      status: 'completed',
      content: 'Session B answer'
    })
  })

  it('sendPrompt stores empty model completions as an error response', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'tile_created',
          tile: {
            id: 'tile-1',
            prompt: 'Hello',
            responses: {
              'openai/gpt-4': {
                status: 'pending',
                content: '',
                position: { x: 0, y: 0, width: 280, height: 200 }
              }
            }
          }
        }
      })
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'error',
          tile_id: 'tile-1',
          model_id: 'openai/gpt-4',
          error: 'No response returned from model'
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    await store.sendPrompt('Hello', ['openai/gpt-4'])

    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4']).toMatchObject({
      status: 'error',
      content: '',
      error_message: 'No response returned from model'
    })
  })

  it('does not publish a late session A send failure as session B store error', async () => {
    let rejectSessionA
    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(() => new Promise((resolve, reject) => {
      rejectSessionA = reject
    }))
    const store = useCanvasStore()
    store.currentSession = { id: 'session-A', prompt_tiles: [], debates: [] }

    const pendingSend = store.sendPrompt('Hello from A', ['openai/gpt-4'])
    await flushPromises()
    store.currentSession = { id: 'session-B', prompt_tiles: [], debates: [] }
    rejectSessionA(new Error('Session A provider failed'))

    await expect(pendingSend).rejects.toThrow('Session A provider failed')
    expect(store.currentSession.id).toBe('session-B')
    expect(store.error).toBeNull()
  })

  it('scopes concurrent sendPrompt streams per tile so same-model interleaved chunks do not cross-contaminate', async () => {
    // Simulates the real Tauri behavior: every setupTauriStreamListener() call registers
    // its own listener, and ALL listeners receive every canvas-stream event for the session.
    const handlers = []
    listenMock.mockImplementation(async (_eventName, handler) => {
      handlers.push(handler)
      return unlistenMock
    })

    function broadcast(payload) {
      // Iterate a snapshot since a handler's own unlisten() (invoked from inside a
      // handler-triggered callback) must not mutate the array mid-broadcast.
      handlers.slice().forEach(handler => handler({ payload }))
    }

    let resolveA
    let resolveB
    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async (_sessionId, request) => {
      if (request.prompt === 'Prompt A') {
        return new Promise(resolve => { resolveA = resolve })
      }
      return new Promise(resolve => { resolveB = resolve })
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    const promiseA = store.sendPrompt('Prompt A', ['shared-model'])
    await flushPromises()
    const promiseB = store.sendPrompt('Prompt B', ['shared-model'])
    await flushPromises()

    expect(handlers.length).toBe(2)

    // tile_created is broadcast to BOTH listeners (this is what causes duplicate-tile
    // pushes today if the push isn't deduped by tile id).
    broadcast({
      session_id: 'session-1',
      type: 'tile_created',
      tile: {
        id: 'tile-A',
        prompt: 'Prompt A',
        responses: {
          'shared-model': { status: 'pending', content: '', position: { x: 0, y: 0, width: 280, height: 200 } }
        }
      }
    })
    broadcast({
      session_id: 'session-1',
      type: 'tile_created',
      tile: {
        id: 'tile-B',
        prompt: 'Prompt B',
        responses: {
          'shared-model': { status: 'pending', content: '', position: { x: 0, y: 300, width: 280, height: 200 } }
        }
      }
    })

    resolveA('tile-A')
    resolveB('tile-B')
    await flushPromises()

    // Interleave chunk events for both tiles under the SAME model id.
    broadcast({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-A', model_id: 'shared-model', chunk: 'Hello ' })
    broadcast({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-B', model_id: 'shared-model', chunk: 'World ' })
    broadcast({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-A', model_id: 'shared-model', chunk: 'from A' })
    broadcast({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-B', model_id: 'shared-model', chunk: 'from B' })
    broadcast({ session_id: 'session-1', type: 'complete', tile_id: 'tile-A', model_id: 'shared-model' })
    broadcast({ session_id: 'session-1', type: 'complete', tile_id: 'tile-B', model_id: 'shared-model' })

    await Promise.all([promiseA, promiseB])

    const tiles = store.currentSession.prompt_tiles
    expect(tiles.map(t => t.id).sort()).toEqual(['tile-A', 'tile-B'])

    const tileA = tiles.find(t => t.id === 'tile-A')
    const tileB = tiles.find(t => t.id === 'tile-B')
    expect(tileA.responses['shared-model'].content).toBe('Hello from A')
    expect(tileB.responses['shared-model'].content).toBe('World from B')
    expect(tileA.responses['shared-model'].status).toBe('completed')
    expect(tileB.responses['shared-model'].status).toBe('completed')

    // No streaming keys should remain — single-ownership decrement, no double-decrement leak.
    expect(store.streamingModels.size).toBe(0)
  })

  it('buffers stream events arriving before invoke resolves, then replays own-tile events and drops foreign-tile ones', async () => {
    // Exercises the buffer-replay branch: in production, canvas-stream events and the
    // invoke() promise travel on separate channels, so chunk/complete can arrive BEFORE
    // sendPrompt() resolves with this operation's tile id. Those events must be buffered
    // (not guessed at), then replayed in order once the id is known — and any buffered
    // event belonging to a different tile must be dropped at replay.
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    function emit(payload) {
      streamHandler({ payload })
    }

    let resolveInvoke
    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      return new Promise(resolve => { resolveInvoke = resolve })
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'tile-other',
          prompt: 'Someone else',
          responses: {
            'shared-model': {
              status: 'completed',
              content: 'untouched',
              position: { x: 500, y: 0, width: 280, height: 200 }
            }
          }
        }
      ],
      debates: []
    }

    const promise = store.sendPrompt('Prompt buffered', ['shared-model'])
    await flushPromises()

    // Invoke has NOT resolved yet — every tile-scoped event below lands in the buffer.
    emit({
      session_id: 'session-1',
      type: 'tile_created',
      tile: {
        id: 'tile-own',
        prompt: 'Prompt buffered',
        responses: {
          'shared-model': { status: 'pending', content: '', position: { x: 0, y: 0, width: 280, height: 200 } }
        }
      }
    })
    emit({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-own', model_id: 'shared-model', chunk: 'first ' })
    // Foreign-tile chunk interleaved into the buffer — must be dropped at replay, not
    // appended to this operation's modelContent nor written into tile-other.
    emit({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-other', model_id: 'shared-model', chunk: 'INTRUDER' })
    emit({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-own', model_id: 'shared-model', chunk: 'second' })
    emit({ session_id: 'session-1', type: 'complete', tile_id: 'tile-own', model_id: 'shared-model' })

    // Nothing applied yet: replay only happens after invoke resolves with the tile id.
    const tileOwnBefore = store.currentSession.prompt_tiles.find(t => t.id === 'tile-own')
    expect(tileOwnBefore.responses['shared-model'].content).toBe('')

    resolveInvoke('tile-own')
    // If mark/replay ordering ever regresses (replay before streaming keys are marked,
    // or buffered complete not clearing its key), this await hangs until the test times out.
    await expect(promise).resolves.toBe('tile-own')

    const tileOwn = store.currentSession.prompt_tiles.find(t => t.id === 'tile-own')
    expect(tileOwn.responses['shared-model'].content).toBe('first second')
    expect(tileOwn.responses['shared-model'].status).toBe('completed')

    // Foreign tile untouched — its buffered chunk was dropped, not applied.
    const tileOther = store.currentSession.prompt_tiles.find(t => t.id === 'tile-other')
    expect(tileOther.responses['shared-model'].content).toBe('untouched')
    expect(tileOther.responses['shared-model'].status).toBe('completed')

    // Buffered complete cleared its streaming key — no leak, no timeout wait.
    expect(store.streamingModels.size).toBe(0)
  })

  it('loadSession maps persisted backend errors onto response.error_message', async () => {
    vi.spyOn(apiClient.canvas, 'get').mockResolvedValue({
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'tile-1',
          prompt: 'Hello',
          responses: {
            'openai/gpt-4': {
              status: 'error',
              content: '',
              error: 'No response returned from model',
              position: { x: 0, y: 0, width: 280, height: 200 }
            }
          }
        }
      ],
      debates: []
    })

    const store = useCanvasStore()
    await store.loadSession('session-1')

    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4']).toMatchObject({
      status: 'error',
      error: 'No response returned from model',
      error_message: 'No response returned from model'
    })
  })

  it('does not let an invalidated session load overwrite a newer destination owner', async () => {
    let finishLoad
    vi.spyOn(apiClient.canvas, 'get').mockImplementation(() => new Promise(resolve => {
      finishLoad = () => resolve({
        id: 'stale-canvas',
        prompt_tiles: [],
        debates: [],
      })
    }))
    const store = useCanvasStore()

    const pending = store.loadSession('stale-canvas')
    store.clearSession()
    store.currentSession = {
      id: 'twin-chat',
      tags: ['companion-twin-chat'],
      prompt_tiles: [],
      debates: [],
    }
    finishLoad()
    await pending

    expect(store.currentSession.id).toBe('twin-chat')
  })

  it('does not let an invalidated session create install itself over a newer owner', async () => {
    let finishCreate
    vi.spyOn(apiClient.canvas, 'create').mockImplementation(() => new Promise(resolve => {
      finishCreate = () => resolve({
        id: 'stale-created',
        tags: [],
        prompt_tiles: [],
        debates: [],
      })
    }))
    const store = useCanvasStore()

    const pending = store.createSession({ title: 'Stale create' })
    store.clearSession()
    store.currentSession = {
      id: 'new-owner',
      tags: ['companion-twin-chat'],
      prompt_tiles: [],
      debates: [],
    }
    finishCreate()

    await expect(pending).resolves.toBeNull()
    expect(store.currentSession.id).toBe('new-owner')
    expect(store.sessions.some(session => session.id === 'stale-created')).toBe(false)
    expect(store.loading).toBe(false)
  })

  it('does not let a stale create clear loading owned by a newer session load', async () => {
    let finishCreate
    let finishLoad
    vi.spyOn(apiClient.canvas, 'create').mockImplementation(() => new Promise(resolve => {
      finishCreate = () => resolve({ id: 'stale-created', prompt_tiles: [], debates: [] })
    }))
    vi.spyOn(apiClient.canvas, 'get').mockImplementation(() => new Promise(resolve => {
      finishLoad = () => resolve({ id: 'new-owner', prompt_tiles: [], debates: [] })
    }))
    const store = useCanvasStore()

    const pendingCreate = store.createSession({ title: 'Stale create' })
    const pendingLoad = store.loadSession('new-owner')
    finishCreate()
    await pendingCreate

    expect(store.loading).toBe(true)
    expect(store.currentSession).toBeNull()

    finishLoad()
    await pendingLoad
    expect(store.currentSession.id).toBe('new-owner')
    expect(store.loading).toBe(false)
  })

  it('clears loading when deleting a session invalidates an in-flight session load', async () => {
    let finishLoad
    vi.spyOn(apiClient.canvas, 'get').mockImplementation(() => new Promise(resolve => {
      finishLoad = () => resolve({
        id: 'stale-canvas',
        prompt_tiles: [],
        debates: [],
      })
    }))
    vi.spyOn(apiClient.canvas, 'delete').mockResolvedValue()
    const store = useCanvasStore()
    store.sessions = [{ id: 'deleted-session', prompt_tiles: [], debates: [] }]

    const pending = store.loadSession('stale-canvas')
    expect(store.loading).toBe(true)
    await store.deleteSession('deleted-session')

    expect(store.loading).toBe(false)
    finishLoad()
    await pending
    expect(store.loading).toBe(false)
  })

  it('deleteTile removes the full descendant tree from the current session', async () => {
    vi.spyOn(apiClient.canvas, 'deleteTile').mockResolvedValue()

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        { id: 'root', parent_tile_id: null, responses: {} },
        { id: 'child', parent_tile_id: 'root', responses: {} },
        { id: 'grandchild', parent_tile_id: 'child', responses: {} },
        { id: 'other', parent_tile_id: null, responses: {} }
      ],
      debates: []
    }

    await store.deleteTile('root')

    expect(store.currentSession.prompt_tiles.map(tile => tile.id)).toEqual(['other'])
  })

  it('deleteTile restores the previous canvas tree if the backend delete fails', async () => {
    vi.spyOn(apiClient.canvas, 'deleteTile').mockRejectedValue(new Error('disk write failed'))

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        { id: 'root', parent_tile_id: null, responses: {} },
        { id: 'child', parent_tile_id: 'root', responses: {} },
        { id: 'grandchild', parent_tile_id: 'child', responses: {} },
        { id: 'other', parent_tile_id: null, responses: {} }
      ],
      debates: []
    }

    await expect(store.deleteTile('root')).rejects.toThrow('disk write failed')
    expect(store.currentSession.prompt_tiles.map(tile => tile.id)).toEqual([
      'root',
      'child',
      'grandchild',
      'other'
    ])
  })

  it('deleteTile removes debates that depend on the deleted subtree', async () => {
    vi.spyOn(apiClient.canvas, 'deleteTile').mockResolvedValue()

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        { id: 'root', parent_tile_id: null, responses: {} },
        { id: 'child', parent_tile_id: 'root', parent_model_id: 'model-a', responses: {} },
        { id: 'other', parent_tile_id: null, responses: {} }
      ],
      debates: [
        { id: 'debate-child', source_tile_ids: ['child'], participating_models: ['model-a'] },
        { id: 'debate-other', source_tile_ids: ['other'], participating_models: ['model-b'] }
      ]
    }

    await store.deleteTile('root')

    expect(store.currentSession.debates.map(debate => debate.id)).toEqual(['debate-other'])
  })

  it('deleteResponse removes only the deleted model branch and dependent debates', async () => {
    vi.spyOn(apiClient.canvas, 'deleteResponse').mockResolvedValue()

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'root',
          parent_tile_id: null,
          models: ['model-a', 'model-b'],
          responses: {
            'model-a': {
              status: 'completed',
              content: 'A',
              position: { x: 0, y: 0, width: 280, height: 200 }
            },
            'model-b': {
              status: 'completed',
              content: 'B',
              position: { x: 0, y: 220, width: 280, height: 200 }
            }
          }
        },
        {
          id: 'branch-a',
          parent_tile_id: 'root',
          parent_model_id: 'model-a',
          models: ['model-a'],
          responses: {
            'model-a': {
              status: 'completed',
              content: 'branch-a',
              position: { x: 500, y: 0, width: 280, height: 200 }
            }
          }
        },
        {
          id: 'branch-a-child',
          parent_tile_id: 'branch-a',
          parent_model_id: 'model-a',
          models: ['model-a'],
          responses: {
            'model-a': {
              status: 'completed',
              content: 'branch-a-child',
              position: { x: 1000, y: 0, width: 280, height: 200 }
            }
          }
        },
        {
          id: 'branch-b',
          parent_tile_id: 'root',
          parent_model_id: 'model-b',
          models: ['model-b'],
          responses: {
            'model-b': {
              status: 'completed',
              content: 'branch-b',
              position: { x: 500, y: 400, width: 280, height: 200 }
            }
          }
        }
      ],
      debates: [
        { id: 'debate-a', source_tile_ids: ['root'], participating_models: ['model-a'] },
        { id: 'debate-branch-a', source_tile_ids: ['branch-a'], participating_models: ['model-a'] },
        { id: 'debate-b', source_tile_ids: ['root'], participating_models: ['model-b'] }
      ]
    }

    await store.deleteResponse('root', 'model-a')

    expect(apiClient.canvas.deleteResponse).toHaveBeenCalledWith('session-1', 'root', 'model-a')
    expect(store.currentSession.prompt_tiles.map(tile => tile.id)).toEqual(['root', 'branch-b'])
    expect(store.currentSession.prompt_tiles[0].models).toEqual(['model-b'])
    expect(store.currentSession.prompt_tiles[0].responses['model-a']).toBeUndefined()
    expect(store.currentSession.debates.map(debate => debate.id)).toEqual(['debate-b'])
  })

  it('recordPreferenceFeedback stores explicit canvas feedback against the current session', async () => {
    const feedbackSpy = vi.spyOn(apiClient.twin, 'recordCanvasFeedback').mockResolvedValue({
      trace_event_id: 'evt-1',
      created_record_ids: ['rec-1']
    })

    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: []
    }

    const result = await store.recordPreferenceFeedback('tile-1', 'model-a', 'accept', 'This answer matches me')

    expect(feedbackSpy).toHaveBeenCalledWith('session-1', {
      feedback_type: 'accept',
      response: {
        tile_id: 'tile-1',
        model_id: 'model-a'
      },
      rationale: 'This answer matches me',
      content: null
    })
    expect(result.created_record_ids).toEqual(['rec-1'])
  })

  it('deduplicates in-flight feedback by session, tile, and model', async () => {
    let finishFeedback
    const feedbackSpy = vi.spyOn(apiClient.twin, 'recordCanvasFeedback').mockImplementation(
      () => new Promise(resolve => { finishFeedback = resolve })
    )
    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: [],
    }

    const first = store.recordPreferenceFeedback('tile-1', 'model-a', 'accept')
    const duplicate = store.recordPreferenceFeedback('tile-1', 'model-a', 'reject')

    expect(feedbackSpy).toHaveBeenCalledOnce()
    expect(store.feedbackInFlight.has('session-1:tile-1:model-a')).toBe(true)
    await expect(duplicate).resolves.toBeNull()

    finishFeedback({ trace_event_id: 'evt-1', created_record_ids: [] })
    await first
    expect(store.feedbackInFlight.size).toBe(0)
  })

  it('passes the frozen response witness through explicit insight capture', async () => {
    const feedbackSpy = vi.spyOn(apiClient.twin, 'recordCanvasFeedback').mockResolvedValue({
      trace_event_id: 'evt-1',
      created_record_ids: ['rec-1']
    })
    const store = useCanvasStore()
    store.currentSession = {
      id: 'session-1',
      prompt_tiles: [],
      debates: [],
    }

    await store.captureInsight('preference', 'Concrete details', {
      response: { tile_id: 'tile-1', model_id: 'model-a' },
      responseWitness: {
        response_id: 'response-a',
        response_content: 'The exact visible answer',
      },
    })

    expect(feedbackSpy).toHaveBeenCalledWith('session-1', {
      feedback_type: 'insight',
      kind: 'preference',
      content: 'Concrete details',
      rationale: null,
      response: { tile_id: 'tile-1', model_id: 'model-a' },
      response_witness: {
        response_id: 'response-a',
        response_content: 'The exact visible answer',
      },
      confidence: 0.8,
    })
  })

  it('session_saved reconciles silently mid-stream: no loading flash, no clobbered stream content, no reverted drag', async () => {
    const handlers = []
    listenMock.mockImplementation(async (_eventName, handler) => {
      handlers.push(handler)
      return unlistenMock
    })
    function broadcast(payload) {
      handlers.slice().forEach(handler => handler({ payload }))
    }

    let resolveGet
    vi.spyOn(apiClient.canvas, 'get').mockImplementation(() => new Promise(resolve => { resolveGet = resolve }))
    vi.spyOn(apiClient.canvas, 'updateTilePosition').mockResolvedValue()

    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      broadcast({
        session_id: 'session-1',
        type: 'tile_created',
        tile: {
          id: 'tile-1',
          prompt: 'Hello',
          position: { x: 0, y: 0, width: 200, height: 120 },
          responses: {
            'openai/gpt-4': { status: 'pending', content: '', position: { x: 300, y: 0, width: 280, height: 200 } }
          }
        }
      })
      return 'tile-1'
    })

    const store = useCanvasStore()
    store.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }

    const promise = store.sendPrompt('Hello', ['openai/gpt-4'])
    await flushPromises()

    // Partial content streams in
    broadcast({ session_id: 'session-1', type: 'chunk', tile_id: 'tile-1', model_id: 'openai/gpt-4', chunk: 'Partial answer' })

    // User drags the tile locally, inside the ~150ms position-debounce window
    store.updateTilePosition('tile-1', { x: 999, y: 999 })

    // Backend fires session_saved mid-stream (model is still marked streaming, not complete)
    broadcast({ session_id: 'session-1', type: 'session_saved' })
    await flushPromises()

    // The reconciliation fetch is still in flight (resolveGet not called yet) — loading
    // must never flip true, unlike the old wholesale loadSession() behavior.
    expect(store.loading).toBe(false)

    // Server's disk snapshot predates the chunk/drag above (stale content + stale position)
    resolveGet({
      id: 'session-1',
      prompt_tiles: [
        {
          id: 'tile-1',
          prompt: 'Hello',
          position: { x: 0, y: 0, width: 200, height: 120 },
          responses: {
            'openai/gpt-4': { status: 'pending', content: '', position: { x: 300, y: 0, width: 280, height: 200 } }
          }
        }
      ],
      debates: []
    })
    await flushPromises()

    expect(store.loading).toBe(false)
    expect(store.currentSession.prompt_tiles[0].responses['openai/gpt-4'].content).toBe('Partial answer')
    expect(store.currentSession.prompt_tiles[0].position).toMatchObject({ x: 999, y: 999 })

    // Finish the stream normally
    broadcast({ session_id: 'session-1', type: 'complete', tile_id: 'tile-1', model_id: 'openai/gpt-4' })
    await promise

    expect(store.streamingModels.size).toBe(0)
  })

  it('drops a stale session_saved reconciliation when the user has switched to another session', async () => {
    const handlers = []
    listenMock.mockImplementation(async (_eventName, handler) => {
      handlers.push(handler)
      return unlistenMock
    })
    function broadcast(payload) {
      handlers.slice().forEach(handler => handler({ payload }))
    }

    // The reconciliation fetch returns session A (the session whose stream finished in
    // the background) — it must NOT be applied once the user is viewing session B.
    vi.spyOn(apiClient.canvas, 'get').mockResolvedValue({
      id: 'session-A',
      prompt_tiles: [
        {
          id: 'tile-A1',
          prompt: 'Background prompt',
          position: { x: 0, y: 0, width: 200, height: 120 },
          responses: {
            'openai/gpt-4': { status: 'completed', content: 'A answer', position: { x: 300, y: 0, width: 280, height: 200 } }
          }
        }
      ],
      debates: []
    })

    vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async () => {
      broadcast({
        session_id: 'session-A',
        type: 'tile_created',
        tile: {
          id: 'tile-A1',
          prompt: 'Background prompt',
          position: { x: 0, y: 0, width: 200, height: 120 },
          responses: {
            'openai/gpt-4': { status: 'pending', content: '', position: { x: 300, y: 0, width: 280, height: 200 } }
          }
        }
      })
      return 'tile-A1'
    })

    const store = useCanvasStore()
    store.currentSession = { id: 'session-A', prompt_tiles: [], debates: [] }

    const promise = store.sendPrompt('Background prompt', ['openai/gpt-4'])
    await flushPromises()

    // User switches to session B while A's stream is still in flight
    store.currentSession = {
      id: 'session-B',
      prompt_tiles: [
        {
          id: 'tile-B1',
          prompt: 'B prompt',
          position: { x: 10, y: 10, width: 200, height: 120 },
          responses: {}
        }
      ],
      debates: []
    }

    // A's backend save lands — its session_saved must be dropped, not rendered under B
    broadcast({ session_id: 'session-A', type: 'session_saved' })
    await flushPromises()

    expect(store.currentSession.id).toBe('session-B')
    expect(store.currentSession.prompt_tiles.map(t => t.id)).toEqual(['tile-B1'])

    // Let A's stream finish so the operation resolves cleanly
    broadcast({ session_id: 'session-A', type: 'complete', tile_id: 'tile-A1', model_id: 'openai/gpt-4' })
    await promise
    expect(store.streamingModels.size).toBe(0)
  })

  it('drops late tile and response mutations after switching sessions but clears the stream tracker', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })
    vi.spyOn(apiClient.canvas, 'sendPrompt').mockResolvedValue('tile-A1')
    const store = useCanvasStore()
    store.currentSession = { id: 'session-A', prompt_tiles: [], debates: [] }

    const promise = store.sendCompanionPrompt({
      prompt: 'Background Twin prompt',
      modelId: 'openai/gpt-4'
    })
    await flushPromises()
    store.currentSession = {
      id: 'session-B',
      prompt_tiles: [{ id: 'tile-B1', prompt: 'Foreground', responses: {} }],
      debates: []
    }

    streamHandler({ payload: {
      session_id: 'session-A',
      type: 'tile_created',
      tile: {
        id: 'tile-A1',
        prompt: 'Background Twin prompt',
        responses: {
          'openai/gpt-4': { status: 'pending', content: '', position: { x: 0, y: 0, width: 280, height: 200 } }
        }
      }
    } })
    streamHandler({ payload: {
      session_id: 'session-A',
      type: 'chunk',
      tile_id: 'tile-A1',
      model_id: 'openai/gpt-4',
      chunk: 'must not appear'
    } })
    streamHandler({ payload: {
      session_id: 'session-A',
      type: 'complete',
      tile_id: 'tile-A1',
      model_id: 'openai/gpt-4'
    } })

    await promise

    expect(store.currentSession.id).toBe('session-B')
    expect(store.currentSession.prompt_tiles.map(tile => tile.id)).toEqual(['tile-B1'])
    expect(store.streamingModels.size).toBe(0)
  })

  it('exportTwinData proxies export requests to the twin API', async () => {
    const exportSpy = vi.spyOn(apiClient.twin, 'exportData').mockResolvedValue({
      train: { count: 3 },
      eval: { count: 1 },
      holdout: { count: 1 }
    })

    const store = useCanvasStore()
    const result = await store.exportTwinData({ eval_percentage: 20 })

    expect(exportSpy).toHaveBeenCalledWith({ eval_percentage: 20 })
    expect(result.eval.count).toBe(1)
  })
})
