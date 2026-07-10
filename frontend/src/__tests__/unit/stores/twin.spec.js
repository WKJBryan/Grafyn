import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useTwinStore } from '@/stores/twin'
import * as apiClient from '@/api/client'

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
