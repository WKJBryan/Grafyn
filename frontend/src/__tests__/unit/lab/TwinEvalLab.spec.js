import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import TwinEvalLab from '@/lab/TwinEvalLab.vue'

const twinEvalApi = vi.hoisted(() => ({
  getModelMatrix: vi.fn(),
  previewInput: vi.fn(),
  previewContext: vi.fn(),
  runLab: vi.fn(),
  exportResults: vi.fn()
}))

vi.mock('@/api/client', () => ({
  twinEval: twinEvalApi
}))

const sampleQuestion = `What is the most accurate description?

A) Vision-led
B) Co-created
C) Market-driven`

function mountLab() {
  return mount(TwinEvalLab)
}

describe('TwinEvalLab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    twinEvalApi.getModelMatrix.mockResolvedValue([
      {
        key: 'gemma4-e2b-it',
        label: 'Gemma 4 E2B IT',
        provider: 'ollama',
        runner_ready: true,
        quantization: 'Q4_K_M',
        runner_model_id: 'gemma4:e2b-q4_k_m',
        model_id: 'google/gemma-4-E2B-it',
        notes: 'Runnable'
      },
      {
        key: 'qwen-sealion-v4-5-27b-it',
        label: 'Qwen SEA-LION v4.5 27B IT',
        provider: 'ollama',
        runner_ready: false,
        quantization: 'Q4_K_M',
        runner_model_id: 'qwen-sealion-v4.5:27b-q4_k_m',
        model_id: 'aisingapore/Qwen-SEA-LION-v4.5-27B-IT',
        notes: 'Setup needed'
      }
    ])
    twinEvalApi.previewInput.mockResolvedValue({
      id: 'case-1',
      question: 'What is the most accurate description?',
      options: [
        { key: 'A', text: 'Vision-led' },
        { key: 'B', text: 'Co-created' },
        { key: 'C', text: 'Market-driven' }
      ],
      answer_key: null
    })
    twinEvalApi.previewContext.mockResolvedValue({
      mode: 'system_only',
      retrieval_items: [],
      constitution_items: [],
      action_gaps: [],
      system_prompt: 'Use the selected context only.',
      private_store_accessed: false
    })
    twinEvalApi.runLab.mockResolvedValue({
      question: { id: 'case-1' },
      context_packet: { mode: 'system_only', private_store_accessed: false },
      skipped_models: [],
      results: [
        {
          case_id: 'case-1',
          model_key: 'gemma4-e2b-it',
          selected_option: 'B',
          outside_options_answer: null,
          final_answer: 'B',
          confidence: 0.72,
          correctness_score: null,
          rationale: 'The change was co-created.',
          raw_response: '{"selected_option":"B"}',
          model_trace: 'visible trace',
          sycophancy_flag: false,
          context_citations: []
        }
      ]
    })
    twinEvalApi.exportResults.mockResolvedValue({ json: '[]', csv: 'case_id' })
  })

  it('loads as a standalone lab and does not mention the normal Grafyn route', async () => {
    const wrapper = mountLab()
    await flushPromises()

    expect(wrapper.text()).toContain('Grafyn Twin Eval Lab')
    expect(wrapper.text()).toContain('Selected context mode')
    expect(wrapper.text()).not.toContain('Notes')
    expect(wrapper.text()).not.toContain('Twin Workspace')
    expect(wrapper.text()).not.toContain('Max tokens')
  })

  it('previews raw MCQ input without requiring an answer key', async () => {
    const wrapper = mountLab()
    await flushPromises()

    await wrapper.find('[data-test="lab-question"]').setValue(sampleQuestion)
    await wrapper.find('[data-test="preview-input"]').trigger('click')
    await flushPromises()

    expect(twinEvalApi.previewInput).toHaveBeenCalledWith(sampleQuestion, null)
    expect(wrapper.text()).toContain('A. Vision-led')
    expect(wrapper.text()).toContain('No answer key')
  })

  it('previews selected context before running and sends custom protocol settings', async () => {
    const wrapper = mountLab()
    await flushPromises()

    await wrapper.find('[data-test="lab-question"]').setValue(sampleQuestion)
    await wrapper.find('[data-test="context-mode"]').setValue('retrieval_only')
    await wrapper.find('[data-test="show-trace"]').setValue(true)
    await wrapper.find('[data-test="preview-context"]').trigger('click')
    await flushPromises()

    expect(twinEvalApi.previewContext).toHaveBeenCalledWith(expect.objectContaining({
      raw_question: sampleQuestion,
      answer_key: null,
      context_mode: 'retrieval_only',
      system_prompt: null,
      show_reasoning_trace: true,
      structured_output: false
    }))

    await wrapper.find('[data-test="run-lab"]').trigger('click')
    await flushPromises()

    const runRequest = twinEvalApi.runLab.mock.calls.at(-1)[0]
    expect(runRequest).toEqual(expect.objectContaining({
      raw_question: sampleQuestion,
      answer_key: null,
      context_mode: 'retrieval_only',
      model_keys: ['gemma4-e2b-it'],
      system_prompt: null,
      show_reasoning_trace: true,
      structured_output: false
    }))
    expect(runRequest).not.toHaveProperty('max_tokens')
    expect(wrapper.text()).toContain('visible trace')
  })

it('uses structured JSON output only when explicitly enabled', async () => {
  const wrapper = mountLab()
  await flushPromises()

  await wrapper.find('[data-test="lab-question"]').setValue(sampleQuestion)
  await wrapper.find('[data-test="structured-output"]').setValue(true)
  await wrapper.find('[data-test="run-lab"]').trigger('click')
  await flushPromises()

  expect(twinEvalApi.runLab).toHaveBeenCalledWith(expect.objectContaining({
    structured_output: true
  }))
})

  it('only enables custom system prompt in system-only mode', async () => {
    const wrapper = mountLab()
    await flushPromises()

    const systemPrompt = wrapper.find('[data-test="system-prompt"]')
    expect(systemPrompt.element.disabled).toBe(false)

    await wrapper.find('[data-test="lab-question"]').setValue(sampleQuestion)
    await systemPrompt.setValue('custom protocol')
    await wrapper.find('[data-test="preview-context"]').trigger('click')
    await flushPromises()

    expect(twinEvalApi.previewContext).toHaveBeenLastCalledWith(expect.objectContaining({
      context_mode: 'system_only',
      system_prompt: 'custom protocol'
    }))

    await wrapper.find('[data-test="context-mode"]').setValue('constitution_only')
    expect(wrapper.find('[data-test="system-prompt"]').element.disabled).toBe(true)

    await wrapper.find('[data-test="preview-context"]').trigger('click')
    await flushPromises()

    expect(twinEvalApi.previewContext).toHaveBeenLastCalledWith(expect.objectContaining({
      context_mode: 'constitution_only',
      system_prompt: null
    }))
  })

  it('exports lab results through the backend exporter', async () => {
    const wrapper = mountLab()
    await flushPromises()

    await wrapper.find('[data-test="lab-question"]').setValue(sampleQuestion)
    await wrapper.find('[data-test="run-lab"]').trigger('click')
    await flushPromises()
    await wrapper.find('[data-test="export-json"]').trigger('click')

    expect(twinEvalApi.exportResults).toHaveBeenCalledWith([
      expect.objectContaining({ model_key: 'gemma4-e2b-it' })
    ])
  })
})
