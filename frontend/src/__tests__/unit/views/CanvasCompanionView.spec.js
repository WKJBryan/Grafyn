import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { shallowReactive } from 'vue'
import { createMemoryHistory, createRouter, routeLocationKey, routerKey } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CanvasCompanionView from '@/views/companion/CanvasCompanionView.vue'
import CanvasComposer from '@/components/companion/CanvasComposer.vue'
import CanvasSessionSheet from '@/components/companion/CanvasSessionSheet.vue'
import LinearCanvasThread from '@/components/companion/LinearCanvasThread.vue'
import TwinPreferenceCaptureDialog from '@/components/companion/TwinPreferenceCaptureDialog.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import { useCanvasStore } from '@/stores/canvas'
import * as apiClient from '@/api/client'

function session(id = 'session-a') {
  return {
    id,
    title: 'Thread A',
    tags: [],
    prompt_tiles: [],
    debates: [],
    updated_at: '2026-09-01T00:00:00Z',
  }
}

async function mountView(path = '/canvas/session-a') {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/canvas', component: { template: '<div />' } },
      { path: '/canvas/:id', component: { template: '<div />' } },
    ],
  })
  await router.push(path)
  await router.isReady()
  return {
    router,
    wrapper: mount(CanvasCompanionView, {
      global: {
        plugins: [router],
        stubs: {
          CompanionShell: {
            template: '<div data-test="inner-companion-shell"><slot /></div>',
          },
          QuickImageComposer: {
            props: ['collapsible'],
            template: '<section class="quick-image-stub" :data-collapsible="String(collapsible)" />',
          },
        },
      },
    }),
  }
}

describe('CanvasCompanionView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.restoreAllMocks()
  })

  it('leaves the single compact shell owned by App', async () => {
    const store = useCanvasStore()
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.find('[data-test="inner-companion-shell"]').exists()).toBe(false)
  })

  it('mounts compact image creation beside the existing text composer without changing its models', async () => {
    const store = useCanvasStore()
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    const textModels = store.availableModels
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.get('.quick-image-stub').attributes('data-collapsible')).toBeDefined()
    expect(wrapper.getComponent(CanvasComposer).props('models')).toBe(textModels)
  })

  it('loads the routed session and model catalog through the existing Canvas store', async () => {
    const store = useCanvasStore()
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const loadSession = vi.spyOn(store, 'loadSession').mockResolvedValue()

    await mountView()
    await flushPromises()

    expect(store.loadSessions).toHaveBeenCalledOnce()
    expect(store.loadModels).toHaveBeenCalledOnce()
    expect(loadSession).toHaveBeenCalledWith('session-a')
  })

  it('sends one-model follow-ups without duplicating persistence or streaming', async () => {
    const store = useCanvasStore()
    store.currentSession = session()
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const send = vi.spyOn(store, 'sendCompanionPrompt').mockResolvedValue('tile-new')
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    wrapper.getComponent(CanvasComposer).vm.$emit('submit', {
      prompt: 'Use my notes',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'knowledge',
      parentTileId: 'tile-parent',
      parentModelId: 'model-parent',
    })
    await flushPromises()

    expect(send).toHaveBeenCalledWith({
      prompt: 'Use my notes',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'knowledge',
      parentTileId: 'tile-parent',
      parentModelId: 'model-parent',
    })
  })

  it('waits for the exact routed thread before a fast send after navigation', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishRouteLoad
    vi.spyOn(store, 'loadSession').mockImplementation(id => new Promise(resolve => {
      finishRouteLoad = () => {
        store.currentSession = session(id)
        resolve()
      }
    }))
    const invokedSessions = []
    const send = vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(async () => {
      invokedSessions.push(store.currentSession?.id)
      return 'tile-new'
    })
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await router.push('/canvas/session-b')
    await flushPromises()
    expect(wrapper.getComponent(CanvasComposer).props('busy')).toBe(true)

    wrapper.getComponent(CanvasComposer).vm.$emit('submit', {
      prompt: 'Stay in the routed thread',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'plain',
      parentTileId: null,
      parentModelId: null,
    })
    await flushPromises()
    expect(send).not.toHaveBeenCalled()

    finishRouteLoad()
    await flushPromises()

    expect(invokedSessions).toEqual(['session-b'])
  })

  it('serializes route loads so a stale result cannot overwrite the newest thread', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const finishLoads = new Map()
    const loadSession = vi.spyOn(store, 'loadSession').mockImplementation(id => new Promise(resolve => {
      finishLoads.set(id, () => {
        store.currentSession = session(id)
        resolve()
      })
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await router.push('/canvas/session-b')
    await flushPromises()
    await router.push('/canvas/session-c')
    await flushPromises()

    expect(loadSession.mock.calls.map(([id]) => id)).toEqual(['session-b'])
    finishLoads.get('session-b')()
    await flushPromises()
    expect(loadSession.mock.calls.map(([id]) => id)).toEqual(['session-b', 'session-c'])
    expect(wrapper.getComponent(CanvasComposer).props('busy')).toBe(true)

    finishLoads.get('session-c')()
    await flushPromises()

    expect(store.currentSession.id).toBe('session-c')
    expect(wrapper.getComponent(CanvasComposer).props('busy')).toBe(false)
  })

  it('wires compact session create, select, and rename to store-owned CRUD', async () => {
    const store = useCanvasStore()
    store.sessions = [session('session-a'), session('session-b')]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const create = vi.spyOn(store, 'createSession').mockResolvedValue(session('session-new'))
    const update = vi.spyOn(store, 'updateSession').mockResolvedValue(session('session-a'))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    const sheet = wrapper.getComponent(CanvasSessionSheet)
    sheet.vm.$emit('create')
    await flushPromises()
    expect(create).toHaveBeenCalledOnce()
    expect(router.currentRoute.value.path).toBe('/canvas/session-new')

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    wrapper.getComponent(CanvasSessionSheet).vm.$emit('select', 'session-b')
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/canvas/session-b')

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    const reopenedSheet = wrapper.getComponent(CanvasSessionSheet)
    reopenedSheet.vm.$emit('rename', { id: 'session-a', title: 'Renamed' })
    await flushPromises()
    expect(update).toHaveBeenCalledWith('session-a', { title: 'Renamed' })
  })

  it('requires explicit confirmation before deleting a compact Canvas thread', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.sessions = [session('session-a')]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const remove = vi.spyOn(store, 'deleteSession').mockResolvedValue()
    const { wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    wrapper.getComponent(CanvasSessionSheet).vm.$emit('delete', 'session-a')
    await wrapper.vm.$nextTick()

    const confirmation = wrapper.getComponent(ConfirmDialog)
    expect(remove).not.toHaveBeenCalled()
    expect(confirmation.props('visible')).toBe(true)

    confirmation.vm.$emit('cancel')
    await wrapper.vm.$nextTick()
    expect(confirmation.props('visible')).toBe(false)
    expect(remove).not.toHaveBeenCalled()

    wrapper.getComponent(CanvasSessionSheet).vm.$emit('delete', 'session-a')
    await wrapper.vm.$nextTick()
    confirmation.vm.$emit('confirm')
    await flushPromises()

    expect(remove).toHaveBeenCalledOnce()
    expect(remove).toHaveBeenCalledWith('session-a')
  })

  it('does not reuse a tagged Twin chat session for base Canvas prompts', async () => {
    const store = useCanvasStore()
    const twinChat = { ...session('twin-chat'), title: 'Twin private chat', tags: ['companion-twin-chat'] }
    store.currentSession = twinChat
    store.sessions = [twinChat, session('canvas-a')]
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const create = vi.spyOn(store, 'createSession').mockImplementation(async () => {
      store.currentSession = session('canvas-new')
      return store.currentSession
    })
    const invokedSessions = []
    vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(async () => {
      invokedSessions.push(store.currentSession?.id)
      return 'tile-new'
    })
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.get('h1').text()).toBe('Canvas')
    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    expect(wrapper.getComponent(CanvasSessionSheet).props('sessions').map(item => item.id)).toEqual(['canvas-a'])

    wrapper.getComponent(CanvasComposer).vm.$emit('submit', {
      prompt: 'Keep this outside Twin chat',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'plain',
      parentTileId: null,
      parentModelId: null,
    })
    await flushPromises()

    expect(create).toHaveBeenCalledOnce()
    expect(invokedSessions).toEqual(['canvas-new'])
  })

  it('clears and redirects a committed tagged Twin chat deep link', async () => {
    const store = useCanvasStore()
    store.currentSession = { ...session('twin-chat'), tags: ['companion-twin-chat'] }
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const clear = vi.spyOn(store, 'clearSession')
    const send = vi.spyOn(store, 'sendCompanionPrompt').mockResolvedValue('tile-new')
    const { router, wrapper } = await mountView('/canvas/twin-chat')
    await flushPromises()

    expect(send).not.toHaveBeenCalled()
    expect(clear).toHaveBeenCalledOnce()
    expect(store.currentSession).toBeNull()
    expect(router.currentRoute.value.path).toBe('/canvas')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  })

  it('clears and redirects a tagged Twin chat loaded from the API', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const loadSession = vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = { ...session(id), tags: ['companion-twin-chat'] }
      return store.currentSession
    })
    const clear = vi.spyOn(store, 'clearSession')
    const { router, wrapper } = await mountView('/canvas/twin-chat')
    await flushPromises()

    expect(loadSession).toHaveBeenCalledWith('twin-chat')
    expect(clear).toHaveBeenCalledOnce()
    expect(store.currentSession).toBeNull()
    expect(router.currentRoute.value.path).toBe('/canvas')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  })

  it('invalidates a pending routed-session load when compact Canvas unmounts', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(() => new Promise(() => {}))
    const clear = vi.spyOn(store, 'clearSession')
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await router.push('/canvas/session-b')
    await flushPromises()
    wrapper.unmount()

    expect(clear).toHaveBeenCalledOnce()
  })

  it('does not start a queued routed-session load after compact Canvas unmounts', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishSessionB
    const loadSession = vi.spyOn(store, 'loadSession').mockImplementation(id => {
      if (id !== 'session-b') {
        store.currentSession = session(id)
        return Promise.resolve(store.currentSession)
      }
      return new Promise(resolve => {
        finishSessionB = () => resolve(null)
      })
    })
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: '/canvas/:id?', component: { template: '<div />' } }],
    })
    await router.push('/canvas/session-a')
    await router.isReady()
    const stableRoute = shallowReactive({ params: { id: 'session-a' } })
    const wrapper = mount(CanvasCompanionView, {
      global: {
        provide: {
          [routeLocationKey]: stableRoute,
          [routerKey]: router,
        },
      },
    })
    await flushPromises()
    expect(wrapper.vm.$.setupState.route).toBe(stableRoute)

    stableRoute.params = { id: 'session-b' }
    const pendingSessionB = wrapper.vm.$.setupState.requireRouteSession('session-b')
    await flushPromises()
    stableRoute.params = { id: 'session-c' }
    const queuedSessionC = wrapper.vm.$.setupState.requireRouteSession('session-c')
    await flushPromises()
    expect(loadSession.mock.calls.map(([id]) => id)).toEqual(['session-b'])

    wrapper.unmount()
    finishSessionB()
    await pendingSessionB
    await queuedSessionC

    expect(loadSession.mock.calls.map(([id]) => id)).toEqual(['session-b'])
    expect(store.currentSession).toBeNull()
  })

  it('keeps store streaming errors visible in the compact view', async () => {
    const store = useCanvasStore()
    store.error = 'Provider stream failed'
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.get('[role="alert"]').text()).toContain('Provider stream failed')
  })

  it('invalidates a pending auto-create on unmount before it can navigate or send', async () => {
    const store = useCanvasStore()
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishCreate
    vi.spyOn(apiClient.canvas, 'create').mockImplementation(() => new Promise(resolve => {
      finishCreate = () => resolve(session('stale-created'))
    }))
    const send = vi.spyOn(store, 'sendCompanionPrompt').mockResolvedValue('tile-new')
    const { router, wrapper } = await mountView('/canvas')
    await flushPromises()

    wrapper.getComponent(CanvasComposer).vm.$emit('submit', {
      prompt: 'Do not outlive this view',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'plain',
      parentTileId: null,
      parentModelId: null,
    })
    await wrapper.vm.$nextTick()
    wrapper.unmount()
    finishCreate()
    await flushPromises()

    expect(store.currentSession).toBeNull()
    expect(router.currentRoute.value.path).not.toBe('/canvas/stale-created')
    expect(send).not.toHaveBeenCalled()
  })

  it('does not install session A reply state after the user moves to session B', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishSend
    vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(() => new Promise(resolve => {
      finishSend = () => resolve('tile-from-a')
    }))
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    wrapper.getComponent(CanvasComposer).vm.$emit('submit', {
      prompt: 'Answer in A',
      modelId: 'model-a',
      provider: 'openrouter',
      mode: 'plain',
      parentTileId: null,
      parentModelId: null,
    })
    await wrapper.vm.$nextTick()
    store.currentSession = session('session-b')
    finishSend()
    await flushPromises()

    expect(wrapper.getComponent(CanvasComposer).props('parent')).toBeNull()
  })

  it('owns a pending successful send and its draft by the submitted session', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let finishSend
    vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(() => new Promise(resolve => {
      finishSend = () => resolve('tile-from-a')
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Canvas prompt"]').setValue('Session A draft')
    await wrapper.get('.canvas-composer').trigger('submit')
    await router.push('/canvas/session-b')
    await flushPromises()

    const sessionBPrompt = wrapper.get('[aria-label="Canvas prompt"]')
    expect(sessionBPrompt.element.value).toBe('')
    await sessionBPrompt.setValue('Session B draft')
    finishSend()
    await flushPromises()

    expect(sessionBPrompt.element.value).toBe('Session B draft')
    expect(wrapper.getComponent(CanvasComposer).props('parent')).toBeNull()

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[aria-label="Canvas prompt"]').element.value).toBe('')
  })

  it('lets session B send while session A is pending and keeps B busy until B finishes', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let finishSessionA
    let finishSessionB
    const send = vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(() => {
      const owner = store.currentSession?.id
      return new Promise(resolve => {
        if (owner === 'session-a') finishSessionA = () => resolve('tile-from-a')
        if (owner === 'session-b') finishSessionB = () => resolve('tile-from-b')
      })
    })
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Canvas prompt"]').setValue('Session A draft')
    await wrapper.get('.canvas-composer').trigger('submit')
    await router.push('/canvas/session-b')
    await flushPromises()

    const sessionBComposer = wrapper.getComponent(CanvasComposer)
    expect(sessionBComposer.props('busy')).toBe(false)
    await wrapper.get('[aria-label="Canvas prompt"]').setValue('Session B draft')
    await wrapper.get('.canvas-composer').trigger('submit')
    await wrapper.vm.$nextTick()

    expect(send).toHaveBeenCalledTimes(2)
    expect(sessionBComposer.props('busy')).toBe(true)

    finishSessionA()
    await flushPromises()

    expect(sessionBComposer.props('busy')).toBe(true)
    expect(wrapper.get('[aria-label="Canvas prompt"]').element.value).toBe('Session B draft')

    finishSessionB()
    await flushPromises()

    expect(sessionBComposer.props('busy')).toBe(false)
    expect(wrapper.get('[aria-label="Canvas prompt"]').element.value).toBe('')
  })

  it('keeps a late failed send and its draft scoped to the submitted session', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failSend
    vi.spyOn(store, 'sendCompanionPrompt').mockImplementation(() => new Promise((resolve, reject) => {
      failSend = () => reject(new Error('Session A provider failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Canvas prompt"]').setValue('Retry session A')
    await wrapper.get('.canvas-composer').trigger('submit')
    await router.push('/canvas/session-b')
    await flushPromises()

    const sessionBPrompt = wrapper.get('[aria-label="Canvas prompt"]')
    expect(sessionBPrompt.element.value).toBe('')
    await sessionBPrompt.setValue('Keep session B')
    failSend()
    await flushPromises()

    expect(sessionBPrompt.element.value).toBe('Keep session B')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[aria-label="Canvas prompt"]').element.value).toBe('Retry session A')
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A provider failed')
  })

  it('keeps a late regenerate failure scoped to the session that started it', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failRegenerate
    vi.spyOn(store, 'regenerateResponse').mockImplementation(() => new Promise((resolve, reject) => {
      failRegenerate = () => reject(new Error('Session A regenerate failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    wrapper.getComponent(LinearCanvasThread).vm.$emit('regenerate', {
      tileId: 'tile-a',
      modelId: 'model-a',
    })
    await router.push('/canvas/session-b')
    await flushPromises()
    failRegenerate()
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A regenerate failed')
  })

  it('keeps a late feedback failure scoped to the session that started it', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failFeedback
    vi.spyOn(store, 'recordPreferenceFeedback').mockImplementation(() => new Promise((resolve, reject) => {
      failFeedback = () => reject(new Error('Session A feedback failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    wrapper.getComponent(LinearCanvasThread).vm.$emit('feedback', {
      tileId: 'tile-a',
      modelId: 'model-a',
      feedbackType: 'accept',
    })
    await router.push('/canvas/session-b')
    await flushPromises()
    failFeedback()
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A feedback failed')
  })

  it('hides session A store errors while the routed session B load is still pending', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.sessions = [session('session-a'), session('session-b')]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let failSessionBLoad
    vi.spyOn(apiClient.canvas, 'get').mockImplementation(id => new Promise((resolve, reject) => {
      if (id === 'session-b') failSessionBLoad = () => reject(new Error('Session B load failed'))
    }))
    let failRename
    vi.spyOn(apiClient.canvas, 'update').mockImplementation(() => new Promise((resolve, reject) => {
      failRename = () => reject(new Error('Session A rename failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    wrapper.getComponent(CanvasSessionSheet).vm.$emit('rename', {
      id: 'session-a',
      title: 'Renamed A',
    })
    await router.push('/canvas/session-b')
    await flushPromises()

    expect(store.currentSession.id).toBe('session-a')
    failRename()
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    failSessionBLoad()
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('session-b')
  })

  it('keeps a late rename failure scoped to the renamed session', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.sessions = [session('session-a'), session('session-b')]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failRename
    vi.spyOn(apiClient.canvas, 'update').mockImplementation(() => new Promise((resolve, reject) => {
      failRename = () => reject(new Error('Session A rename failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    wrapper.getComponent(CanvasSessionSheet).vm.$emit('rename', {
      id: 'session-a',
      title: 'Renamed A',
    })
    await router.push('/canvas/session-b')
    await flushPromises()
    failRename()
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A rename failed')
  })

  it('keeps a late delete failure scoped to the deleted session', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.sessions = [session('session-a'), session('session-b')]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failDelete
    vi.spyOn(apiClient.canvas, 'delete').mockImplementation(() => new Promise((resolve, reject) => {
      failDelete = () => reject(new Error('Session A delete failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    await wrapper.get('[aria-label="Open Canvas sessions"]').trigger('click')
    wrapper.getComponent(CanvasSessionSheet).vm.$emit('delete', 'session-a')
    await wrapper.vm.$nextTick()
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await router.push('/canvas/session-b')
    await flushPromises()
    failDelete()
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A delete failed')
  })

  it('preserves the Canvas draft when provider sending fails', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    store.availableModels = [{ id: 'model-a', name: 'Model A', provider: 'openrouter' }]
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'sendCompanionPrompt').mockRejectedValue(new Error('Provider unavailable'))
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    const prompt = wrapper.get('[aria-label="Canvas prompt"]')
    await prompt.setValue('Keep this draft')
    await wrapper.get('.canvas-composer').trigger('submit')
    await flushPromises()

    expect(prompt.element.value).toBe('Keep this draft')
    expect(wrapper.get('[role="alert"]').text()).toContain('Provider unavailable')
  })

  it('wires exact in-flight feedback ownership into both response choices', async () => {
    const store = useCanvasStore()
    store.currentSession = {
      ...session('session-a'),
      prompt_tiles: [{
        id: 'tile-a',
        prompt: 'Question',
        models: ['model-a'],
        created_at: '2026-09-01T00:00:00Z',
        responses: {
          'model-a': { model_id: 'model-a', content: 'Answer', status: 'completed' },
        },
      }],
    }
    store.feedbackInFlight.add('session-a:tile-a:model-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.get('[aria-label="Accept tile-a model-a"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Reject tile-a model-a"]').attributes('disabled')).toBeDefined()
  })

  it('opts Canvas into preference capture only for completed global responses', async () => {
    const store = useCanvasStore()
    store.currentSession = {
      ...session('session-a'),
      prompt_tiles: [
        {
          id: 'tile-global',
          prompt: 'Global question',
          models: ['model-a'],
          created_at: '2026-09-01T00:00:00Z',
          twin_relationship_variant: { relationships: [] },
          responses: {
            'model-a': {
              id: 'response-global', model_id: 'model-a', content: 'Global answer', status: 'completed',
            },
          },
        },
        {
          id: 'tile-contextual',
          prompt: 'Contextual question',
          models: ['model-a'],
          created_at: '2026-09-01T01:00:00Z',
          twin_relationship_variant: { relationships: [{
            subject_id: 'owner', predicate: 'with', object_id: 'person-alex', direction: 'directed',
          }] },
          responses: {
            'model-a': {
              id: 'response-contextual', model_id: 'model-a', content: 'Contextual answer', status: 'completed',
            },
          },
        },
      ],
    }
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    expect(wrapper.getComponent(LinearCanvasThread).props('allowPreferenceCapture')).toBe(true)
    expect(wrapper.find('[aria-label="Capture Twin preference from tile-global model-a"]').exists())
      .toBe(true)
    expect(wrapper.find('[aria-label="Capture Twin preference from tile-contextual model-a"]').exists())
      .toBe(false)
  })

  it('captures only blank-starting user-authored preference evidence from the exact response', async () => {
    const store = useCanvasStore()
    store.currentSession = {
      ...session('session-a'),
      prompt_tiles: [{
        id: 'tile-a',
        prompt: 'Question',
        models: ['model-a'],
        created_at: '2026-09-01T00:00:00Z',
        twin_relationship_variant: { relationships: [] },
        responses: {
          'model-a': {
            id: 'response-a', model_id: 'model-a', model_name: 'Model A',
            content: 'MODEL TEXT MUST NOT PREFILL', status: 'completed',
          },
        },
      }],
    }
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const capture = vi.spyOn(store, 'captureInsight').mockResolvedValue({})
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    await wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').trigger('click')
    await wrapper.vm.$nextTick()

    const dialog = wrapper.getComponent(TwinPreferenceCaptureDialog)
    expect(dialog.get('textarea').element.value).toBe('')
    expect(dialog.get('textarea').element.value).not.toContain('MODEL TEXT')
    store.currentSession.prompt_tiles[0].responses['model-a'].id = 'response-regenerated'
    store.currentSession.prompt_tiles[0].responses['model-a'].content = 'REGENERATED MODEL TEXT'
    dialog.vm.$emit('submit', '  I prefer concrete details.  ')
    await flushPromises()

    expect(capture).toHaveBeenCalledWith('preference', 'I prefer concrete details.', {
      response: { tile_id: 'tile-a', model_id: 'model-a' },
      responseWitness: {
        response_id: 'response-a',
        response_content: 'MODEL TEXT MUST NOT PREFILL',
      },
    })
    expect(wrapper.text()).toContain('Evidence recorded. More matching evidence may be needed before Grafyn proposes a Twin memory for review.')
    expect(wrapper.findComponent(TwinPreferenceCaptureDialog).exists()).toBe(false)
  })

  it('deduplicates an exact capture and preserves the dialog draft after failure', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let failCapture
    const capture = vi.spyOn(store, 'captureInsight').mockImplementation(() => new Promise((resolve, reject) => {
      failCapture = () => reject(new Error('Evidence capture failed'))
    }))
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    wrapper.getComponent(LinearCanvasThread).vm.$emit('capture-preference', {
      tileId: 'tile-a', modelId: 'model-a', responseId: 'response-a', responseContent: 'Answer A',
    })
    await wrapper.vm.$nextTick()
    const dialog = wrapper.getComponent(TwinPreferenceCaptureDialog)
    await dialog.get('textarea').setValue('Keep this preference draft')
    dialog.vm.$emit('submit', 'Keep this preference draft')
    dialog.vm.$emit('submit', 'Keep this preference draft')
    await flushPromises()

    expect(capture).toHaveBeenCalledTimes(1)
    expect(wrapper.getComponent(LinearCanvasThread).props('captureInFlight'))
      .toEqual(new Set(['tile-a:model-a']))

    failCapture()
    await flushPromises()

    expect(wrapper.getComponent(TwinPreferenceCaptureDialog).get('textarea').element.value)
      .toBe('Keep this preference draft')
    expect(wrapper.getComponent(TwinPreferenceCaptureDialog).get('[role="alert"]').text())
      .toContain('Evidence capture failed')
    expect(wrapper.findAll('[role="alert"]')).toHaveLength(1)
  })

  it('locks regeneration from opening the preference dialog until its exact capture settles', async () => {
    const store = useCanvasStore()
    store.currentSession = {
      ...session('session-a'),
      prompt_tiles: [{
        id: 'tile-a',
        prompt: 'Question',
        models: ['model-a'],
        created_at: '2026-09-01T00:00:00Z',
        twin_relationship_variant: { relationships: [] },
        responses: {
          'model-a': {
            id: 'response-a', model_id: 'model-a', model_name: 'Model A',
            content: 'Answer A', status: 'completed',
          },
        },
      }],
    }
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishCapture
    vi.spyOn(store, 'captureInsight').mockImplementation(() => new Promise(resolve => {
      finishCapture = () => resolve({})
    }))
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    const regenerate = wrapper.get('[aria-label="Regenerate tile-a model-a"]')
    expect(regenerate.attributes('disabled')).toBeUndefined()
    await wrapper.get('[aria-label="Capture Twin preference from tile-a model-a"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(regenerate.attributes('disabled')).toBeDefined()

    wrapper.getComponent(TwinPreferenceCaptureDialog).vm.$emit('submit', 'Stable preference')
    await flushPromises()
    expect(regenerate.attributes('disabled')).toBeDefined()

    finishCapture()
    await flushPromises()
    expect(regenerate.attributes('disabled')).toBeUndefined()
  })

  it('allows closing a pending capture without releasing its originating response lock', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    let finishCapture
    const capture = vi.spyOn(store, 'captureInsight').mockImplementation(() => new Promise(resolve => {
      finishCapture = () => resolve({})
    }))
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    const thread = wrapper.getComponent(LinearCanvasThread)
    thread.vm.$emit('capture-preference', {
      tileId: 'tile-a', modelId: 'model-a', responseId: 'response-a', responseContent: 'Answer A',
    })
    await wrapper.vm.$nextTick()
    const dialog = wrapper.getComponent(TwinPreferenceCaptureDialog)
    dialog.vm.$emit('submit', 'Keep this preference lock')
    await flushPromises()

    expect(dialog.props('submitting')).toBe(true)
    await dialog.get('[data-test="cancel-twin-preference"]').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.findComponent(TwinPreferenceCaptureDialog).exists()).toBe(false)
    expect(thread.props('captureInFlight')).toEqual(new Set(['tile-a:model-a']))

    thread.vm.$emit('capture-preference', {
      tileId: 'tile-a', modelId: 'model-a', responseId: 'response-a', responseContent: 'Answer A',
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent(TwinPreferenceCaptureDialog).exists()).toBe(false)
    expect(capture).toHaveBeenCalledOnce()

    finishCapture()
    await flushPromises()
    expect(thread.props('captureInFlight')).toEqual(new Set())
  })

  it('closes capture on route change and scopes a late failure to its originating session', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    vi.spyOn(store, 'loadSession').mockImplementation(async id => {
      store.currentSession = session(id)
      return store.currentSession
    })
    let failCapture
    vi.spyOn(store, 'captureInsight').mockImplementation(() => new Promise((resolve, reject) => {
      failCapture = () => reject(new Error('Session A evidence failed'))
    }))
    const { router, wrapper } = await mountView('/canvas/session-a')
    await flushPromises()

    wrapper.getComponent(LinearCanvasThread).vm.$emit('capture-preference', {
      tileId: 'tile-a', modelId: 'model-a', responseId: 'response-a', responseContent: 'Answer A',
    })
    await wrapper.vm.$nextTick()
    wrapper.getComponent(TwinPreferenceCaptureDialog).vm.$emit('submit', 'Session A preference')
    await router.push('/canvas/session-b')
    await flushPromises()

    expect(wrapper.findComponent(TwinPreferenceCaptureDialog).exists()).toBe(false)
    failCapture()
    await flushPromises()
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)

    await router.push('/canvas/session-a')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Session A evidence failed')
  })

  it('discards the capture draft when the active session changes without a route change', async () => {
    const store = useCanvasStore()
    store.currentSession = session('session-a')
    vi.spyOn(store, 'loadSessions').mockResolvedValue()
    vi.spyOn(store, 'loadModels').mockResolvedValue()
    const { wrapper } = await mountView('/canvas')
    await flushPromises()

    wrapper.getComponent(LinearCanvasThread).vm.$emit('capture-preference', {
      tileId: 'tile-a', modelId: 'model-a', responseId: 'response-a', responseContent: 'Answer A',
    })
    await wrapper.vm.$nextTick()
    await wrapper.getComponent(TwinPreferenceCaptureDialog).get('textarea')
      .setValue('Discard with session A')

    store.currentSession = session('session-b')
    await wrapper.vm.$nextTick()

    expect(wrapper.findComponent(TwinPreferenceCaptureDialog).exists()).toBe(false)
  })
})
