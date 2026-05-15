import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useCanvasStore, THINK_HARDER_PROMPT, THINK_HARDER_WEB_SEARCH_MAX_RESULTS } from '@/stores/canvas'
import * as apiClient from '@/api/client'

const listenMock = vi.fn()
const unlistenMock = vi.fn()

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

describe('Canvas Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    listenMock.mockResolvedValue(unlistenMock)
  })

  it('thinkHarderFromResponse creates a same-model full-history request with deeper defaults and no max token cap', async () => {
    let streamHandler
    listenMock.mockImplementation(async (_eventName, handler) => {
      streamHandler = handler
      return unlistenMock
    })

    const sendPromptSpy = vi.spyOn(apiClient.canvas, 'sendPrompt').mockImplementation(async (_sessionId, request) => {
      streamHandler({
        payload: {
          session_id: 'session-1',
          type: 'complete',
          tile_id: 'child-tile',
          model_id: 'openai/gpt-4'
        }
      })
      return `request:${request.prompt}`
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
