import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: {} })
}))

vi.mock('@/api/client', () => ({
  twin: {
    getReview: vi.fn().mockResolvedValue([]),
    listMemoryDigest: vi.fn().mockResolvedValue([]),
    listConstitutionItems: vi.fn().mockResolvedValue([]),
    listActionGaps: vi.fn().mockResolvedValue([]),
    listDecisionEpisodes: vi.fn().mockResolvedValue([]),
    getConstitutionSetup: vi.fn().mockResolvedValue({}),
    getDecisionMirrorConfig: vi.fn().mockResolvedValue({
      preset: 'balanced',
      advanced_enabled: false,
      weights: {}
    }),
    updateDecisionOutcome: vi.fn().mockResolvedValue({})
  }
}))

import TwinReviewView from '@/views/TwinReviewView.vue'
import { twin } from '@/api/client'

function decisionItem(overrides = {}, episodeOverrides = {}) {
  return {
    episode: {
      id: 'episode-1',
      session_id: 'session-1',
      tile_id: 'tile-1',
      decision: 'Ship the importer before polish?',
      options: ['Ship now', 'Wait a sprint'],
      chosen_option: null,
      twin_prediction: null,
      agreement: null,
      correction_note: null,
      primitive_assessment: {},
      reflection_cards: [],
      ...episodeOverrides
    },
    reflection_cards: [],
    feedback_events: [],
    prediction_sealed: false,
    ...overrides
  }
}

async function mountWithDecisions(items) {
  twin.listDecisionEpisodes.mockResolvedValue(items)
  const wrapper = mount(TwinReviewView)
  await flushPromises()
  return wrapper
}

describe('TwinReviewView decision predictions', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    twin.getReview.mockResolvedValue([])
    twin.listMemoryDigest.mockResolvedValue([])
    twin.listConstitutionItems.mockResolvedValue([])
    twin.listActionGaps.mockResolvedValue([])
    twin.getConstitutionSetup.mockResolvedValue({})
    twin.getDecisionMirrorConfig.mockResolvedValue({
      preset: 'balanced',
      advanced_enabled: false,
      weights: {}
    })
    twin.updateDecisionOutcome.mockResolvedValue({})
  })

  it('shows the sealed badge and no prediction content before the outcome', async () => {
    const wrapper = await mountWithDecisions([
      decisionItem({ prediction_sealed: true })
    ])

    expect(wrapper.text()).toContain('Twin sealed a prediction')
    expect(wrapper.text()).not.toContain('Twin predicted')
    expect(wrapper.find('.prediction-reveal').exists()).toBe(false)
  })

  it('reveals the prediction with a Missed badge and correction input after a disagreeing outcome', async () => {
    const wrapper = await mountWithDecisions([
      decisionItem(
        { prediction_sealed: false },
        {
          chosen_option: 'Wait a sprint',
          agreement: false,
          twin_prediction: {
            predicted_option: 'Ship now',
            confidence: 0.8,
            rationale: 'Past launches favored speed.',
            parse_mode: 'json',
            model_id: 'test/model',
            context_version: 'ctx-test',
            sealed_at: '2026-06-12T00:00:00Z'
          }
        }
      )
    ])

    expect(wrapper.text()).toContain('Twin predicted')
    expect(wrapper.text()).toContain('Ship now')
    expect(wrapper.text()).toContain('80% confident')
    expect(wrapper.text()).toContain('Missed')
    expect(wrapper.text()).toContain('Past launches favored speed.')
    expect(wrapper.find('.correction-row input').exists()).toBe(true)
  })

  it('shows Matched badge without a correction input on agreement', async () => {
    const wrapper = await mountWithDecisions([
      decisionItem(
        { prediction_sealed: false },
        {
          chosen_option: 'Ship now',
          agreement: true,
          twin_prediction: {
            predicted_option: 'Ship now',
            confidence: 0.6,
            rationale: null,
            parse_mode: 'json',
            model_id: 'test/model',
            context_version: 'ctx-test',
            sealed_at: '2026-06-12T00:00:00Z'
          }
        }
      )
    ])

    expect(wrapper.text()).toContain('Matched my choice')
    expect(wrapper.find('.correction-row input').exists()).toBe(false)
  })

  it('includes chosen_option and correction_note in the outcome payload', async () => {
    const wrapper = await mountWithDecisions([
      decisionItem(
        { prediction_sealed: false },
        {
          chosen_option: 'Wait a sprint',
          agreement: false,
          twin_prediction: {
            predicted_option: 'Ship now',
            confidence: null,
            rationale: null,
            parse_mode: 'json',
            model_id: 'test/model',
            context_version: 'ctx-test',
            sealed_at: '2026-06-12T00:00:00Z'
          }
        }
      )
    ])

    await wrapper.find('.chosen-option-select').setValue('Wait a sprint')
    await wrapper.find('.correction-row input').setValue('Twin overweights speed.')
    await wrapper.find('.outcome-row button').trigger('click')
    await flushPromises()

    expect(twin.updateDecisionOutcome).toHaveBeenCalledWith(
      'episode-1',
      expect.objectContaining({
        chosen_option: 'Wait a sprint',
        correction_note: 'Twin overweights speed.'
      })
    )
  })

  it('keeps legacy episodes without options editable via free text', async () => {
    const wrapper = await mountWithDecisions([
      decisionItem({}, { options: [], chosen_option: 'Did something else' })
    ])

    const freeText = wrapper.find('.outcome-row input[placeholder="Chosen option"]')
    expect(freeText.exists()).toBe(true)
    expect(freeText.element.value).toBe('Did something else')
  })
})
