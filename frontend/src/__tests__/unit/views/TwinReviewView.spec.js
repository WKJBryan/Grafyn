import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import TwinReviewView from '@/views/TwinReviewView.vue'

const api = vi.hoisted(() => ({
  getReview: vi.fn(),
  runInference: vi.fn(),
  resolveEvidence: vi.fn(),
  setPromotion: vi.fn(),
  listMemoryDigest: vi.fn(),
  reviewMemoryDigestItem: vi.fn(),
  listConstitutionItems: vi.fn(),
  reviewConstitutionItem: vi.fn(),
  listActionGaps: vi.fn(),
  reviewActionGap: vi.fn(),
  listDecisionEpisodes: vi.fn(),
  updateDecisionOutcome: vi.fn(),
  getConstitutionSetup: vi.fn(),
  saveConstitutionSetup: vi.fn(),
  runConstitutionInference: vi.fn(),
  getDecisionMirrorConfig: vi.fn(),
  updateDecisionMirrorConfig: vi.fn(),
  resetDecisionMirrorConfig: vi.fn(),
  exportData: vi.fn()
}))
const routeState = vi.hoisted(() => ({ query: {} }))

vi.mock('vue-router', () => ({
  useRoute: () => routeState
}))

vi.mock('@/api/client', () => ({
  twin: api
}))

function mountView() {
  return mount(TwinReviewView, {
    global: {
      stubs: {
        RouterLink: {
          props: ['to'],
          template: '<a><slot /></a>'
        }
      }
    }
  })
}

describe('TwinReviewView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    routeState.query = {}
    localStorage.clear()
    api.getReview.mockResolvedValue([
      {
        record: {
          id: 'record-1',
          kind: 'preference',
          content: 'Prefers evidence-backed implementation detail.',
          confidence: 0.8,
          origin: 'inferred',
          promotion_state: 'candidate',
          metadata: { signal_family: 'explicit_feedback' }
        },
        evidence_count: 2,
        latest_evidence: { summary: 'accept feedback recorded' }
      }
    ])
    api.listMemoryDigest.mockResolvedValue([
      {
        id: 'digest-1',
        pattern: 'User benefits from hard go/no-go gates before scaling.',
        evidence_count: 3,
        confidence: 0.82,
        state: 'pending',
        trigger_reason: '3+ evidence points'
      }
    ])
    api.listConstitutionItems.mockResolvedValue([
      {
        id: 'constitution-1',
        claim: 'Validate single-user value before topology.',
        dimension: 'values',
        confidence: 0.86,
        priority: 0.9,
        status: 'candidate',
        source: 'behavior_inference',
        evidence_refs: [{
          event_id: 'evt-1',
          source_type: 'behavior',
          source_label: 'Prompt submitted'
        }],
        scope: ['research']
      }
    ])
    api.listActionGaps.mockResolvedValue([
      {
        id: 'gap-1',
        stated_value: 'Validate first',
        revealed_behavior: 'Jumps to topology',
        driver_hypothesis: 'Architecture novelty pull',
        somatic_taste_signal: 'Excitement',
        decision_risk: 'May overbuild before proof',
        confidence: 0.7,
        status: 'candidate',
        evidence_refs: [{ event_id: 'evt-2' }]
      }
    ])
    api.listDecisionEpisodes.mockResolvedValue([
      {
        episode: {
          id: 'decision-1',
          decision: 'Build the Twin Constitution?',
          options: ['yes', 'later'],
          review_date: '2026-05-15',
          primitive_assessment: {
            stakes: 'high',
            action_gap_risk: 'medium'
          }
        },
        reflection_cards: [
          {
            id: 'card-1',
            content: '## Decision Frame\nBuild the workspace first.',
            scores: {
              breadth_score: 0.8,
              depth_score: 0.7,
              evidence_grounding_score: 0.9,
              blind_spot_score: 0.75,
              actionability_score: 0.8,
              counterargument_score: 0.7,
              uncertainty_score: 0.65,
              privacy_score: 1,
              unsupported_claim_count: 1,
              overall_score: 0.82
            },
            evidence_packet: {
              selected_sources: [
                {
                  source_type: 'constitution_item',
                  id: 'constitution-1',
                  label: 'Validate single-user value before topology.',
                  weight: 1.35,
                  reason: 'Selected because the decision is about product sequencing.'
                },
                {
                  source_type: 'action_gap',
                  id: 'gap-1',
                  label: 'May overbuild before proof',
                  weight: 1.25,
                  reason: 'Selected as a follow-through risk.'
                }
              ],
              excluded_private_count: 1,
              excluded_rejected_count: 2,
              excluded_no_train_count: 1,
              config_snapshot: {
                preset: 'evidence_strict',
                advanced_enabled: false,
                weights: { constitution_weight: 1.35 }
              }
            }
          }
        ],
        feedback_events: [
          {
            id: 'feedback-1',
            event_type: 'feedback_recorded',
            created_at: '2026-05-08T00:00:00Z',
            payload: {
              feedback_type: 'reject',
              rationale: 'Decision Mirror reflection marked Not Me'
            }
          }
        ]
      }
    ])
    api.getConstitutionSetup.mockResolvedValue({
      twin_name: 'Alex Chen',
      twin_role: 'founder deciding from product evidence',
      source_boundaries: ['reviewed notes only'],
      values: ['evidence-backed work'],
      tastes: ['clean UX'],
      constraints: [],
      somatic_cues: [],
      action_tendencies: []
    })
    api.getDecisionMirrorConfig.mockResolvedValue({
      preset: 'balanced',
      advanced_enabled: false,
      weights: {
        notes_weight: 1,
        approved_records_weight: 1,
        candidate_records_weight: 0.6,
        constitution_weight: 1.25,
        action_gaps_weight: 1.2,
        breadth_weight: 1,
        depth_weight: 1,
        evidence_grounding_weight: 1.25,
        blind_spot_weight: 1,
        actionability_weight: 1
      }
    })
    api.runInference.mockResolvedValue({ created_records: 1, updated_records: 1 })
    api.runConstitutionInference.mockResolvedValue({
      created_constitution_items: 1,
      created_action_gaps: 1,
      scanned_behavior_events: 6,
      scanned_notes: 3,
      scanned_interviews: 1,
      auto_active_items: 1,
      review_candidate_items: 2,
      extracted_research_findings: 1,
      pruned_stale_constitution_items: 3,
      pruned_stale_records: 2,
      updated_setup_entries: 5,
      skipped_domain_claims: 4
    })
    api.resolveEvidence.mockResolvedValue([
      {
        event_id: 'evt-1',
        event_type: 'feedback_recorded',
        created_at: '2026-05-06T00:00:00Z',
        session_id: 'session-1',
        model_id: 'openai/gpt-4o',
        prompt_excerpt: 'Implement this with tests.',
        response_excerpt: 'Changed src/file.rs and ran cargo test.'
      }
    ])
    api.setPromotion.mockResolvedValue({})
    api.reviewConstitutionItem.mockResolvedValue({})
    api.reviewActionGap.mockResolvedValue({})
    api.reviewMemoryDigestItem.mockResolvedValue({})
    api.updateDecisionOutcome.mockResolvedValue({})
    api.saveConstitutionSetup.mockResolvedValue({})
    api.updateDecisionMirrorConfig.mockResolvedValue({
      preset: 'evidence_strict',
      advanced_enabled: false,
      weights: { evidence_grounding_weight: 1.8 }
    })
    api.resetDecisionMirrorConfig.mockResolvedValue({
      preset: 'balanced',
      advanced_enabled: false,
      weights: { evidence_grounding_weight: 1.25 }
    })
    api.exportData.mockResolvedValue({
      decision_mirror_benchmark: { count: 1 }
    })
  })

  it('loads the Twin Workspace overview with constitution, gaps, and decisions', async () => {
    const wrapper = mountView()
    await flushPromises()

    expect(api.listConstitutionItems).toHaveBeenCalled()
    expect(api.listActionGaps).toHaveBeenCalled()
    expect(api.listDecisionEpisodes).toHaveBeenCalled()
    expect(wrapper.text()).toContain('Twin Workspace')
    expect(wrapper.text()).toContain('Build the Twin Constitution?')
    expect(wrapper.text()).toContain('May overbuild before proof')
  })

  it('shows context trace details for a decision reflection card', async () => {
    routeState.query = { decision: 'decision-1', trace: '1' }
    const wrapper = mountView()
    await flushPromises()

    const trace = wrapper.find('.context-trace-details')
    expect(trace.exists()).toBe(true)
    expect(trace.attributes('open')).toBeDefined()
    expect(wrapper.text()).toContain('Context Trace')
    expect(wrapper.text()).toContain('Stricter Evidence')
    expect(wrapper.text()).toContain('Validate single-user value before topology.')
    expect(wrapper.text()).toContain('Excluded 4')
    expect(wrapper.text()).toContain('Decision Mirror reflection marked Not Me')
  })

  it('reviews constitution items from the Constitution tab', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('.tab-button').find(button => button.text().includes('Constitution')).trigger('click')
    await flushPromises()
    await wrapper.findAll('.review-actions button').find(button => button.text() === 'Keep').trigger('click')
    await flushPromises()

    expect(api.reviewConstitutionItem).toHaveBeenCalledWith('constitution-1', { action: 'keep' })
  })

  it('shows constitution source labels and expanded inference summary', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('.tab-button').find(button => button.text().includes('Constitution')).trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Behavior Inference')
    expect(wrapper.text()).toContain('Prompt submitted')

    await wrapper.findAll('button').find(button => button.text() === 'Run Constitution').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('1 active')
    expect(wrapper.text()).toContain('6 behavior events')
    expect(wrapper.text()).toContain('1 interviews')
    expect(wrapper.text()).toContain('1 findings')
    expect(wrapper.text()).toContain('3 stale items removed')
    expect(wrapper.text()).toContain('2 stale records removed')
    expect(wrapper.text()).toContain('5 setup entries')
  })

  it('opens the evidence drawer for a user record', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('.tab-button').find(button => button.text().includes('Memory Review')).trigger('click')
    await flushPromises()
    await wrapper.findAll('button').find(button => button.text() === 'Evidence').trigger('click')
    await flushPromises()

    expect(api.resolveEvidence).toHaveBeenCalledWith('record-1')
    expect(wrapper.text()).toContain('Implement this with tests.')
    expect(wrapper.text()).toContain('openai/gpt-4o')
  })

  it('saves setup cards as constitution seed evidence', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('.tab-button').find(button => button.text().includes('Setup')).trigger('click')
    await flushPromises()
    const textareas = wrapper.findAll('textarea')
    expect(wrapper.text()).toContain('Twin Identity')
    expect(wrapper.html().indexOf('Twin Identity')).toBeLessThan(wrapper.html().indexOf('Operating Priors'))
    await wrapper.find('input[aria-label="Twin name"]').setValue('Alex Chen')
    await wrapper.find('input[aria-label="Twin role"]').setValue('founder deciding from product evidence')
    await textareas[0].setValue('reviewed notes only\nuploaded interviews')
    await textareas[1].setValue('evidence-backed work\nfast feedback')
    await wrapper.findAll('button').find(button => button.text() === 'Save Setup').trigger('click')
    await flushPromises()

    expect(api.saveConstitutionSetup).toHaveBeenCalledWith(expect.objectContaining({
      twin_name: 'Alex Chen',
      twin_role: 'founder deciding from product evidence',
      source_boundaries: ['reviewed notes only', 'uploaded interviews'],
      values: ['evidence-backed work', 'fast feedback']
    }))
  })

  it('shows the tutorial, can dismiss it, and reopens the full guide', async () => {
    const wrapper = mountView()
    await flushPromises()

    expect(wrapper.text()).toContain('How To Use')
    await wrapper.findAll('button').find(button => button.text() === 'Dismiss').trigger('click')
    await flushPromises()

    expect(localStorage.getItem('grafyn.twinWorkspaceTutorial.dismissed')).toBe('true')
    await wrapper.findAll('.tab-button').find(button => button.text().includes('Guide')).trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Canvas Buttons')
    expect(wrapper.text()).toContain('Create Reflection Card')
    expect(wrapper.text()).toContain('Export Benchmark')
  })

  it('saves presets and exports the decision benchmark from Config', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('.tab-button').find(button => button.text().includes('Config')).trigger('click')
    await flushPromises()
    await wrapper.find('.config-row select').setValue('evidence_strict')
    await wrapper.findAll('button').find(button => button.text() === 'Save Config').trigger('click')
    await flushPromises()

    expect(api.updateDecisionMirrorConfig).toHaveBeenCalledWith({
      preset: 'evidence_strict',
      advanced_enabled: false
    })

    await wrapper.findAll('button').find(button => button.text() === 'Export Benchmark').trigger('click')
    await flushPromises()
    expect(api.exportData).toHaveBeenCalledWith({ bundle_name: 'decision-mirror-benchmark' })
  })
})
