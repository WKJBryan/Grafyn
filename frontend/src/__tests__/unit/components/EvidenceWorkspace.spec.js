import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { evidence } from '@/api/evidence'
import { useEvidenceStore } from '@/stores/evidence'
import EvidenceInterview from '@/components/twin/EvidenceInterview.vue'
import EvidenceGoals from '@/components/twin/EvidenceGoals.vue'
import EvidencePrediction from '@/components/twin/EvidencePrediction.vue'
import RelationshipEvidencePanel from '@/components/twin/RelationshipEvidencePanel.vue'
import EvidencePredictionComparisons from '@/components/twin/EvidencePredictionComparisons.vue'
import EvidenceWorkspace from '@/components/twin/EvidenceWorkspace.vue'
import EvidencePredictionLedger from '@/components/twin/EvidencePredictionLedger.vue'

vi.mock('@/api/evidence', () => ({
  evidence: {
    snapshot: vi.fn(),
    saveInterview: vi.fn(),
    saveGoal: vi.fn(),
    predict: vi.fn(),
    recordChoice: vi.fn(),
    reviewRelationship: vi.fn(),
    listPredictions: vi.fn(),
    installEmbeddings: vi.fn(),
  },
}))
const snapshot = () => ({
  subject_id: 'bryan-pilot',
  subject_name: 'Bryan',
  interview_draft: null,
  sources: [],
  goals: [],
  cases: [],
  relationships: [],
  jobs: [],
})
let store
beforeEach(() => {
  vi.clearAllMocks()
  setActivePinia(createPinia())
  store = useEvidenceStore()
  store.snapshot = snapshot()
  evidence.snapshot.mockImplementation(async () => snapshot())
  evidence.listPredictions.mockResolvedValue({ records: [], batches: [] })
})
const button = (wrapper, label) => wrapper.findAll('button').find((item) => item.text() === label)

describe('explicit local embedding setup', () => {
  it('installs only after a user click and displays progress while the download runs', async () => {
    store.snapshot.embedding_status =
      'pending: embeddinggemma is not installed; no model was downloaded'
    let finish
    evidence.installEmbeddings.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    evidence.snapshot.mockResolvedValue({
      ...snapshot(),
      embedding_status: 'ready: embeddinggemma',
    })
    const wrapper = mount(EvidenceWorkspace, { props: { tab: 'goals' } })
    await flushPromises()
    expect(evidence.installEmbeddings).not.toHaveBeenCalled()
    await button(wrapper, 'Install embeddinggemma locally').trigger('click')
    expect(evidence.installEmbeddings).toHaveBeenCalledWith('pilot')
    expect(wrapper.text()).toContain('Downloading the embedding model into local Ollama')
    expect(button(wrapper, 'Installing embeddinggemma…').element.disabled).toBe(true)
    finish()
    await flushPromises()
    expect(button(wrapper, 'Install embeddinggemma locally')).toBeUndefined()
    expect(store.busy).toBe(false)
  })

  it.each([
    'ready: embeddinggemma',
    'pending: embeddinggemma has not been checked',
    'pending: local Ollama runtime unavailable',
  ])('does not offer a download for %s', (status) => {
    store.snapshot.embedding_status = status
    const wrapper = mount(EvidenceWorkspace, { props: { tab: 'goals' } })
    expect(button(wrapper, 'Install embeddinggemma locally')).toBeUndefined()
    expect(evidence.installEmbeddings).not.toHaveBeenCalled()
  })
})

describe('interrupted prediction resume', () => {
  it('resumes the same saved request and batch instead of creating a forecast', async () => {
    const record = {
      id: 'saved-pending',
      batch_id: 'frozen-batch',
      status: 'pending',
      sealed: true,
      human_choice: null,
      request: {
        domain: 'everyday',
        situation: 'Original situation',
        options: ['Both', 'Neither'],
        validation: true,
        clarifications: [{ question: 'What budget?', answer: 'Known budget' }],
      },
    }
    store.prediction = record
    evidence.predict.mockResolvedValue({ ...record, status: 'completed' })
    const wrapper = mount(EvidencePredictionLedger)
    await flushPromises()
    await button(wrapper, 'Resume pending comparisons').trigger('click')
    await flushPromises()
    expect(evidence.predict).toHaveBeenCalledWith(
      { ...record.request, id: 'saved-pending', batch_id: 'frozen-batch' },
      'pilot'
    )
    expect(button(wrapper, 'Resume pending comparisons')).toBeUndefined()
  })
})

describe('guided interview', () => {
  it('resumes the saved step and preserves distinctions in submitted evidence', async () => {
    store.snapshot.interview_draft = {
      id: 'draft1',
      step: 2,
      situation: 'A decision',
      domain: 'everyday',
      wanted: 'Rest',
      expected: 'Work',
      chosen: '',
      rationale: '',
      options: ['Rest', 'Work'],
      rejected: [],
      constraints: [],
      goal_ids: [],
    }
    const wrapper = mount(EvidenceInterview)
    expect(wrapper.text()).toContain('Step 3 of 4')
    await wrapper.findAll('textarea')[0].setValue('Rest')
    await wrapper.findAll('textarea')[1].setValue('Work')
    await button(wrapper, 'Save and pause').trigger('click')
    await flushPromises()
    expect(evidence.saveInterview).toHaveBeenCalledWith(
      expect.objectContaining({
        submit: false,
        draft: expect.objectContaining({
          id: 'draft1',
          domain: 'everyday',
          wanted: 'Rest',
          expected: 'Work',
          chosen: 'Rest',
          rejected: ['Work'],
          step: 2,
        }),
      }),
      'pilot'
    )
  })
})

describe('goal revision editor', () => {
  it('does not convert blank quantities into zero or invent a deadline', async () => {
    const wrapper = mount(EvidenceGoals)
    await button(wrapper, 'New goal').trigger('click')
    await wrapper.find('input').setValue('Reach people')
    await wrapper.find('textarea').setValue('Reach many people quickly')
    await button(wrapper, 'Add measurable criterion (optional)').trigger('click')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    const request = evidence.saveGoal.mock.calls[0][0]
    expect(request.criteria[0]).toMatchObject({
      target: null,
      deadline: null,
      observed_progress: null,
      field_provenance: { target: 'unknown', deadline: 'unknown' },
    })
    expect(request.effective_at).toBeNull()
  })
})

describe('prediction disclosure', () => {
  it('resumes a sealed saved decision using the original situation and domain', async () => {
    const record = {
      id: 'saved',
      batch_id: 'frozen-batch',
      sealed: true,
      validation: true,
      status: 'completed',
      request: {
        domain: 'everyday',
        situation: 'Which evening plan?',
        options: ['Stay in', 'Go out'],
        clarifications: [],
        validation: true,
      },
      questions: [],
    }
    evidence.listPredictions.mockResolvedValue({
      records: [record],
      batches: [{ id: 'frozen-batch', model: 'local', created_at: '2026-09-05' }],
    })
    const wrapper = mount(EvidencePrediction)
    await flushPromises()
    const selector = wrapper
      .findAll('select')
      .find((item) => item.find('option[value="saved"]').exists())
    await selector.setValue('saved')
    await flushPromises()
    expect(store.batchId).toBe('frozen-batch')
    expect(wrapper.findAll('textarea')[0].element.value).toBe('Which evening plan?')
    expect(wrapper.findAll('textarea')[1].element.value).toBe('Stay in\nGo out')
    expect(wrapper.text()).toContain('Prediction sealed')
    expect(store.predictionRequest.domain).toBe('everyday')
  })
  it('hides every forecast field while sealed and reveals only after choice', async () => {
    store.prediction = {
      id: 'p1',
      validation: true,
      sealed: true,
      proposed_action: 'SECRET CHOICE',
      conditional_branches: [{ condition: 'SECRET CONDITION', action: 'SECRET BRANCH' }],
      assumptions: ['SECRET ASSUMPTION'],
      evidence_ids: ['SECRET EVIDENCE'],
      questions: ['What is the budget?', 'When is it due?', 'Extra question?'],
    }
    evidence.recordChoice.mockResolvedValue({
      id: 'p1',
      validation: true,
      sealed: false,
      proposed_action: 'SECRET CHOICE',
    })
    const wrapper = mount(EvidencePrediction)
    expect(wrapper.text()).not.toContain('SECRET')
    expect(wrapper.text()).toContain('What is the budget?')
    expect(wrapper.text()).not.toContain('Extra question?')
    const choiceInput = wrapper.findAll('input').find((item) => item.element.required)
    await choiceInput.setValue('My choice')
    await wrapper.findAll('form').at(-1).trigger('submit')
    await flushPromises()
    expect(evidence.recordChoice).toHaveBeenCalledWith(
      { prediction_id: 'p1', choice: 'My choice', rationale: null },
      'pilot'
    )
    expect(wrapper.text()).toContain('SECRET CHOICE')
  })
  it('shows normal conditional branches and retains at most two questions', () => {
    store.prediction = {
      id: 'p2',
      validation: false,
      sealed: false,
      conditional_branches: [{ condition: 'budget is tight', action: 'defer' }],
      questions: ['One?', 'Two?', 'Three?'],
    }
    const wrapper = mount(EvidencePrediction)
    expect(wrapper.text()).toContain('If budget is tight: defer')
    expect(wrapper.text()).not.toContain('Three?')
  })
})

describe('human comparison review', () => {
  it('records condition and stage separately without rewriting the actual choice', async () => {
    const record = {
      id: 'p',
      sealed: false,
      human_choice: 'Both, but later',
      human_rationale: 'Timing',
      comparisons: [
        {
          condition: 'goal_paths',
          stage: 'before_clarification',
          status: 'completed',
          forecast: { proposed_action: 'Wait and combine', assumptions: [], evidence_ids: [] },
        },
        {
          condition: 'goal_paths',
          stage: 'after_clarification',
          status: 'completed',
          forecast: { proposed_action: 'Do both later', assumptions: [], evidence_ids: [] },
        },
      ],
      adjudications: {},
    }
    evidence.recordChoice.mockResolvedValue(record)
    const wrapper = mount(EvidencePredictionComparisons, { props: { record } })
    await wrapper.findAll('select')[0].setValue('ambiguous')
    await wrapper.findAll('select')[1].setValue('agree')
    await button(wrapper, 'Save my judgements').trigger('click')
    await flushPromises()
    expect(evidence.recordChoice).toHaveBeenCalledWith(
      {
        prediction_id: 'p',
        choice: 'Both, but later',
        rationale: 'Timing',
        adjudications: {
          'goal_paths:before_clarification': 'ambiguous',
          'goal_paths:after_clarification': 'agree',
        },
      },
      'pilot'
    )
  })
})

describe('relationship review', () => {
  it('shows contextual verdict, explanation, model identity and assessment quotes', () => {
    const receipt = {
      source_id: 'source', source_revision: 1, start: 0, end: 10,
      locator: 'Paragraph 1', quote: 'Original first passage',
    }
    const wrapper = mount(RelationshipEvidencePanel, {
      props: { relationship: {
        id: 'assessed', relation: 'equivalent', review_status: 'tentative',
        provenance: 'local_semantic_similarity', similarity: 0.84,
        from_receipt: receipt, to_receipt: { ...receipt, quote: 'Original second passage' },
        assessment: {
          model_version: 'qwen3.6:27b@sha256:abc', prompt_version: 'relationship-v1',
          result: { verdict: 'equivalent', direction: 'right_to_left',
            explanation: 'Same commitment in different words.', conditions: ['Same timeframe'],
            from_quote: 'Assessed first quote', to_quote: 'Assessed second quote' },
        },
      } },
    })
    const text = wrapper.text()
    expect(text).toContain('Contextual verdict: equivalent')
    expect(text).toContain('RIGHT → LEFT (assessment input order)')
    expect(text).toContain('Same commitment in different words.')
    expect(text).toContain('qwen3.6:27b@sha256:abc')
    expect(text).toContain('Original first passage')
    expect(text).toContain('Original second passage')
    expect(text).toContain('Assessed first quote')
    expect(text).toContain('Assessment LEFT (input order)')
    expect(text).toContain('Assessment RIGHT (input order)')
    expect(text).toContain('Candidate relatedness 0.84')
    expect(text).not.toContain('request_payload')
  })

  it('disables equivalent and conflicts corrections when scope conditions are absent', () => {
    const receipt = { source_id: 'source', source_revision: 1, quote: 'Exact passage' }
    const wrapper = mount(RelationshipEvidencePanel, {
      props: { relationship: {
        id: 'unscoped', relation: 'related', review_status: 'tentative',
        provenance: 'imported', similarity: null, conditions: [],
        from_receipt: receipt, to_receipt: receipt,
      } },
    })
    for (const kind of ['equivalent', 'conflicts']) {
      const option = wrapper.findAll('option').find((item) => item.text() === kind)
      expect(option.attributes('disabled')).toBeDefined()
    }
    expect(wrapper.text()).toContain('Equivalent and conflicts corrections require scope conditions')
  })

  it('disables effect corrections when a directed relation lacks an effect basis', () => {
    const receipt = { source_id: 'source', source_revision: 1, quote: 'Exact passage' }
    const wrapper = mount(RelationshipEvidencePanel, {
      props: { relationship: {
        id: 'directed-support', relation: 'supports', directed: true, causal_basis: null,
        review_status: 'tentative', provenance: 'imported', similarity: null,
        conditions: ['Same scope'], from_receipt: receipt, to_receipt: receipt,
      } },
    })
    for (const kind of ['enables', 'inhibits', 'requires', 'contributes_to']) {
      const option = wrapper.findAll('option').find((item) => item.text() === kind)
      expect(option.attributes('disabled')).toBeDefined()
    }
    expect(wrapper.text()).toContain('Effect corrections require an existing direction and effect basis')
  })

  it('shows both exact receipts and corrects the relation without changing source text', async () => {
    const receipt = {
      source_id: 'source',
      source_revision: 1,
      start: 0,
      end: 10,
      locator: 'Paragraph 1',
      quote: 'Exact first passage',
    }
    const wrapper = mount(RelationshipEvidencePanel, {
      props: {
        relationship: {
          id: 'r1',
          relation: 'enables',
          review_status: 'tentative',
          provenance: 'inferred',
          similarity: null,
          causal_basis: 'target_stated_belief',
          from_receipt: receipt,
          to_receipt: { ...receipt, quote: 'Exact second passage' },
          conditions: [],
        },
      },
    })
    expect(wrapper.text()).toContain('Exact first passage')
    expect(wrapper.text()).toContain('Exact second passage')
    expect(wrapper.text()).toContain('target stated belief')
    expect(wrapper.text()).toContain('Unscored / neutral length')
    await wrapper.find('select').setValue('inhibits')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(evidence.reviewRelationship).toHaveBeenCalledWith(
      { id: 'r1', status: 'confirmed', relation: 'inhibits' },
      'pilot'
    )
  })
})
