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

  it('shows Twin context mode and emits advisor mode by default', async () => {
    const wrapper = mountDialog()

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('What should my twin consider?')
    await wrapper.find('#contextMode').setValue('twin')
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.text()).toContain('Twin (notes + user records)')
    expect(wrapper.text()).toContain('Twin Answer Mode')
    expect(wrapper.emitted('submit')[0][0]).toMatchObject({
      contextMode: 'twin',
      twinAnswerMode: 'advisor'
    })
  })

  it('emits simulation mode when selected', async () => {
    const wrapper = mountDialog()

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.find('#prompt').setValue('Simulate my likely response')
    await wrapper.find('#contextMode').setValue('twin')
    await wrapper.findAll('.segmented-control button').find(button => button.text() === 'Simulation').trigger('click')
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0]).toMatchObject({
      contextMode: 'twin',
      twinAnswerMode: 'simulation'
    })
  })

  it('sets Decision mode to Twin Advisor and emits decision metadata', async () => {
    const wrapper = mountDialog()

    await wrapper.find('.model-selector-stub').trigger('click')
    await wrapper.findAll('.segmented-control button').find(button => button.text() === 'Decision').trigger('click')
    await wrapper.find('#prompt').setValue('Should Grafyn build Decision Mirror first?')
    await wrapper.find('#decisionOptions').setValue('Decision Mirror\nTopology')
    await wrapper.find('#decisionStakes').setValue('Product direction')
    await wrapper.find('#decisionLeaning').setValue('Decision Mirror')
    await wrapper.find('#decisionReviewDate').setValue('2026-05-15')
    await wrapper.find('.btn-primary').trigger('click')

    expect(wrapper.emitted('submit')[0][0]).toMatchObject({
      promptType: 'decision',
      contextMode: 'twin',
      twinAnswerMode: 'advisor',
      decisionMetadata: {
        decision: 'Should Grafyn build Decision Mirror first?',
        options: ['Decision Mirror', 'Topology'],
        stakes: 'Product direction',
        initial_leaning: 'Decision Mirror',
        review_date: '2026-05-15'
      }
    })
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
