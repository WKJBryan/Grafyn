import { afterEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ModelSelector from '@/components/canvas/ModelSelector.vue'

const models = [
  { id: 'openai/gpt-4o', name: 'OpenAI: GPT-4o', provider: 'openai', context_length: 128000, pricing: { prompt: 0.000005, completion: 0.000015 } },
  { id: 'openai/gpt-4o-mini', name: 'OpenAI: GPT-4o Mini', provider: 'openai', context_length: 128000, pricing: { prompt: 0.000001, completion: 0.000003 } },
  { id: 'anthropic/claude-3.5-sonnet', name: 'Anthropic: Claude 3.5 Sonnet', provider: 'anthropic', context_length: 200000, pricing: { prompt: 0.000003, completion: 0.000015 } },
  { id: 'google/gemini-1.5-pro', name: 'Google: Gemini 1.5 Pro', provider: 'google', context_length: 1000000, pricing: { prompt: 0.000002, completion: 0.00001 } },
  { id: 'meta-llama/llama-3.1-70b-instruct', name: 'Meta: Llama 3.1 70B Instruct', provider: 'meta', context_length: 128000, pricing: { prompt: 0.0000015, completion: 0.0000025 } }
]

describe('ModelSelector', () => {
  let wrapper

  afterEach(() => {
    wrapper?.unmount()
  })

  it('renders all selected model chips when more than three models are selected', () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: models.map(model => model.id)
      }
    })

    const tags = wrapper.findAll('.model-tag')
    expect(tags).toHaveLength(5)
    expect(wrapper.find('.selected-count').text()).toContain('5 selected')
    expect(wrapper.find('.selected-tags').text()).toContain('GPT-4o')
    expect(wrapper.find('.selected-tags').text()).toContain('Claude 3.5 Sonnet')
    expect(wrapper.find('.selected-tags').text()).toContain('Gemini 1.5 Pro')
  })
})
