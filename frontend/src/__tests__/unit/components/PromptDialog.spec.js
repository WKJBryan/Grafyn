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
          props: ['modelValue'],
          emits: ['update:modelValue'],
          template: '<button class="model-selector-stub" @click="$emit(\'update:modelValue\', [\'openai/gpt-4o\'])" />'
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

  it('submits web search enabled when the Canvas default is on', async () => {
    const wrapper = mountDialog({
      smartWebSearch: true,
      openRouterConfigured: true
    })

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('Tell me about anything')
    await wrapper.vm.$nextTick()
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0].webSearch).toBe(true)
    expect(wrapper.find('.web-search-hint').text()).toContain('Live web search is on for this prompt by default')
  })

  it('submits web search disabled when the Canvas default is off', async () => {
    const wrapper = mountDialog({
      smartWebSearch: false,
      openRouterConfigured: true
    })

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('Tell me about anything')
    await wrapper.vm.$nextTick()
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0].webSearch).toBe(false)
    expect(wrapper.find('.web-search-hint').text()).toContain('Live web search is off for this prompt')
  })

  it('shows that live web search is off when OpenRouter is unavailable', () => {
    const wrapper = mountDialog({ smartWebSearch: true, openRouterConfigured: false })

    expect(wrapper.find('.web-search-hint').text()).toContain('OpenRouter is not configured')
    expect(wrapper.find('.web-search-hint').classes()).toContain('disabled')
  })
})
