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

  it('renders all selected model chips in the wrapping selection area when more than three models are selected', () => {
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
    expect(wrapper.find('.selected-tags').text()).toContain('Llama 3.1 70B Instruct')
    expect(tags[0].attributes('title')).toBeTruthy()
  })

  it('removes a selected model when the chip close button is clicked', async () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: ['openai/gpt-4o', 'anthropic/claude-3.5-sonnet']
      }
    })

    await wrapper.find('.tag-remove').trigger('click')

    expect(wrapper.emitted('update:modelValue')).toEqual([
      [['anthropic/claude-3.5-sonnet']]
    ])
  })

  it('applies a preset by replacing the current selection with valid models', async () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: [],
        presets: [
          {
            id: 'preset-1',
            name: 'Fast',
            model_ids: ['openai/gpt-4o', 'google/gemini-1.5-pro']
          }
        ]
      }
    })

    await wrapper.find('.preset-chip').trigger('click')

    expect(wrapper.emitted('update:modelValue')).toEqual([
      [['openai/gpt-4o', 'google/gemini-1.5-pro']]
    ])
  })

  it('disables a preset when none of its models are available in the current picker', () => {
    wrapper = mount(ModelSelector, {
      props: {
        models: [models[0]],
        modelValue: [],
        presets: [
          {
            id: 'preset-1',
            name: 'Unavailable',
            model_ids: ['google/gemini-1.5-pro']
          }
        ]
      }
    })

    const button = wrapper.find('.preset-chip')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.attributes('title')).toContain('No models')
  })

  it('emits create-preset for the current selection and rejects duplicate names', async () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: ['openai/gpt-4o'],
        presets: [
          {
            id: 'preset-1',
            name: 'Quality',
            model_ids: ['anthropic/claude-3.5-sonnet']
          }
        ]
      }
    })

    await wrapper.find('.save-current-btn').trigger('click')
    await wrapper.find('.preset-input').setValue('Fast Trio')
    await wrapper.find('.preset-editor .btn-primary').trigger('click')

    expect(wrapper.emitted('create-preset')).toEqual([
      [{ name: 'Fast Trio', modelIds: ['openai/gpt-4o'] }]
    ])

    await wrapper.find('.save-current-btn').trigger('click')
    await wrapper.find('.preset-input').setValue('quality')
    await wrapper.find('.preset-editor .btn-primary').trigger('click')

    expect(wrapper.find('.preset-error').text()).toContain('unique')
  })

  it('emits update-preset when the active preset is edited with the current selection', async () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: ['openai/gpt-4o', 'google/gemini-1.5-pro'],
        presets: [
          {
            id: 'preset-1',
            name: 'Starter',
            model_ids: ['openai/gpt-4o']
          }
        ]
      }
    })

    await wrapper.find('.preset-chip').trigger('click')
    await wrapper.find('.preset-manage-btn').trigger('click')

    expect(wrapper.emitted('update-preset')).toEqual([
      [{ id: 'preset-1', modelIds: ['openai/gpt-4o', 'google/gemini-1.5-pro'] }]
    ])
  })

  it('targets a preset on first click without replacing an existing custom selection', async () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: ['google/gemini-1.5-pro'],
        presets: [
          {
            id: 'preset-1',
            name: 'Starter',
            model_ids: ['openai/gpt-4o']
          }
        ]
      }
    })

    await wrapper.find('.preset-chip').trigger('click')

    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect(wrapper.find('.preset-helper').text()).toContain('Updating "Starter"')
  })

  it('renders presets inline before the popular action and keeps edit/delete at the end', () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: [],
        presets: [
          {
            id: 'preset-1',
            name: 'Starter',
            model_ids: ['openai/gpt-4o']
          }
        ]
      }
    })

    const quickActions = wrapper.find('.quick-actions')
    expect(quickActions.find('.preset-inline').exists()).toBe(true)
    expect(quickActions.element.firstElementChild.className).toContain('preset-inline')
    expect(quickActions.text()).toContain('Popular')
    expect(quickActions.text()).toContain('Update')
    expect(quickActions.text()).toContain('Delete')
    expect(quickActions.text()).toContain('New Preset')
  })

  it('disables saving when the preset limit is reached', () => {
    wrapper = mount(ModelSelector, {
      props: {
        models,
        modelValue: ['openai/gpt-4o'],
        presets: Array.from({ length: 8 }, (_, index) => ({
          id: `preset-${index}`,
          name: `Preset ${index}`,
          model_ids: ['openai/gpt-4o']
        }))
      }
    })

    const saveButton = wrapper.find('.save-current-btn')
    expect(saveButton.attributes('disabled')).toBeDefined()
    expect(saveButton.attributes('title')).toContain('Maximum 8 presets')
    expect(wrapper.find('.preset-helper').text()).toContain('Maximum 8 presets reached')
  })
})
