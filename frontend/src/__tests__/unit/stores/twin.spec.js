import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useTwinStore } from '@/stores/twin'
import * as apiClient from '@/api/client'

function pendingProposalFixture(overrides = {}) {
  return {
    item_id: 'memory-1',
    kind: 'pending_proposal',
    claim: {
      subject_id: 'owner',
      predicate: 'prefers',
      object: 'evidence first',
      polarity: 'affirmed'
    },
    summary: null,
    proposal_event_id: null,
    review_event_ids: [],
    causal_stream: 'sync_eligible',
    governance: {
      review: 'pending',
      authority: 'evidence_observation',
      sensitivity: 'standard',
      visibility: 'synced_vault',
      allowed_uses: {
        recall: true,
        twin_advisor: true,
        twin_simulation: false,
        export: true,
        training: false,
        sync: true
      }
    },
    relationship_variant: { relationships: [] },
    evidence_event_ids: ['a'.repeat(64)],
    support_count: 3,
    opposition_count: 0,
    prior_exact_support_count: 3,
    last_confirmed_at: '2026-08-31T09:59:00Z',
    valid_from: null,
    valid_to: null,
    superseded_by: [],
    goals: ['ship'],
    tags: ['work'],
    ...overrides
  }
}

function deferred() {
  let resolve
  let reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

describe('Twin Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.restoreAllMocks()
    localStorage.clear()
  })

  it('loadWorkspace loads records, digest, constitution, gaps, decisions, setup, and config', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([
      { record: { id: 'r1', promotion_state: 'candidate' }, evidence_count: 1 }
    ])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([{ id: 'd1' }])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([
      { id: 'c1', dimension: 'values', status: 'active' }
    ])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([{ id: 'g1', status: 'candidate' }])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([
      { episode: { id: 'e1', outcome: null } }
    ])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({
      twin_name: 'Alex',
      twin_role: 'founder',
      values: ['evidence-backed work']
    })
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({
      preset: 'evidence_strict',
      advanced_enabled: true,
      weights: { notes_weight: 2 }
    })

    const store = useTwinStore()
    await store.loadWorkspace()

    expect(store.reviewRecords).toHaveLength(1)
    expect(store.memoryDigestItems).toHaveLength(1)
    expect(store.constitutionItems).toHaveLength(1)
    expect(store.actionGaps).toHaveLength(1)
    expect(store.decisions).toHaveLength(1)
    expect(store.setupDraft.twin_name).toBe('Alex')
    expect(store.setupDraft.values).toBe('evidence-backed work')
    expect(store.configDraft.preset).toBe('evidence_strict')
    expect(store.configDraft.advanced_enabled).toBe(true)
    expect(store.configDraft.weights.notes_weight).toBe(2)
    // defaults still merged in for weights not present in the response
    expect(store.configDraft.weights.action_gaps_weight).toBe(1)
    expect(store.activeConstitutionCount).toBe(1)
    expect(store.activeActionGapCount).toBe(1)
    expect(store.pendingReviewCount).toBe(2)
    expect(store.pendingOutcomeCount).toBe(1)
    expect(store.healthSummary).toBe('1 principles / 1 gaps / 1 decisions')
  })

  it('loadWorkspace sets an error message when the API call fails', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockRejectedValue(new Error('boom'))
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})

    const store = useTwinStore()
    await store.loadWorkspace()

    expect(store.message).toEqual({ type: 'error', text: 'boom' })
  })

  it('loads and appends observation and proposal pages from their returned cursors', async () => {
    const filter = {
      relationships: [],
      goals: ['ship'],
      tags: ['work']
    }
    const observationSpy = vi.spyOn(apiClient.twin, 'listObservations')
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-observation-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-1',
        items: [{ item_id: 'observation-1', kind: 'observation' }],
        nextCursor: 'observation-cursor-2'
      })
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-observation-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-1',
        items: [{ item_id: 'observation-2', kind: 'observation' }],
        nextCursor: null
      })
    const proposalSpy = vi.spyOn(apiClient.twin, 'listProposals')
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-proposal-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-1',
        items: [{ item_id: 'memory-1' }],
        nextCursor: 'proposal-cursor-2'
      })
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-proposal-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-1',
        items: [{ item_id: 'memory-2' }],
        nextCursor: null
      })
    const store = useTwinStore()
    const request = {
      referenceTime: '2026-08-31T10:00:00Z',
      filter,
      cursor: null,
      limit: 1
    }

    await store.loadObservations(request)
    await store.loadMoreObservations()
    await store.loadProposals(request)
    await store.loadMoreProposals()

    expect(observationSpy).toHaveBeenNthCalledWith(2, {
      ...request,
      cursor: 'observation-cursor-2'
    })
    expect(proposalSpy).toHaveBeenNthCalledWith(2, {
      ...request,
      cursor: 'proposal-cursor-2'
    })
    expect(store.observations).toEqual([
      { item_id: 'observation-1', kind: 'observation' },
      { item_id: 'observation-2', kind: 'observation' }
    ])
    expect(store.observationCursor).toBeNull()
    expect(store.observationPage.filterDigest).toBe('filter-1')
    expect(store.proposals).toEqual([{ item_id: 'memory-1' }, { item_id: 'memory-2' }])
    expect(store.proposalCursor).toBeNull()
    expect(store.proposalPage.snapshotId).toBe('snapshot-proposal-1')
  })

  it('appends timeline pages using the returned cursor', async () => {
    const timelineSpy = vi.spyOn(apiClient.twin, 'getEventTimeline')
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-timeline-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-timeline-1',
        items: [{ item_id: 'memory-1', state: 'pending' }],
        nextCursor: 'timeline-cursor-2'
      })
      .mockResolvedValueOnce({
        schemaVersion: 1,
        snapshotId: 'snapshot-timeline-1',
        referenceTime: '2026-08-31T10:00:00Z',
        filterDigest: 'filter-timeline-1',
        items: [{ item_id: 'memory-1', state: 'accepted' }],
        nextCursor: null
      })
    const store = useTwinStore()
    const request = {
      referenceTime: '2026-08-31T10:00:00Z',
      filter: { relationships: [], goals: [], tags: [] },
      cursor: null,
      limit: 1
    }

    await store.loadTimeline(request)
    await store.loadMoreTimeline()

    expect(timelineSpy).toHaveBeenNthCalledWith(2, {
      ...request,
      cursor: 'timeline-cursor-2'
    })
    expect(store.timeline).toEqual([
      { item_id: 'memory-1', state: 'pending' },
      { item_id: 'memory-1', state: 'accepted' }
    ])
    expect(store.timelineCursor).toBeNull()
  })

  it('stores projection, timeline, and attention with per-operation loading and errors', async () => {
    let resolveProjection
    vi.spyOn(apiClient.twin, 'getStateProjection').mockImplementation(() =>
      new Promise(resolve => { resolveProjection = resolve })
    )
    vi.spyOn(apiClient.twin, 'getEventTimeline').mockRejectedValue(new Error('timeline unavailable'))
    vi.spyOn(apiClient.twin, 'rankAttention').mockResolvedValue({
      schemaVersion: 1,
      snapshotId: 'snapshot-attention',
      referenceTime: '2026-08-31T10:00:00Z',
      filterDigest: 'filter-attention',
      trace: { selected: [], excluded: [] }
    })
    const store = useTwinStore()
    const projectionRequest = { referenceTime: '2026-08-31T10:00:00Z' }

    const pendingProjection = store.loadProjection(projectionRequest)
    expect(store.twinStateLoading.projection).toBe(true)
    resolveProjection({
      snapshot_id: 'snapshot-projection',
      reference_time: '2026-08-31T10:00:00Z'
    })
    await pendingProjection

    expect(store.twinStateLoading.projection).toBe(false)
    expect(store.projection.snapshot_id).toBe('snapshot-projection')

    await store.loadTimeline({
      referenceTime: '2026-08-31T10:00:00Z',
      filter: { relationships: [], goals: [], tags: [] },
      cursor: null,
      limit: 20
    })
    expect(store.twinStateLoading.timeline).toBe(false)
    expect(store.twinStateError.timeline).toBe('timeline unavailable')

    const attentionRequest = {
      referenceTime: '2026-08-31T10:00:00Z',
      profile: 'decision',
      query: 'What matters?',
      relationshipVariant: { relationships: [] },
      goals: [],
      destination: 'local',
      filter: { relationships: [], goals: [], tags: [] },
      limit: 10
    }
    await store.rankAttention(attentionRequest)
    expect(store.attention.trace).toEqual({ selected: [], excluded: [] })
    expect(store.twinStateError.attention).toBeNull()
  })

  it('keeps every latest Twin resource coherent when A is invalidated before B', async () => {
    const observations = [deferred(), deferred()]
    const proposals = [deferred(), deferred()]
    const projections = [deferred(), deferred()]
    const timelines = [deferred(), deferred()]
    const attentions = [deferred(), deferred()]
    const workspaceReviews = [deferred(), deferred()]
    vi.spyOn(apiClient.twin, 'listObservations')
      .mockReturnValueOnce(observations[0].promise)
      .mockReturnValueOnce(observations[1].promise)
    vi.spyOn(apiClient.twin, 'listProposals')
      .mockReturnValueOnce(proposals[0].promise)
      .mockReturnValueOnce(proposals[1].promise)
    vi.spyOn(apiClient.twin, 'getStateProjection')
      .mockReturnValueOnce(projections[0].promise)
      .mockReturnValueOnce(projections[1].promise)
    vi.spyOn(apiClient.twin, 'getEventTimeline')
      .mockReturnValueOnce(timelines[0].promise)
      .mockReturnValueOnce(timelines[1].promise)
    vi.spyOn(apiClient.twin, 'rankAttention')
      .mockReturnValueOnce(attentions[0].promise)
      .mockReturnValueOnce(attentions[1].promise)
    vi.spyOn(apiClient.twin, 'getReview')
      .mockReturnValueOnce(workspaceReviews[0].promise)
      .mockReturnValueOnce(workspaceReviews[1].promise)
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const store = useTwinStore()
    const pageRequest = suffix => ({
      referenceTime: `2026-09-01T0${suffix}:00:00Z`,
      filter: { relationships: [], goals: [], tags: [] },
      cursor: null,
      limit: 20,
    })
    const attentionRequest = suffix => ({
      referenceTime: `2026-09-01T0${suffix}:00:00Z`,
      profile: 'capture_review',
      query: suffix,
      relationshipVariant: { relationships: [] },
      goals: [],
      destination: 'local',
      filter: { relationships: [], goals: [], tags: [] },
      limit: 20,
    })

    const pendingA = Promise.all([
      store.loadObservations(pageRequest('1')),
      store.loadProposals(pageRequest('1')),
      store.loadProjection({ referenceTime: '2026-09-01T01:00:00Z' }),
      store.loadTimeline(pageRequest('1')),
      store.rankAttention(attentionRequest('a')),
      store.loadWorkspace(),
    ])
    store.invalidateTwinStateRequests()
    const pendingB = Promise.all([
      store.loadObservations(pageRequest('2')),
      store.loadProposals(pageRequest('2')),
      store.loadProjection({ referenceTime: '2026-09-01T02:00:00Z' }),
      store.loadTimeline(pageRequest('2')),
      store.rankAttention(attentionRequest('b')),
      store.loadWorkspace(),
    ])

    observations[1].resolve({ items: [{ item_id: 'observation-b' }], nextCursor: null })
    proposals[1].resolve({ items: [{ item_id: 'proposal-b' }], nextCursor: null })
    projections[1].resolve({ snapshot_id: 'projection-b' })
    timelines[1].resolve({ items: [{ item_id: 'timeline-b' }], nextCursor: null })
    attentions[1].resolve({ trace: { selected: [{ item_id: 'attention-b' }] } })
    workspaceReviews[1].resolve([{ record: { id: 'workspace-b' } }])
    await pendingB

    observations[0].resolve({ items: [{ item_id: 'observation-a' }], nextCursor: null })
    proposals[0].resolve({ items: [{ item_id: 'proposal-a' }], nextCursor: null })
    projections[0].resolve({ snapshot_id: 'projection-a' })
    timelines[0].resolve({ items: [{ item_id: 'timeline-a' }], nextCursor: null })
    attentions[0].resolve({ trace: { selected: [{ item_id: 'attention-a' }] } })
    workspaceReviews[0].resolve([{ record: { id: 'workspace-a' } }])
    await pendingA

    expect(store.observations[0].item_id).toBe('observation-b')
    expect(store.proposals[0].item_id).toBe('proposal-b')
    expect(store.projection.snapshot_id).toBe('projection-b')
    expect(store.timeline[0].item_id).toBe('timeline-b')
    expect(store.attention.trace.selected[0].item_id).toBe('attention-b')
    expect(store.reviewRecords[0].record.id).toBe('workspace-b')
    expect(Object.values(store.twinStateLoading).every(value => value === false)).toBe(true)
    expect(Object.values(store.twinStateError).every(value => value == null)).toBe(true)
  })

  it('ignores a stale Twin resource error after the newer generation succeeds', async () => {
    const projectionA = deferred()
    const projectionB = deferred()
    vi.spyOn(apiClient.twin, 'getStateProjection')
      .mockReturnValueOnce(projectionA.promise)
      .mockReturnValueOnce(projectionB.promise)
    const store = useTwinStore()

    const pendingA = store.loadProjection({ referenceTime: '2026-09-01T01:00:00Z' })
    store.invalidateTwinStateRequests()
    const pendingB = store.loadProjection({ referenceTime: '2026-09-01T02:00:00Z' })
    projectionB.resolve({ snapshot_id: 'projection-b' })
    await pendingB
    projectionA.reject(new Error('stale projection failure'))
    await pendingA

    expect(store.projection.snapshot_id).toBe('projection-b')
    expect(store.twinStateError.projection).toBeNull()
    expect(store.twinStateLoading.projection).toBe(false)
  })

  it('does not let a stale proposal review commit or launch refresh loaders after invalidation', async () => {
    const reviewResult = deferred()
    const proposal = pendingProposalFixture()
    const initialReferenceTime = '2026-09-01T01:00:00Z'
    const reviewedReferenceTime = '2026-09-01T02:00:00Z'
    vi.spyOn(apiClient.twin, 'reviewProposal').mockReturnValue(reviewResult.promise)
    const observations = vi.spyOn(apiClient.twin, 'listObservations').mockResolvedValue({ items: [] })
    const proposals = vi.spyOn(apiClient.twin, 'listProposals').mockResolvedValue({ items: [] })
    const projection = vi.spyOn(apiClient.twin, 'getStateProjection').mockResolvedValue({ snapshot_id: 'new' })
    const timeline = vi.spyOn(apiClient.twin, 'getEventTimeline').mockResolvedValue({ items: [] })
    const store = useTwinStore()
    store.proposalPage = {
      snapshotId: 'snapshot-a',
      referenceTime: initialReferenceTime,
      items: [structuredClone(proposal)],
    }
    store.projection = {
      snapshot_id: 'snapshot-a',
      reference_time: initialReferenceTime,
      pending_proposals: [structuredClone(proposal)],
    }

    const pending = store.reviewProposal(proposal.item_id, 'accept')
    store.invalidateTwinStateRequests()
    reviewResult.resolve({
      snapshot: {
        snapshot_id: 'snapshot-after-review',
        reference_time: reviewedReferenceTime,
        pending_proposals: [],
      },
      referenceTime: reviewedReferenceTime,
    })
    await pending

    expect(store.projection.snapshot_id).toBe('snapshot-a')
    expect(observations).not.toHaveBeenCalled()
    expect(proposals).not.toHaveBeenCalled()
    expect(projection).not.toHaveBeenCalled()
    expect(timeline).not.toHaveBeenCalled()
    expect(store.twinStateLoading.review).toBe(false)
  })

  it('does not let a stale digest review launch a new workspace generation after invalidation', async () => {
    const digestReview = deferred()
    vi.spyOn(apiClient.twin, 'reviewMemoryDigestItem').mockReturnValue(digestReview.promise)
    const workspace = vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const store = useTwinStore()

    const pending = store.reviewMemoryDigestItem('digest-a', 'keep')
    store.invalidateTwinStateRequests()
    digestReview.resolve()
    await pending

    expect(workspace).not.toHaveBeenCalled()
    expect(store.message).toBeNull()
  })

  it('reviews a proposal against the loaded snapshot and reloads state at server review time', async () => {
    const initialReferenceTime = '2026-08-31T10:00:00Z'
    const reviewedReferenceTime = '2026-08-31T10:05:00Z'
    const filter = { relationships: [], goals: ['ship'], tags: ['work'] }
    const proposalItem = pendingProposalFixture()
    const page = (referenceTime, items = [], snapshotId = `snapshot-${referenceTime}`) => ({
      schemaVersion: 1,
      snapshotId,
      referenceTime,
      filterDigest: 'filter-1',
      items,
      nextCursor: null
    })
    const observationSpy = vi.spyOn(apiClient.twin, 'listObservations')
      .mockResolvedValueOnce(page(initialReferenceTime))
      .mockResolvedValueOnce(page(reviewedReferenceTime))
    const proposalSpy = vi.spyOn(apiClient.twin, 'listProposals')
      .mockResolvedValueOnce(page(initialReferenceTime, [proposalItem], 'snapshot-before-review'))
      .mockResolvedValueOnce(page(reviewedReferenceTime, [], 'snapshot-after-review'))
    const timelineSpy = vi.spyOn(apiClient.twin, 'getEventTimeline')
      .mockResolvedValueOnce(page(initialReferenceTime))
      .mockResolvedValueOnce(page(reviewedReferenceTime))
    const projectionSpy = vi.spyOn(apiClient.twin, 'getStateProjection').mockResolvedValue({
      snapshot_id: 'snapshot-after-review',
      reference_time: reviewedReferenceTime,
      pending_proposals: []
    })
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewProposal').mockResolvedValue({
      schemaVersion: 1,
      memoryId: 'memory-1',
      decision: 'accept',
      proposalEventId: 'proposal-event-1',
      reviewEventId: 'review-event-1',
      referenceTime: reviewedReferenceTime,
      snapshot: {
        snapshot_id: 'snapshot-after-review',
        reference_time: reviewedReferenceTime,
        pending_proposals: []
      }
    })
    const store = useTwinStore()
    const initialPageRequest = {
      referenceTime: initialReferenceTime,
      filter,
      cursor: null,
      limit: 20
    }
    await store.loadObservations(initialPageRequest)
    await store.loadProposals(initialPageRequest)
    await store.loadTimeline(initialPageRequest)
    store.projection = {
      snapshot_id: 'snapshot-before-review',
      reference_time: initialReferenceTime,
      pending_proposals: [structuredClone(proposalItem)]
    }
    store.attention = { trace: { selected: [{ item_id: 'memory-1' }] } }
    const reviewedClaim = {
      subject_id: 'owner',
      predicate: 'prefers',
      object: 'evidence first',
      polarity: 'affirmed'
    }

    const response = await store.reviewProposal(
      'memory-1',
      'accept',
      reviewedClaim,
      'This is accurate.'
    )

    expect(reviewSpy).toHaveBeenCalledWith({
      memoryId: 'memory-1',
      decision: 'accept',
      reviewedClaim,
      rationale: 'This is accurate.',
      expectedSnapshotId: 'snapshot-before-review',
      snapshotReferenceTime: initialReferenceTime
    })
    expect(observationSpy).toHaveBeenLastCalledWith({
      ...initialPageRequest,
      referenceTime: reviewedReferenceTime,
      cursor: null
    })
    expect(proposalSpy).toHaveBeenLastCalledWith({
      ...initialPageRequest,
      referenceTime: reviewedReferenceTime,
      cursor: null
    })
    expect(timelineSpy).toHaveBeenLastCalledWith({
      ...initialPageRequest,
      referenceTime: reviewedReferenceTime,
      cursor: null
    })
    expect(projectionSpy).toHaveBeenCalledWith({ referenceTime: reviewedReferenceTime })
    expect(store.projection.snapshot_id).toBe('snapshot-after-review')
    expect(store.attention).toBeNull()
    expect(store.twinStateLoading.review).toBe(false)
    expect(store.twinStateError.review).toBeNull()
    expect(response.reviewEventId).toBe('review-event-1')
  })

  it('refuses to review a proposal that is absent from the displayed proposal page', async () => {
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewProposal').mockResolvedValue({})
    const store = useTwinStore()
    store.proposalPage = {
      snapshotId: 'snapshot-one',
      referenceTime: '2026-08-31T10:00:00Z',
      items: []
    }
    store.projection = {
      snapshot_id: 'snapshot-one',
      reference_time: '2026-08-31T10:00:00Z',
      pending_proposals: [pendingProposalFixture()]
    }

    const response = await store.reviewProposal('memory-1', 'accept')

    expect(response).toBeNull()
    expect(reviewSpy).not.toHaveBeenCalled()
    expect(store.twinStateError.review).toContain('displayed proposal')
  })

  it('refuses to review when the displayed page and projection snapshots differ', async () => {
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewProposal').mockResolvedValue({})
    const proposal = pendingProposalFixture()
    const store = useTwinStore()
    store.proposalPage = {
      snapshotId: 'snapshot-page',
      referenceTime: '2026-08-31T10:00:00Z',
      items: [proposal]
    }
    store.projection = {
      snapshot_id: 'snapshot-projection',
      reference_time: '2026-08-31T10:00:00Z',
      pending_proposals: [structuredClone(proposal)]
    }

    const response = await store.reviewProposal('memory-1', 'accept')

    expect(response).toBeNull()
    expect(reviewSpy).not.toHaveBeenCalled()
    expect(store.twinStateError.review).toContain('same Twin snapshot')
  })

  it('refuses to review when the displayed page and projection reference times differ', async () => {
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewProposal').mockResolvedValue({})
    const proposal = pendingProposalFixture()
    const store = useTwinStore()
    store.proposalPage = {
      snapshotId: 'snapshot-one',
      referenceTime: '2026-08-31T10:00:00Z',
      items: [proposal]
    }
    store.projection = {
      snapshot_id: 'snapshot-one',
      reference_time: '2026-08-31T10:00:01Z',
      pending_proposals: [structuredClone(proposal)]
    }

    const response = await store.reviewProposal('memory-1', 'accept')

    expect(response).toBeNull()
    expect(reviewSpy).not.toHaveBeenCalled()
    expect(store.twinStateError.review).toContain('same Twin snapshot')
  })

  it('refuses to review when the projected proposal differs from the displayed item', async () => {
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewProposal').mockResolvedValue({})
    const proposal = pendingProposalFixture()
    const store = useTwinStore()
    store.proposalPage = {
      snapshotId: 'snapshot-one',
      referenceTime: '2026-08-31T10:00:00Z',
      items: [proposal]
    }
    store.projection = {
      snapshot_id: 'snapshot-one',
      reference_time: '2026-08-31T10:00:00Z',
      pending_proposals: [pendingProposalFixture({ support_count: 4 })]
    }

    const response = await store.reviewProposal('memory-1', 'accept')

    expect(response).toBeNull()
    expect(reviewSpy).not.toHaveBeenCalled()
    expect(store.twinStateError.review).toContain('changed since it was displayed')
  })

  it('reviewConstitutionItem calls the api with the given action and reloads', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const reviewSpy = vi.spyOn(apiClient.twin, 'reviewConstitutionItem').mockResolvedValue({})

    const store = useTwinStore()
    await store.reviewConstitutionItem('c1', 'keep')

    expect(reviewSpy).toHaveBeenCalledWith('c1', { action: 'keep' })
    expect(store.message.type).toBe('success')
  })

  it('reviewConstitutionItem sets an error message on failure', async () => {
    vi.spyOn(apiClient.twin, 'reviewConstitutionItem').mockRejectedValue(new Error('nope'))

    const store = useTwinStore()
    await store.reviewConstitutionItem('c1', 'keep')

    expect(store.message).toEqual({ type: 'error', text: 'nope' })
  })

  it('reviewActionGap and reviewMemoryDigestItem call their respective api functions', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const gapSpy = vi.spyOn(apiClient.twin, 'reviewActionGap').mockResolvedValue({})
    const digestSpy = vi.spyOn(apiClient.twin, 'reviewMemoryDigestItem').mockResolvedValue({})

    const store = useTwinStore()
    await store.reviewActionGap('g1', 'soften')
    await store.reviewMemoryDigestItem('d1', 'reject')

    expect(gapSpy).toHaveBeenCalledWith('g1', { action: 'soften' })
    expect(digestSpy).toHaveBeenCalledWith('d1', { action: 'reject' })
  })

  it('setPromotion calls the api and reports the new state', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const promoSpy = vi.spyOn(apiClient.twin, 'setPromotion').mockResolvedValue({})

    const store = useTwinStore()
    await store.setPromotion('r1', 'endorsed')

    expect(promoSpy).toHaveBeenCalledWith('r1', 'endorsed', null)
    expect(store.message.text).toBe('Set record to Endorsed')
  })

  it('openEvidence loads evidence for a record and tracks loading state', async () => {
    let resolveEvidence
    vi.spyOn(apiClient.twin, 'resolveEvidence').mockImplementation(() => new Promise(resolve => {
      resolveEvidence = resolve
    }))

    const store = useTwinStore()
    const promise = store.openEvidence('r1')
    expect(store.selectedRecordId).toBe('r1')
    expect(store.evidenceLoading).toBe(true)

    resolveEvidence([{ event_id: 'evt-1' }])
    await promise

    expect(store.evidenceLoading).toBe(false)
    expect(store.selectedEvidence).toEqual([{ event_id: 'evt-1' }])
  })

  it('openEvidence sets an error message and stops loading on failure', async () => {
    vi.spyOn(apiClient.twin, 'resolveEvidence').mockRejectedValue(new Error('evidence failed'))

    const store = useTwinStore()
    await store.openEvidence('r1')

    expect(store.evidenceLoading).toBe(false)
    expect(store.message).toEqual({ type: 'error', text: 'evidence failed' })
  })

  it('saveSetup splits textarea drafts into arrays before calling the api', async () => {
    vi.spyOn(apiClient.twin, 'getReview').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listMemoryDigest').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listConstitutionItems').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listActionGaps').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'listDecisionEpisodes').mockResolvedValue([])
    vi.spyOn(apiClient.twin, 'getConstitutionSetup').mockResolvedValue({})
    vi.spyOn(apiClient.twin, 'getDecisionMirrorConfig').mockResolvedValue({})
    const saveSpy = vi.spyOn(apiClient.twin, 'saveConstitutionSetup').mockResolvedValue({})

    const store = useTwinStore()
    store.setupDraft.twin_name = ' Alex Chen '
    store.setupDraft.twin_role = ' founder '
    store.setupDraft.values = 'evidence-backed work\nfast feedback'

    await store.saveSetup()

    expect(saveSpy).toHaveBeenCalledWith(expect.objectContaining({
      twin_name: 'Alex Chen',
      twin_role: 'founder',
      values: ['evidence-backed work', 'fast feedback']
    }))
  })

  it('dismissTutorial persists the dismissal and clears the intro flag', () => {
    const store = useTwinStore()
    expect(store.showTutorialIntro).toBe(true)

    store.dismissTutorial()

    expect(store.showTutorialIntro).toBe(false)
    expect(localStorage.getItem('grafyn.twinWorkspaceTutorial.dismissed')).toBe('true')
  })

  it('exportDecisionBenchmark reports the exported decision count', async () => {
    vi.spyOn(apiClient.twin, 'exportData').mockResolvedValue({
      decision_mirror_benchmark: { count: 4 }
    })

    const store = useTwinStore()
    await store.exportDecisionBenchmark()

    expect(store.message.text).toBe('Exported benchmark: 4 decisions')
    expect(store.exportingBenchmark).toBe(false)
  })
})
