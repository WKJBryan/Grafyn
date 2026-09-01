import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import TwinChat from '@/components/companion/TwinChat.vue'
import { useCanvasStore } from '@/stores/canvas'
import { useTwinStore } from '@/stores/twin'
import * as apiClient from '@/api/client'

function response(modelId, content = 'Answer') {
  return {
    id: `response-${modelId}`,
    model_id: modelId,
    model_name: modelId,
    content,
    status: 'completed',
    created_at: '2026-09-01T00:00:00Z',
  }
}

function chatSession(tiles = []) {
  return {
    id: 'twin-chat-session',
    title: 'Twin Advisor',
    tags: ['companion-twin-chat'],
    prompt_tiles: tiles,
    debates: [],
  }
}

const globalVariant = { relationships: [] }
const alexVariant = {
  relationships: [{
    subject_id: 'owner',
    predicate: 'with',
    object_id: 'person-alex',
    direction: 'directed',
  }],
}
const bobVariant = {
  relationships: [{
    subject_id: 'owner',
    predicate: 'with',
    object_id: 'person-bob',
    direction: 'directed',
  }],
}

describe('TwinChat', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.restoreAllMocks()
  })

  it('does not create a persisted chat session merely by mounting', async () => {
    const canvas = useCanvasStore()
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const createSpy = vi.spyOn(canvas, 'createSession').mockResolvedValue(chatSession())

    mount(TwinChat)
    await flushPromises()

    expect(createSpy).not.toHaveBeenCalled()
  })

  it('makes the global chat boundary explicit even when the page filter shows all relationships', async () => {
    const canvas = useCanvasStore()
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()

    const wrapper = mount(TwinChat)
    await flushPromises()

    expect(wrapper.get('[data-chat-context]').text())
      .toContain('Global context — relationship-specific memory excluded')
  })

  it('fails closed instead of creating a duplicate when session discovery fails', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(canvas, 'loadSessions').mockImplementation(async () => {
      canvas.error = 'Session discovery failed'
    })
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const createSpy = vi.spyOn(canvas, 'createSession').mockResolvedValue(chatSession())
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('wrong-tile')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Do not duplicate my chat')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(createSpy).not.toHaveBeenCalled()
    expect(sendSpy).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session discovery failed')
  })

  it('reopens the latest tagged Twin chat on mount without rendering an ordinary Canvas session', async () => {
    const canvas = useCanvasStore()
    canvas.currentSession = {
      ...chatSession([{
        id: 'canvas-tile',
        prompt: 'Ordinary Canvas content',
        models: [],
        responses: {},
      }]),
      id: 'canvas-session',
      tags: [],
    }
    canvas.sessions = [
      { ...chatSession(), id: 'older-chat', updated_at: '2026-08-31T01:00:00Z' },
      { ...chatSession(), id: 'latest-chat', updated_at: '2026-09-01T01:00:00Z' },
    ]
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const loadSession = vi.spyOn(canvas, 'loadSession').mockImplementation(async id => {
      canvas.currentSession = {
        ...chatSession([{
          id: 'persisted-tile',
          prompt: 'Persisted Twin history',
          models: ['model-a'],
          responses: { 'model-a': response('model-a', 'Remembered answer') },
        }]),
        id,
      }
    })

    const wrapper = mount(TwinChat)
    expect(wrapper.text()).not.toContain('Ordinary Canvas content')
    await flushPromises()

    expect(loadSession).toHaveBeenCalledWith('latest-chat')
    expect(wrapper.text()).toContain('Persisted Twin history')
    expect(wrapper.text()).toContain('Remembered answer')
  })

  it('invalidates a pending Twin history load when chat unmounts', async () => {
    const canvas = useCanvasStore()
    canvas.currentSession = { ...chatSession(), id: 'canvas-session', tags: [] }
    canvas.sessions = [{ ...chatSession(), id: 'latest-chat', updated_at: '2026-09-01T01:00:00Z' }]
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const loadSession = vi.spyOn(canvas, 'loadSession').mockImplementation(() => new Promise(() => {}))
    const clearSession = vi.spyOn(canvas, 'clearSession')

    const wrapper = mount(TwinChat)
    await flushPromises()
    expect(loadSession).toHaveBeenCalledWith('latest-chat')

    wrapper.unmount()

    expect(clearSession).toHaveBeenCalledOnce()
  })

  it('sends Advisor history with the exact latest parent tile and model ids', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'anthropic' }]
    canvas.currentSession = chatSession([
      {
        id: 'tile-b',
        prompt: 'Later by id',
        models: ['model-b'],
        created_at: '2026-09-01T01:00:00Z',
        responses: { 'model-b': response('model-b') },
      },
      {
        id: 'tile-a',
        prompt: 'Earlier by id',
        models: ['model-a'],
        created_at: '2026-09-01T01:00:00Z',
        responses: { 'model-a': response('model-a') },
      },
    ])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('What should I do next?')
    await wrapper.get('form').trigger('submit')

    expect(sendSpy).toHaveBeenCalledWith({
      prompt: 'What should I do next?',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'twin',
      answerMode: 'advisor',
      relationshipVariant: globalVariant,
      parentTileId: 'tile-b',
      parentModelId: 'model-b',
    })
  })

  it('skips newer pending, error, and empty responses when choosing the automatic parent', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-send', name: 'Send model', provider: 'openrouter' }]
    canvas.currentSession = chatSession([
      {
        id: 'tile-completed',
        prompt: 'Usable history',
        models: ['model-completed'],
        created_at: '2026-09-01T00:00:00Z',
        responses: { 'model-completed': response('model-completed', 'Usable answer') },
      },
      {
        id: 'tile-pending',
        prompt: 'Pending history',
        models: ['model-pending'],
        created_at: '2026-09-01T01:00:00Z',
        responses: {
          'model-pending': { ...response('model-pending', 'Partial answer'), status: 'streaming' },
        },
      },
      {
        id: 'tile-error',
        prompt: 'Failed history',
        models: ['model-error'],
        created_at: '2026-09-01T02:00:00Z',
        responses: { 'model-error': { ...response('model-error'), status: 'error' } },
      },
      {
        id: 'tile-empty',
        prompt: 'Empty history',
        models: ['model-empty'],
        created_at: '2026-09-01T03:00:00Z',
        responses: { 'model-empty': response('model-empty', '  ') },
      },
    ])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Continue from usable history')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({
      parentTileId: 'tile-completed',
      parentModelId: 'model-completed',
    }))
  })

  it('sends without an automatic parent when persisted history has no completed usable response', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-send', name: 'Send model', provider: 'openrouter' }]
    canvas.currentSession = chatSession([
      {
        id: 'tile-pending',
        prompt: 'Pending history',
        models: ['model-pending'],
        created_at: '2026-09-01T00:00:00Z',
        responses: { 'model-pending': { ...response('model-pending'), status: 'pending' } },
      },
      {
        id: 'tile-error',
        prompt: 'Failed history',
        models: ['model-error'],
        created_at: '2026-09-01T01:00:00Z',
        responses: { 'model-error': { ...response('model-error'), status: 'error' } },
      },
    ])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Start without failed history')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({
      parentTileId: null,
      parentModelId: null,
    }))
  })

  it('shows and parents only turns from the selected persisted relationship context', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-send', name: 'Send model', provider: 'openrouter' }]
    canvas.currentSession = chatSession([
      {
        id: 'tile-global',
        prompt: 'Global history',
        models: ['model-global'],
        created_at: '2026-09-01T00:00:00Z',
        context_mode: 'twin_history',
        twin_relationship_variant: globalVariant,
        responses: { 'model-global': response('model-global') },
      },
      {
        id: 'tile-alex',
        prompt: 'Alex history',
        models: ['model-alex'],
        created_at: '2026-09-01T01:00:00Z',
        context_mode: 'twin_history',
        twin_relationship_variant: alexVariant,
        responses: { 'model-alex': response('model-alex') },
      },
      {
        id: 'tile-bob',
        prompt: 'Bob history',
        models: ['model-bob'],
        created_at: '2026-09-01T02:00:00Z',
        context_mode: 'twin_history',
        twin_relationship_variant: bobVariant,
        responses: { 'model-bob': response('model-bob') },
      },
    ])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')

    const wrapper = mount(TwinChat, { props: { relationshipVariant: alexVariant } })
    await flushPromises()

    expect(wrapper.text()).toContain('Alex history')
    expect(wrapper.get('[data-chat-context]').text()).toContain('owner with person-alex [directed]')
    expect(wrapper.text()).toContain('Context: owner with person-alex [directed]')
    expect(wrapper.text()).not.toContain('Global history')
    expect(wrapper.text()).not.toContain('Bob history')

    await wrapper.get('[aria-label="Follow up on tile-alex model-alex"]').trigger('click')
    expect(wrapper.text()).toContain('Following model-alex')
    await wrapper.setProps({ relationshipVariant: bobVariant })

    expect(wrapper.text()).not.toContain('Following model-alex')
    expect(wrapper.text()).toContain('Bob history')
    expect(wrapper.text()).toContain('Context: owner with person-bob')
    expect(wrapper.text()).not.toContain('Alex history')

    await wrapper.get('[aria-label="Twin message"]').setValue('Use only Bob context')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({
      relationshipVariant: bobVariant,
      parentTileId: 'tile-bob',
      parentModelId: 'model-bob',
    }))
  })

  it('freezes relationship context and parent while async session discovery completes', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-send', name: 'Send model', provider: 'openrouter' }]
    canvas.currentSession = chatSession([
      {
        id: 'tile-alex',
        prompt: 'Alex history',
        models: ['model-alex'],
        created_at: '2026-09-01T01:00:00Z',
        twin_relationship_variant: alexVariant,
        responses: { 'model-alex': response('model-alex') },
      },
      {
        id: 'tile-bob',
        prompt: 'Bob history',
        models: ['model-bob'],
        created_at: '2026-09-01T02:00:00Z',
        twin_relationship_variant: bobVariant,
        responses: { 'model-bob': response('model-bob') },
      },
    ])
    let finishDiscovery
    vi.spyOn(canvas, 'loadSessions').mockImplementation(() => new Promise(resolve => {
      finishDiscovery = resolve
    }))
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')
    const wrapper = mount(TwinChat, { props: { relationshipVariant: alexVariant } })

    await wrapper.get('[aria-label="Twin message"]').setValue('Freeze Alex')
    await wrapper.get('form').trigger('submit')
    await wrapper.setProps({ relationshipVariant: bobVariant })
    finishDiscovery()
    await flushPromises()

    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({
      relationshipVariant: alexVariant,
      parentTileId: 'tile-alex',
      parentModelId: 'model-alex',
    }))
    expect(wrapper.text()).not.toContain('Following model-send')
  })

  it('fails closed when a persisted tile and evidence snapshot disagree on relationship context', async () => {
    const canvas = useCanvasStore()
    canvas.currentSession = chatSession([
      {
        id: 'tile-mismatch',
        prompt: 'Do not expose this mismatched turn',
        models: ['model-a'],
        created_at: '2026-09-01T01:00:00Z',
        twin_relationship_variant: globalVariant,
        twin_evidence_snapshot: { twin_relationship_variant: alexVariant },
        responses: { 'model-a': response('model-a') },
      },
      {
        id: 'tile-valid',
        prompt: 'Valid global turn',
        models: ['model-a'],
        created_at: '2026-09-01T02:00:00Z',
        twin_relationship_variant: globalVariant,
        twin_evidence_snapshot: { twin_relationship_variant: globalVariant },
        responses: { 'model-a': response('model-a') },
      },
    ])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()

    const wrapper = mount(TwinChat)
    await flushPromises()

    expect(wrapper.text()).toContain('Valid global turn')
    expect(wrapper.text()).not.toContain('Do not expose this mismatched turn')
  })

  it('freezes the submitted model and reply parent across an async history load', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [
      { id: 'model-a', name: 'Model A', provider: 'openrouter' },
      { id: 'model-b', name: 'Model B', provider: 'openrouter' },
    ]
    canvas.sessions = [{ ...chatSession(), updated_at: '2026-09-01T01:00:00Z' }]
    canvas.currentSession = { ...chatSession(), id: 'unrelated', tags: [] }
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    let finishHistoryLoad
    vi.spyOn(canvas, 'loadSession').mockImplementation(() => new Promise(resolve => {
      finishHistoryLoad = () => {
        canvas.currentSession = chatSession()
        resolve()
      }
    }))
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-next')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Keep the submitted model')
    await wrapper.get('form').trigger('submit')
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[aria-label="Twin model"]').attributes('disabled')).toBeDefined()
    await wrapper.get('[aria-label="Twin model"]').setValue('model-b')
    finishHistoryLoad()
    await flushPromises()

    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({ modelId: 'model-a' }))
    expect(wrapper.text()).toContain('Following model-a')
  })

  it('fails closed instead of sending into an unrelated session when chat history cannot load', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    canvas.sessions = [{
      ...chatSession(),
      updated_at: '2026-09-01T01:00:00Z',
    }]
    canvas.currentSession = {
      ...chatSession(),
      id: 'unrelated-session',
      tags: [],
    }
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    vi.spyOn(canvas, 'loadSession').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('wrong-tile')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Keep this in Twin chat')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(sendSpy).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toMatch(/could not be loaded/i)
  })

  it('fails closed when Twin chat creation resolves to an ordinary Canvas session', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    canvas.currentSession = { ...chatSession(), id: 'canvas-a', tags: [] }
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    vi.spyOn(canvas, 'createSession').mockImplementation(async () => {
      canvas.currentSession = { ...chatSession(), id: 'canvas-b', tags: [] }
      return canvas.currentSession
    })
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('wrong-tile')

    const wrapper = mount(TwinChat)
    await wrapper.get('[aria-label="Twin message"]').setValue('Keep creation in Twin chat')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(sendSpy).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toMatch(/could not be loaded/i)
  })

  it('gates Simulation on Twin identity and keeps its disclosure visible', async () => {
    const canvas = useCanvasStore()
    const twin = useTwinStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    canvas.currentSession = chatSession()
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const sendSpy = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-simulation')

    const wrapper = mount(TwinChat)
    const simulationOption = wrapper.get('[aria-label="Twin answer mode"] option[value="simulation"]')
    expect(simulationOption.attributes('disabled')).toBeDefined()

    twin.setupDraft.twin_name = 'Bryan Twin'
    twin.setupDraft.twin_role = 'thinking companion'
    await wrapper.get('[aria-label="Twin answer mode"]').setValue('simulation')

    expect(wrapper.get('[role="note"]').text()).toMatch(/configured simulation/i)
    await wrapper.get('[aria-label="Twin message"]').setValue('Respond as configured')
    await wrapper.get('form').trigger('submit')
    expect(sendSpy).toHaveBeenCalledWith(expect.objectContaining({ answerMode: 'simulation' }))
  })

  it('renders the persisted evidence snapshot attached to a Twin answer', () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A' }]
    canvas.currentSession = chatSession([{
      id: 'tile-evidence',
      prompt: 'Use my reviewed memory',
      models: ['model-a'],
      created_at: '2026-09-01T00:00:00Z',
      responses: { 'model-a': response('model-a', 'Grounded answer') },
      twin_evidence_snapshot: {
        projection_snapshot_id: 'a'.repeat(64),
        reference_time: '2026-09-01T00:00:00Z',
        evidence_event_ids: ['b'.repeat(64)],
        note_ids: ['note-a'],
      },
    }])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()

    const wrapper = mount(TwinChat)

    expect(wrapper.text()).toContain('Evidence snapshot')
    expect(wrapper.text()).toContain('1 events')
    expect(wrapper.text()).toContain('1 note')
  })

  it('keeps a persisted Simulation disclosure visible after the selector returns to Advisor', () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A' }]
    canvas.currentSession = chatSession([{
      id: 'tile-simulation',
      prompt: 'Simulate from reviewed memory',
      models: ['model-a'],
      created_at: '2026-09-01T00:00:00Z',
      context_mode: 'twin_history',
      twin_answer_mode: 'simulation',
      responses: { 'model-a': response('model-a', 'Simulated answer') },
    }])
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()

    const wrapper = mount(TwinChat)

    expect(wrapper.get('[data-simulation-disclosure="tile-simulation"]').text())
      .toMatch(/configured simulation/i)
    expect(wrapper.get('[aria-label="Twin answer mode"]').element.value).toBe('advisor')
  })

  it('surfaces model-loading and streaming errors owned by the Canvas store', () => {
    const canvas = useCanvasStore()
    canvas.error = 'Provider stream failed'
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()

    const wrapper = mount(TwinChat)

    expect(wrapper.get('[role="alert"]').text()).toContain('Provider stream failed')
  })

  it('invalidates a pending Twin chat create when the chat unmounts', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    let finishCreate
    vi.spyOn(apiClient.canvas, 'create').mockImplementation(() => new Promise(resolve => {
      finishCreate = () => resolve(chatSession())
    }))
    const send = vi.spyOn(canvas, 'sendCompanionPrompt').mockResolvedValue('tile-new')
    const wrapper = mount(TwinChat)
    await flushPromises()

    await wrapper.get('[aria-label="Twin message"]').setValue('Do not outlive Twin chat')
    await wrapper.get('form').trigger('submit')
    await wrapper.vm.$nextTick()
    wrapper.unmount()
    finishCreate()
    await flushPromises()

    expect(canvas.currentSession).toBeNull()
    expect(send).not.toHaveBeenCalled()
  })

  it('preserves text edited while an earlier Twin send is pending', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    canvas.currentSession = chatSession()
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    let finishSend
    vi.spyOn(canvas, 'sendCompanionPrompt').mockImplementation(() => new Promise(resolve => {
      finishSend = () => resolve('tile-new')
    }))
    const wrapper = mount(TwinChat)
    await flushPromises()

    const message = wrapper.get('[aria-label="Twin message"]')
    await message.setValue('Submitted draft')
    await wrapper.get('form').trigger('submit')
    await message.setValue('New draft while waiting')
    finishSend()
    await flushPromises()

    expect(message.element.value).toBe('New draft while waiting')
  })

  it('does not install a reply target from session A after ownership moves to session B', async () => {
    const canvas = useCanvasStore()
    canvas.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    canvas.currentSession = chatSession()
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    let finishSend
    vi.spyOn(canvas, 'sendCompanionPrompt').mockImplementation(() => new Promise(resolve => {
      finishSend = () => resolve('tile-from-a')
    }))
    const wrapper = mount(TwinChat)
    await flushPromises()

    await wrapper.get('[aria-label="Twin message"]').setValue('Answer in A')
    await wrapper.get('form').trigger('submit')
    canvas.currentSession = { ...chatSession(), id: 'twin-chat-session-b' }
    finishSend()
    await flushPromises()

    expect(wrapper.text()).not.toContain('Following model-a')
    expect(wrapper.get('[aria-label="Twin message"]').element.value).toBe('Answer in A')
  })

  it('wires tagged-session feedback ownership into both Twin response choices', async () => {
    const canvas = useCanvasStore()
    canvas.currentSession = chatSession([{
      id: 'tile-a',
      prompt: 'Question',
      models: ['model-a'],
      created_at: '2026-09-01T00:00:00Z',
      responses: { 'model-a': response('model-a') },
    }])
    canvas.feedbackInFlight.add('twin-chat-session:tile-a:model-a')
    vi.spyOn(canvas, 'loadSessions').mockResolvedValue()
    vi.spyOn(canvas, 'loadModels').mockResolvedValue()
    const wrapper = mount(TwinChat)
    await flushPromises()

    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Reject tile-a model-a"]').attributes('disabled')).toBeDefined()
  })
})
