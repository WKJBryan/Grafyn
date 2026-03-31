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

  it('thinkHarderFromResponse creates a same-model full-history request with deeper defaults', async () => {
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
      max_tokens: 4096,
      web_search: true,
      web_search_max_results: THINK_HARDER_WEB_SEARCH_MAX_RESULTS,
      system_prompt: expect.stringContaining('Verify factual claims')
    }))
  })

  it('sendPrompt omits max_tokens for normal prompts', async () => {
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

    await store.sendPrompt('Hello', ['openai/gpt-4'])

    expect(sendPromptSpy).toHaveBeenCalledWith('session-1', expect.objectContaining({
      prompt: 'Hello',
      models: ['openai/gpt-4'],
      temperature: 0.7,
      context_mode: 'knowledge_search',
      web_search: false,
      web_search_max_results: 5
    }))
    expect(sendPromptSpy.mock.calls[0][1]).not.toHaveProperty('max_tokens')
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
})
