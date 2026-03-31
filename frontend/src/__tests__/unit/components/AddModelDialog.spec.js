import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AddModelDialog from '@/components/canvas/AddModelDialog.vue'

function mountDialog(props = {}) {
  return mount(AddModelDialog, {
    props: {
      models: [
        { id: 'openai/gpt-4o', name: 'GPT-4o' },
        { id: 'anthropic/claude-3.5-sonnet', name: 'Claude 3.5 Sonnet' }
      ],
      presets: [],
      existingModelIds: [],
      ...props
    },
    global: {
      stubs: {
        ModelSelector: {
          props: ['modelValue', 'models', 'presets'],
          emits: ['update:modelValue', 'create-preset'],
          template: `
            <div>
              <div class="selector-presets">{{ presets.length }}</div>
              <div class="selector-models">{{ models.length }}</div>
              <div class="selector-selection">{{ modelValue.length }}</div>
              <button class="select-model-stub" @click="$emit('update:modelValue', ['anthropic/claude-3.5-sonnet'])" />
              <button class="create-preset-stub" @click="$emit('create-preset', { name: 'Compare', modelIds: ['anthropic/claude-3.5-sonnet'] })" />
            </div>
          `
        }
      }
    }
  })
}

describe('AddModelDialog', () => {
  it('passes presets into the selector and starts blank while excluding existing models', () => {
    const wrapper = mountDialog({
      presets: [{ id: 'preset-1', name: 'Compare', model_ids: ['anthropic/claude-3.5-sonnet'] }],
      existingModelIds: ['openai/gpt-4o']
    })

    expect(wrapper.find('.selector-presets').text()).toBe('1')
    expect(wrapper.find('.selector-models').text()).toBe('1')
    expect(wrapper.find('.selector-selection').text()).toBe('0')
  })

  it('forwards preset events and submits selected models', async () => {
    const wrapper = mountDialog()

    await wrapper.find('.create-preset-stub').trigger('click')
    await wrapper.find('.select-model-stub').trigger('click')
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('create-preset')).toEqual([
      [{ name: 'Compare', modelIds: ['anthropic/claude-3.5-sonnet'] }]
    ])
    expect(wrapper.emitted('submit')).toEqual([
      [['anthropic/claude-3.5-sonnet']]
    ])
  })
})
