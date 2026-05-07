import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import TwinReviewView from '@/views/TwinReviewView.vue'

const { getReview, runInference, resolveEvidence, setPromotion } = vi.hoisted(() => ({
  getReview: vi.fn(),
  runInference: vi.fn(),
  resolveEvidence: vi.fn(),
  setPromotion: vi.fn()
}))

vi.mock('@/api/client', () => ({
  twin: {
    getReview,
    runInference,
    resolveEvidence,
    setPromotion
  }
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
    getReview.mockResolvedValue([
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
        latest_evidence: {
          summary: 'accept feedback recorded'
        }
      },
      {
        record: {
          id: 'record-2',
          kind: 'reasoning_pattern',
          content: 'Compares multiple model outputs.',
          confidence: 0.9,
          origin: 'inferred',
          promotion_state: 'auto_promoted',
          metadata: { signal_family: 'model_selection' }
        },
        evidence_count: 3,
        latest_evidence: {
          summary: 'response ranking recorded'
        }
      }
    ])
    runInference.mockResolvedValue({ created_records: 1, updated_records: 1 })
    resolveEvidence.mockResolvedValue([
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
    setPromotion.mockResolvedValue({})
  })

  it('loads review records and filters by state', async () => {
    const wrapper = mountView()
    await flushPromises()

    expect(getReview).toHaveBeenCalled()
    expect(wrapper.text()).toContain('Prefers evidence-backed implementation detail.')
    expect(wrapper.text()).not.toContain('Compares multiple model outputs.')

    await wrapper.findAll('.state-filter').find(button => button.text().includes('Auto Promoted')).trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Compares multiple model outputs.')
  })

  it('opens the evidence drawer for a record', async () => {
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('button').find(button => button.text() === 'Evidence').trigger('click')
    await flushPromises()

    expect(resolveEvidence).toHaveBeenCalledWith('record-1')
    expect(wrapper.text()).toContain('Implement this with tests.')
    expect(wrapper.text()).toContain('openai/gpt-4o')
  })

  it('rejects a record through setPromotion', async () => {
    const promptSpy = vi.spyOn(globalThis, 'prompt').mockReturnValue('Not enough support')
    const wrapper = mountView()
    await flushPromises()

    await wrapper.findAll('button').find(button => button.text() === 'Reject').trigger('click')
    await flushPromises()

    expect(setPromotion).toHaveBeenCalledWith('record-1', 'rejected', 'Not enough support')
    promptSpy.mockRestore()
  })
})
