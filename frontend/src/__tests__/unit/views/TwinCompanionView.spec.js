import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import TwinCompanionView from '@/views/companion/TwinCompanionView.vue'
import TwinReviewQueue from '@/components/companion/TwinReviewQueue.vue'
import TwinChat from '@/components/companion/TwinChat.vue'
import { useTwinStore } from '@/stores/twin'
import { resetTransport, setRuntimeProfile, setRuntimeStatus } from '@/api/transport'
import { createRuntimeProfile } from '@/platform/runtime'
import { CAPABILITY_NAMES, normalizeRuntimeStatus } from '@/platform/capabilities'

const relationship = {
  subject_id: 'owner',
  predicate: 'with',
  object_id: 'person-alex',
  direction: 'directed',
}

const bobRelationship = {
  subject_id: 'owner',
  predicate: 'with',
  object_id: 'person-bob',
  direction: 'directed',
}

const bidirectionalRelationship = {
  ...relationship,
  direction: 'bidirectional',
}

function useTwinRuntime({ twinReview = true, twinChat = true } = {}) {
  setRuntimeProfile(createRuntimeProfile({ isTauri: true, platform: 'android' }))
  const capabilities = Object.fromEntries(CAPABILITY_NAMES.map(name => [name, false]))
  Object.assign(capabilities, {
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview,
    twinChat,
    linearCanvas: true,
  })
  setRuntimeStatus(normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'android',
    capabilities,
    vault: { kind: 'app_private', available: true },
    secureSecrets: { status: 'ready', code: null, message: null },
    nativeImageShare: {
      status: 'unavailable',
      code: 'share_unavailable',
      message: 'Native image sharing is unavailable.',
    },
    diagnostics: [],
  }))
}

describe('TwinCompanionView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useTwinRuntime()
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-09-01T02:03:04Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
    resetTransport()
  })

  function mountView(store, {
    projectionGate = null,
    projection = null,
    timeline = null,
    attentionGate = null,
    attention = null,
    stubQueue = true,
  } = {}) {
    vi.spyOn(store, 'loadWorkspace').mockResolvedValue()
    vi.spyOn(store, 'loadProjection').mockImplementation(async () => {
      if (projectionGate) await projectionGate
      store.projection = projection || {
        snapshot_id: 'a'.repeat(64),
        reference_time: '2026-09-01T02:03:04.000Z',
        reviewed_memories: [{
          item_id: 'memory-a',
          claim: { subject_id: 'owner', predicate: 'prefers', object: 'evidence', polarity: 'affirmed' },
        }],
        pending_proposals: [],
        relationship_variants: [{
          relationship_variant: { relationships: [relationship] },
          reviewed_memory_ids: ['memory-a'],
          pending_memory_ids: [],
          observation_event_ids: [],
        }],
      }
      return store.projection
    })
    vi.spyOn(store, 'loadProposals').mockResolvedValue({ items: [] })
    vi.spyOn(store, 'loadTimeline').mockImplementation(async () => {
      store.timelinePage = { items: timeline || [{ item_id: 'memory-a', state: 'accepted', effective_at: '2026-09-01T01:00:00Z' }] }
      return store.timelinePage
    })
    vi.spyOn(store, 'rankAttention').mockImplementation(async () => {
      if (attentionGate) await attentionGate
      store.attention = attention || { trace: { selected: [], excluded: [] } }
      return store.attention
    })

    return mount(TwinCompanionView, {
      global: {
        stubs: {
          CompanionShell: {
            template: '<div data-test="inner-companion-shell"><slot /></div>',
          },
          TwinReviewQueue: stubQueue,
          TwinChat: true,
        },
      },
    })
  }

  it('leaves the single compact shell owned by App', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()

    expect(wrapper.find('[data-test="inner-companion-shell"]').exists()).toBe(false)
  })

  it('keeps the relationship label locked while its filtered snapshot is refreshing', async () => {
    const store = useTwinStore()
    let finishProjection
    const projectionGate = new Promise(resolve => {
      finishProjection = resolve
    })
    const wrapper = mountView(store, { projectionGate })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[aria-label="Twin relationship filter"]').attributes('disabled')).toBeDefined()

    finishProjection()
    await flushPromises()

    expect(wrapper.get('[aria-label="Twin relationship filter"]').attributes('disabled')).toBeUndefined()
  })

  it('locks refresh and relationship controls while proposal review owns the snapshot', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()

    store.twinStateLoading.review = true
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[aria-label="Refresh Twin companion"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Twin relationship filter"]').attributes('disabled')).toBeDefined()

    store.twinStateLoading.review = false
    store.twinStateLoading.attention = true
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[aria-label="Refresh Twin companion"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Twin relationship filter"]').attributes('disabled')).toBeDefined()
  })

  it('uses one explicit reference time for projection, proposals, timeline, and attention', async () => {
    const store = useTwinStore()
    mountView(store)
    await flushPromises()

    const referenceTime = '2026-09-01T02:03:04.000Z'
    const emptyFilter = { relationships: [], goals: [], tags: [] }
    expect(store.loadProjection).toHaveBeenCalledWith({ referenceTime })
    expect(store.loadProposals).toHaveBeenCalledWith({
      referenceTime,
      filter: emptyFilter,
      cursor: null,
      limit: 50,
    })
    expect(store.loadTimeline).toHaveBeenCalledWith({
      referenceTime,
      filter: emptyFilter,
      cursor: null,
      limit: 50,
    })
    expect(store.rankAttention).toHaveBeenCalledWith(expect.objectContaining({
      referenceTime,
      profile: 'capture_review',
      relationshipVariant: { relationships: [] },
      filter: emptyFilter,
    }))
  })

  it('converts selected projection relationship keys to the camel-case command filter', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()

    const select = wrapper.get('[aria-label="Twin relationship filter"]')
    const selectedValue = select.findAll('option')[1].element.value
    await select.setValue(selectedValue)
    await wrapper.get('[aria-label="Refresh Twin companion"]').trigger('click')
    await flushPromises()

    const expectedRelationship = {
      subjectId: 'owner',
      predicate: 'with',
      objectId: 'person-alex',
      direction: 'directed',
    }
    expect(store.loadProposals).toHaveBeenLastCalledWith(expect.objectContaining({
      filter: { relationships: [expectedRelationship], goals: [], tags: [] },
    }))
    expect(store.rankAttention).toHaveBeenLastCalledWith(expect.objectContaining({
      relationshipVariant: { relationships: [expectedRelationship] },
    }))
  })

  it('passes the selected persisted relationship variant into Twin chat', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()
    await wrapper.get('[aria-label="Open Twin chat"]').trigger('click')

    expect(wrapper.getComponent(TwinChat).props('relationshipVariant')).toEqual({ relationships: [] })

    const select = wrapper.get('[aria-label="Twin relationship filter"]')
    const alexOption = select.findAll('option').find(option => option.text().includes('person-alex'))
    await select.setValue(alexOption.element.value)
    await flushPromises()

    expect(wrapper.getComponent(TwinChat).props('relationshipVariant')).toEqual({
      relationships: [relationship],
    })
  })

  it('keeps unique full relationship variants distinct and passes the exact multi-edge variant', async () => {
    const store = useTwinStore()
    const multiVariant = { relationships: [bobRelationship, relationship] }
    const projection = {
      snapshot_id: 'a'.repeat(64),
      reference_time: '2026-09-01T02:03:04.000Z',
      reviewed_memories: [
        {
          item_id: 'memory-alex',
          claim: { subject_id: 'owner', predicate: 'prefers', object: 'alex-only' },
          relationship_variant: { relationships: [relationship] },
        },
        {
          item_id: 'memory-multi',
          claim: { subject_id: 'owner', predicate: 'prefers', object: 'alex-and-bob' },
          relationship_variant: multiVariant,
        },
      ],
      pending_proposals: [],
      relationship_variants: [
        { relationship_variant: { relationships: [relationship] } },
        { relationship_variant: { relationships: [bidirectionalRelationship] } },
        { relationship_variant: { relationships: [bobRelationship] } },
        { relationship_variant: multiVariant },
        { relationship_variant: { relationships: [relationship, bobRelationship] } },
      ],
    }
    const wrapper = mountView(store, { projection })
    await flushPromises()

    const select = wrapper.get('[aria-label="Twin relationship filter"]')
    const options = select.findAll('option')
    expect(options).toHaveLength(5)
    expect(options.filter(option => option.text().includes('person-alex') && option.text().includes('directed')))
      .toHaveLength(2)
    expect(options.some(option => option.text().includes('person-alex') && option.text().includes('bidirectional')))
      .toBe(true)

    const multiOption = options.find(option => (
      option.text().includes('person-alex') && option.text().includes('person-bob')
    ))
    await select.setValue(multiOption.element.value)
    await flushPromises()

    const expectedRelationships = [
      {
        subjectId: 'owner',
        predicate: 'with',
        objectId: 'person-alex',
        direction: 'directed',
      },
      {
        subjectId: 'owner',
        predicate: 'with',
        objectId: 'person-bob',
        direction: 'directed',
      },
    ]
    expect(store.loadProposals).toHaveBeenLastCalledWith(expect.objectContaining({
      filter: { relationships: expectedRelationships, goals: [], tags: [] },
    }))
    expect(store.rankAttention).toHaveBeenLastCalledWith(expect.objectContaining({
      relationshipVariant: { relationships: expectedRelationships },
    }))

    await wrapper.get('[aria-label="Open Twin chat"]').trigger('click')
    expect(wrapper.getComponent(TwinChat).props('relationshipVariant')).toEqual({
      relationships: [relationship, bobRelationship],
    })

    await wrapper.get('[aria-label="Open Twin memory"]').trigger('click')
    expect(wrapper.text()).toContain('alex-and-bob')
    expect(wrapper.text()).not.toContain('alex-only')
  })

  it('exposes reviewed memory and its temporal state on the compact surface', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()

    await wrapper.get('[aria-label="Open Twin memory"]').trigger('click')

    expect(wrapper.text()).toContain('owner prefers evidence')
    expect(wrapper.text()).toContain('Accepted')
  })

  it('labels every timeline entry as Global or with its exact relationship context', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store, {
      timeline: [
        {
          item_id: 'memory-global',
          state: 'accepted',
          effective_at: '2026-09-01T01:00:00Z',
          relationship_variant: { relationships: [] },
        },
        {
          item_id: 'memory-alex',
          state: 'accepted',
          effective_at: '2026-09-01T01:01:00Z',
          relationship_variant: { relationships: [relationship] },
        },
      ],
    })
    await flushPromises()
    await wrapper.get('[aria-label="Open Twin memory"]').trigger('click')

    const entries = wrapper.findAll('.timeline-list li')
    expect(entries[0].text()).toContain('Context: Global')
    expect(entries[1].text()).toContain('Context: owner with person-alex [directed]')
  })

  it('keeps review available while independently hiding unavailable Twin chat', async () => {
    useTwinRuntime({ twinChat: false })
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()

    expect(wrapper.find('[aria-label="Open Twin chat"]').exists()).toBe(false)
    expect(wrapper.getComponent(TwinReviewQueue).exists()).toBe(true)
    expect(wrapper.get('[aria-label="Open Twin review"]').attributes('aria-current')).toBe('page')
  })

  it('closes an open Twin chat when its mounted runtime capability is revoked', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()
    await wrapper.get('[aria-label="Open Twin chat"]').trigger('click')

    expect(wrapper.getComponent(TwinChat).exists()).toBe(true)

    useTwinRuntime({ twinChat: false })
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[aria-label="Open Twin chat"]').exists()).toBe(false)
    expect(wrapper.findComponent(TwinChat).exists()).toBe(false)
    expect(wrapper.getComponent(TwinReviewQueue).exists()).toBe(true)
    expect(wrapper.get('[aria-label="Open Twin review"]').attributes('aria-current')).toBe('page')
  })

  it('blocks a Twin chat tab action as its mounted capability is revoked', async () => {
    const store = useTwinStore()
    const wrapper = mountView(store)
    await flushPromises()
    const chatTab = wrapper.get('[aria-label="Open Twin chat"]')

    useTwinRuntime({ twinChat: false })
    await chatTab.trigger('click')

    expect(wrapper.find('[aria-label="Open Twin chat"]').exists()).toBe(false)
    expect(wrapper.findComponent(TwinChat).exists()).toBe(false)
    expect(wrapper.getComponent(TwinReviewQueue).exists()).toBe(true)
  })

  it('blocks mounted Twin review actions as runtime authority is revoked', async () => {
    const store = useTwinStore()
    store.proposalPage = {
      items: [{
        item_id: 'proposal-a',
        claim: { subject_id: 'owner', predicate: 'prefers', object: 'evidence' },
      }],
    }
    store.memoryDigestItems = [{ id: 'digest-a', pattern: 'Evidence pattern' }]
    const wrapper = mountView(store, { stubQueue: false })
    await flushPromises()
    const refreshCalls = store.loadWorkspace.mock.calls.length
    const reviewProposal = vi.spyOn(store, 'reviewProposal').mockResolvedValue(null)
    const reviewDigest = vi.spyOn(store, 'reviewMemoryDigestItem').mockResolvedValue()
    const refreshButton = wrapper.get('[aria-label="Refresh Twin companion"]')
    const acceptProposal = wrapper.get('[aria-label="Accept proposal proposal-a"]')
    const keepDigest = wrapper.get('[aria-label="Keep digest digest-a"]')

    useTwinRuntime({ twinReview: false, twinChat: false })
    await wrapper.vm.$nextTick()
    await refreshButton.trigger('click')
    await acceptProposal.trigger('click')
    await keepDigest.trigger('click')

    expect(refreshButton.attributes('disabled')).toBeDefined()
    expect(acceptProposal.attributes('disabled')).toBeDefined()
    expect(keepDigest.attributes('disabled')).toBeDefined()
    expect(store.loadWorkspace).toHaveBeenCalledTimes(refreshCalls)
    expect(reviewProposal).not.toHaveBeenCalled()
    expect(reviewDigest).not.toHaveBeenCalled()
  })

  it('hides stale attention and locks proposal review until the matching refresh finishes', async () => {
    const store = useTwinStore()
    store.proposalPage = {
      items: [{
        item_id: 'proposal-a',
        claim: { subject_id: 'owner', predicate: 'prefers', object: 'evidence' },
        relationship_variant: { relationships: [relationship] },
      }],
    }
    store.attention = {
      trace: {
        selected: [{ item_id: 'proposal-a', attention: { explanation: 'Stale reason' } }],
        excluded: [],
      },
    }
    let finishAttention
    const attentionGate = new Promise(resolve => { finishAttention = resolve })
    const wrapper = mountView(store, {
      attentionGate,
      attention: {
        trace: {
          selected: [{ item_id: 'proposal-a', attention: { explanation: 'Fresh reason' } }],
          excluded: [],
        },
      },
      stubQueue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Stale reason')
    expect(wrapper.get('[aria-label="Accept proposal proposal-a"]').attributes('disabled')).toBeDefined()

    finishAttention()
    await flushPromises()

    expect(wrapper.text()).toContain('Fresh reason')
    expect(wrapper.get('[aria-label="Accept proposal proposal-a"]').attributes('disabled')).toBeUndefined()
  })

  it('invalidates companion-owned Twin requests when the view unmounts', async () => {
    const store = useTwinStore()
    store.invalidateTwinStateRequests = vi.fn()
    const wrapper = mountView(store)
    await flushPromises()

    wrapper.unmount()

    expect(store.invalidateTwinStateRequests).toHaveBeenCalledOnce()
  })

  it('labels contextual reviewed memory and filters it by the exact selected relationship', async () => {
    const store = useTwinStore()
    const projection = {
      snapshot_id: 'a'.repeat(64),
      reference_time: '2026-09-01T02:03:04.000Z',
      reviewed_memories: [
        {
          item_id: 'memory-alex',
          claim: { subject_id: 'owner', predicate: 'prefers', object: 'alex-context' },
          relationship_variant: { relationships: [relationship] },
        },
        {
          item_id: 'memory-bob',
          claim: { subject_id: 'owner', predicate: 'prefers', object: 'bob-context' },
          relationship_variant: { relationships: [bobRelationship] },
        },
      ],
      pending_proposals: [],
      relationship_variants: [relationship, bobRelationship].map(value => ({
        relationship_variant: { relationships: [value] },
        reviewed_memory_ids: [],
        pending_memory_ids: [],
        observation_event_ids: [],
      })),
    }
    const wrapper = mountView(store, { projection })
    await flushPromises()
    await wrapper.get('[aria-label="Open Twin memory"]').trigger('click')

    expect(wrapper.text()).toContain('alex-context')
    expect(wrapper.text()).toContain('bob-context')
    expect(wrapper.text()).toContain('owner with person-alex [directed]')
    expect(wrapper.text()).toContain('owner with person-bob [directed]')

    const select = wrapper.get('[aria-label="Twin relationship filter"]')
    const alexOption = select.findAll('option').find(option => option.text().includes('person-alex'))
    await select.setValue(alexOption.element.value)
    await flushPromises()

    expect(wrapper.text()).toContain('alex-context')
    expect(wrapper.text()).not.toContain('bob-context')
  })

  it('locks digest review and surfaces the store-owned failure message', async () => {
    const store = useTwinStore()
    let finishReview
    const review = vi.spyOn(store, 'reviewMemoryDigestItem').mockImplementation(() => new Promise(resolve => {
      finishReview = () => {
        store.message = { type: 'error', text: 'Digest review failed' }
        resolve()
      }
    }))
    const wrapper = mountView(store)
    await flushPromises()
    const queue = wrapper.getComponent(TwinReviewQueue)

    queue.vm.$emit('review-digest', { id: 'digest-a', action: 'keep' })
    queue.vm.$emit('review-digest', { id: 'digest-a', action: 'keep' })
    await wrapper.vm.$nextTick()

    expect(review).toHaveBeenCalledOnce()
    expect(queue.props('loading')).toBe(true)
    expect(wrapper.get('[aria-label="Refresh Twin companion"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Twin relationship filter"]').attributes('disabled')).toBeDefined()

    finishReview()
    await flushPromises()

    expect(wrapper.get('[role="alert"]').text()).toContain('Digest review failed')
    expect(queue.props('loading')).toBe(false)
  })
})
