import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { mount, flushPromises } from '@vue/test-utils'
import { useEvidenceStore } from '@/stores/evidence'
import { evidence } from '@/api/evidence'
import EvidenceWorkspace from '@/components/twin/EvidenceWorkspace.vue'

vi.mock('@/api/evidence', () => ({
  evidence: {
    snapshot: vi.fn(),
    saveGoal: vi.fn(),
    reviewStatement: vi.fn(),
    listPredictions: vi.fn(),
    createPilot: vi.fn(),
  },
}))
const snapshot = (subject_id) => ({
  subject_id,
  subject_name: subject_id,
  sources: [],
  cases: [],
  goals: [],
  nodes: [],
  statements: [],
  relationships: [],
  jobs: [],
  interview_draft: null,
})
let store
beforeEach(() => {
  vi.clearAllMocks()
  setActivePinia(createPinia())
  store = useEvidenceStore()
  store.snapshot = snapshot('bryan-pilot')
  evidence.snapshot.mockImplementation(async (scope) =>
    snapshot(scope === 'current' ? 'current-person' : 'bryan-pilot')
  )
})

describe('evidence scope isolation', () => {
  it('resets every subject-specific cache and calls current IPC only after explicit switching', async () => {
    expect(store.scope).toBe('pilot')
    store.prediction = { id: 'pilot-prediction' }
    store.predictionRequest = { situation: 'Private pilot situation' }
    store.predictionHistory = {
      records: [{ id: 'pilot-prediction' }],
      batches: [{ id: 'pilot-batch' }],
    }
    store.batchId = 'pilot-batch'
    await store.switchScope('current')
    expect(evidence.snapshot).toHaveBeenCalledWith('current')
    expect(store.scope).toBe('current')
    expect(store.snapshot.subject_id).toBe('current-person')
    expect(store.prediction).toBeNull()
    expect(store.predictionRequest).toBeNull()
    expect(store.predictionHistory).toEqual({ records: [], batches: [] })
    expect(store.batchId).toBe('')
    await store.reviewStatement({ id: 'current-statement', status: 'rejected' })
    expect(evidence.reviewStatement).toHaveBeenCalledWith(
      { id: 'current-statement', status: 'rejected' },
      'current'
    )
    expect(evidence.createPilot).not.toHaveBeenCalled()
  })

  it('blocks scope changes during an in-flight request so its response cannot enter another scope', async () => {
    let finish
    evidence.snapshot.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    const loading = store.load()
    expect(store.busy).toBe(true)
    expect(await store.switchScope('current')).toBe(false)
    expect(store.scope).toBe('pilot')
    finish(snapshot('bryan-pilot'))
    await loading
    expect(store.snapshot.subject_id).toBe('bryan-pilot')
    expect(evidence.snapshot).toHaveBeenCalledTimes(1)
    await store.switchScope('current')
    expect(store.snapshot.subject_id).toBe('current-person')
  })

  it('shows current scope explicitly and withholds personal forms until target mapping exists', async () => {
    evidence.snapshot.mockResolvedValue(snapshot('unconfigured'))
    const wrapper = mount(EvidenceWorkspace, {
      props: { tab: 'interview' },
      global: {
        stubs: {
          RouterLink: true,
          EvidenceRelationships: { template: '<div>Document relationship graph</div>' },
        },
      },
    })
    await wrapper.find('select').setValue('current')
    await flushPromises()
    expect(wrapper.text()).toContain('Current vault evidence')
    expect(wrapper.text()).toContain('Map the target person explicitly during import')
    expect(wrapper.find('textarea').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Set up / open pilot')
    await wrapper.setProps({ tab: 'relationships' })
    expect(wrapper.text()).toContain('Document relationship graph')
    expect(evidence.createPilot).not.toHaveBeenCalled()
  })
})
