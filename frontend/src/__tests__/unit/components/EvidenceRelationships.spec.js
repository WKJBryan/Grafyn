import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useEvidenceStore } from '@/stores/evidence'
import EvidenceRelationships from '@/components/twin/EvidenceRelationships.vue'
import { evidence } from '@/api/evidence'

const receipt = {
  source_id: 'a',
  source_revision: 1,
  quote: 'Exact passage',
  locator: 'Paragraph 1',
}
const relation = (id, from, to, extra = {}) => ({
  id,
  subject_id: 'pilot',
  from_id: from,
  to_id: to,
  relation: 'related',
  similarity: 0.8,
  review_status: 'tentative',
  invalidated: false,
  from_receipt: receipt,
  to_receipt: receipt,
  ...extra,
})
const graphStub = { name: 'GraphView', props: ['evidenceData'], template: '<div />' }
let store
beforeEach(() => {
  setActivePinia(createPinia())
  store = useEvidenceStore()
  store.snapshot = {
    subject_id: 'pilot',
    subject_name: 'Pilot',
    sources: [
      { id: 'a', title: 'Alpha' },
      { id: 'b', title: 'Beta' },
      { id: 's', title: 'Shared passage' },
      { id: 'one', title: 'First neighbor' },
      { id: 'two', title: 'Second neighbor' },
    ],
    statements: [],
    nodes: [],
    goals: [],
    cases: [],
    relationships: [
      relation('as', 'a', 's'),
      relation('a1', 'a', 'one'),
      relation('bs', 'b', 's'),
      relation('b2', 'b', 'two'),
      relation('bad', 'a', 'bad', { review_status: 'rejected' }),
      relation('old', 'a', 'old', { invalidated: true }),
    ],
  }
})
const mountGraph = () =>
  mount(EvidenceRelationships, { global: { stubs: { GraphView: graphStub } } })
afterEach(() => vi.restoreAllMocks())

describe('expandable evidence groups', () => {
  it('separates assessed relationships from withheld and unassessed candidates', async () => {
    store.snapshot.assessment_status = 'Scoring 2 candidate pairs locally'
    store.snapshot.relationships = [
      relation('accepted', 'a', 's', {
        provenance: 'local_semantic_similarity',
        assessment: {
          model_version: 'qwen3.6:27b@sha256:abc',
          result: {
            verdict: 'equivalent',
            direction: 'symmetric',
            explanation: 'The passages express the same preference.',
            conditions: [],
            from_quote: 'Exact passage',
            to_quote: 'Exact passage',
          },
        },
      }),
      relation('withheld', 'a', 'one', {
        provenance: 'local_semantic_similarity',
        assessment: {
          model_version: 'qwen3.6:27b@sha256:abc',
          result: {
            verdict: 'unrelated',
            direction: 'symmetric',
            explanation: 'Only a generic topic overlaps.',
            conditions: [],
            from_quote: 'Exact passage',
            to_quote: 'Exact passage',
          },
        },
      }),
      relation('pending', 'b', 'two', {
        provenance: 'local_semantic_similarity',
        assessment: null,
      }),
      relation('stale', 'b', 'one', {
        provenance: 'local_semantic_similarity',
        assessment: {
          stale: true,
          result: { verdict: 'equivalent', explanation: 'Previously matched.' },
        },
      }),
      relation('review-stale', 'a', 'two', {
        provenance: 'local_semantic_similarity',
        review_status: 'confirmed',
        review_stale: true,
        assessment: { stale: false, result: { verdict: 'related' } },
      }),
    ]
    const wrapper = mountGraph()
    const graph = wrapper.findComponent({ name: 'GraphView' }).props('evidenceData')
    expect(graph.links.map((edge) => edge.id)).toEqual(['accepted'])
    expect(wrapper.text()).toContain('Contextual assessment: Scoring 2 candidate pairs locally')
    expect(wrapper.text()).toContain('Withheld candidates (4)')
    expect(wrapper.text()).toContain('Only a generic topic overlaps.')
    expect(wrapper.text()).toContain('Awaiting contextual assessment')
    expect(wrapper.text()).toContain('Context changed; assessment pending')
    expect(wrapper.text()).toContain('Context changed; review this relationship again')
    expect(wrapper.text()).not.toContain('human rejected')
  })

  it('keeps confirmed overrides visible and gives effects neutral goal-path distance', async () => {
    store.snapshot.relationships = [
      relation('override', 'a', 'one', {
        provenance: 'local_semantic_similarity',
        review_status: 'confirmed',
        assessment: { result: { verdict: 'unrelated', direction: 'symmetric' } },
      }),
      relation('effect', 'a', 'two', {
        relation: 'enables',
        similarity: 0.97,
        from_id: 'two',
        to_id: 'a',
        directed: true,
        assessment: {
          result: { verdict: 'enables', direction: 'right_to_left' },
        },
      }),
    ]
    const wrapper = mountGraph()
    expect(wrapper.findComponent({ name: 'GraphView' }).props('evidenceData').links[0].id).toBe(
      'override'
    )
    await wrapper.find('select').setValue('goal_path')
    const effect = wrapper.findComponent({ name: 'GraphView' }).props('evidenceData').links[0]
    expect(effect.id).toBe('effect')
    expect(effect.score).toBeNull()
    expect(effect.similarity).toBe(0.97)
    expect([effect.source, effect.target]).toEqual(['two', 'a'])
    expect(effect.directed).toBe(true)
  })

  it('rejects a personal statement without altering its original quote', async () => {
    const statement = {
      id: 'statement',
      subject_id: 'pilot',
      statement: 'Personal interpretation',
      kind: 'preference',
      review_status: 'tentative',
      receipts: [receipt],
    }
    store.snapshot.statements = [statement]
    const review = vi
      .spyOn(evidence, 'reviewStatement')
      .mockResolvedValue({ ...statement, review_status: 'rejected' })
    vi.spyOn(evidence, 'snapshot').mockResolvedValue({
      ...store.snapshot,
      statements: [{ ...statement, review_status: 'rejected' }],
    })
    const wrapper = mountGraph()
    expect(wrapper.text()).toContain('Personal interpretation')
    await wrapper
      .findAll('button')
      .find((button) => button.text() === 'Reject statement')
      .trigger('click')
    await flushPromises()
    expect(review).toHaveBeenCalledWith({ id: 'statement', status: 'rejected' }, 'pilot')
    expect(wrapper.text()).not.toContain('Personal interpretation')
    expect(store.snapshot.statements[0].receipts[0].quote).toBe('Exact passage')
  })
  it('expands members and graph edges, while a passage retains overlapping group membership', async () => {
    const wrapper = mountGraph()
    const alpha = wrapper
      .findAll('button')
      .find((button) => button.text().startsWith('Related to Alpha'))
    await alpha.trigger('click')
    expect(alpha.attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('[aria-label="Members related to Alpha"]').text()).toContain(
      'Shared passage'
    )
    expect(wrapper.find('[aria-label="Members related to Alpha"]').text()).toContain(
      'appears in 3 groups'
    )
    let graph = wrapper.findComponent({ name: 'GraphView' }).props('evidenceData')
    expect(graph.nodes.map((node) => node.id).sort()).toEqual(['a', 'one', 's'])
    expect(graph.links.map((edge) => edge.id).sort()).toEqual(['a1', 'as'])
    await wrapper
      .findAll('button')
      .find((button) => button.text().startsWith('Related to Beta'))
      .trigger('click')
    graph = wrapper.findComponent({ name: 'GraphView' }).props('evidenceData')
    expect(graph.nodes.map((node) => node.id).sort()).toEqual(['b', 's', 'two'])
    expect(wrapper.find('[aria-label="Members related to Beta"]').text()).toContain(
      'Shared passage'
    )
  })

  it('excludes rejected goals and future-effective goals from default goal graph and its links', async () => {
    const goal = (id, extra = {}) => ({
      id,
      subject_id: 'pilot',
      label: id,
      revision: 1,
      recorded_at: '2000-01-01T00:00:00Z',
      review_status: 'confirmed',
      ...extra,
    })
    store.snapshot.goals = [
      goal('Current goal'),
      goal('Rejected goal', { review_status: 'rejected' }),
      goal('Future goal', { effective_at: '2999-01-01T00:00:00Z' }),
    ]
    store.snapshot.relationships = store.snapshot.goals.map((goal, index) =>
      relation(`g${index}`, 'a', goal.id, { relation: 'contributes_to' })
    )
    const wrapper = mountGraph()
    await wrapper.find('select').setValue('goal_path')
    const graph = wrapper.findComponent({ name: 'GraphView' }).props('evidenceData')
    expect(graph.nodes.map((node) => node.id).sort()).toEqual(['Current goal', 'a'])
    expect(graph.links.map((edge) => edge.id)).toEqual(['g0'])
    expect(wrapper.text()).not.toContain('Rejected goal')
    expect(wrapper.text()).not.toContain('Future goal')
  })
})
