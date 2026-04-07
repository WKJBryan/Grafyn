import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PromptDialog from '@/components/canvas/PromptDialog.vue'

function mountDialog(props = {}) {
  return mount(PromptDialog, {
    props: {
      models: [{ id: 'openai/gpt-4o', name: 'GPT-4o', context_length: 128000 }],
      ...props
    },
    global: {
      stubs: {
        ModelSelector: {
          props: ['modelValue', 'presets'],
          emits: ['update:modelValue', 'create-preset'],
          template: `
            <div>
              <div class="preset-props">{{ presets.length }}</div>
              <div class="selection-props">{{ modelValue.length }}</div>
              <button class="model-selector-stub" @click="$emit('update:modelValue', ['openai/gpt-4o'])" />
              <button class="create-preset-stub" @click="$emit('create-preset', { name: 'Fast trio', modelIds: ['openai/gpt-4o'] })" />
            </div>
          `
        },
        ContextBudgetDisplay: {
          template: '<div class="budget-stub" />'
        }
      }
    }
  })
}

describe('PromptDialog', () => {
  it('clarifies that vault notes context is not live web search', () => {
    const wrapper = mountDialog()

    expect(wrapper.text()).toContain('Vault Notes (relevant notes)')
    expect(wrapper.find('.context-mode-hint').text()).toContain('This does not search the live web')
  })

  it('submits web search enabled for freshness-sensitive prompts when Canvas auto-search is on', async () => {
    const wrapper = mountDialog({
      smartWebSearch: true,
      openRouterConfigured: true
    })

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('What is the latest version of Node.js?')
    await wrapper.vm.$nextTick()
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0].webSearch).toBe(true)
    expect(wrapper.find('.web-search-hint').text()).toContain('Live web search will run for this prompt')
  })

  it('keeps web search off for self-contained prompts even when Canvas auto-search is on', async () => {
    const wrapper = mountDialog({
      smartWebSearch: true,
      openRouterConfigured: true
    })

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('Explain recursion in simple terms')
    await wrapper.vm.$nextTick()
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0].webSearch).toBe(false)
    expect(wrapper.find('.web-search-hint').text()).toContain('will stay off')
  })

  it('submits web search disabled when Canvas auto-search is off', async () => {
    const wrapper = mountDialog({
      smartWebSearch: false,
      openRouterConfigured: true
    })

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('What is the latest version of Node.js?')
    await wrapper.vm.$nextTick()
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0].webSearch).toBe(false)
    expect(wrapper.find('.web-search-hint').text()).toContain('Enable Canvas Web Search in Settings')
  })

  it('shows that live web search is off when OpenRouter is unavailable', () => {
    const wrapper = mountDialog({ smartWebSearch: true, openRouterConfigured: false })

    expect(wrapper.find('.web-search-hint').text()).toContain('OpenRouter is not configured')
    expect(wrapper.find('.web-search-hint').classes()).toContain('disabled')
  })

  it('passes presets into the selector and starts with no selected models', () => {
    const wrapper = mountDialog({
      presets: [{ id: 'preset-1', name: 'Fast', model_ids: ['openai/gpt-4o'] }]
    })

    expect(wrapper.find('.preset-props').text()).toBe('1')
    expect(wrapper.find('.selection-props').text()).toBe('0')
  })

  it('forwards preset creation events from the selector', async () => {
    const wrapper = mountDialog()

    await wrapper.find('.create-preset-stub').trigger('click')

    expect(wrapper.emitted('create-preset')).toEqual([
      [{ name: 'Fast trio', modelIds: ['openai/gpt-4o'] }]
    ])
  })
})
