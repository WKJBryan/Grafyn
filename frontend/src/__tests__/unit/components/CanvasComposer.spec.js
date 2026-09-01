import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import CanvasComposer from '@/components/companion/CanvasComposer.vue'

describe('CanvasComposer', () => {
  const models = [
    { id: 'model-a', name: 'Model A', provider: 'anthropic' },
    { id: 'model-b', name: 'Model B', provider: 'ollama' },
  ]

  it.each(['plain', 'knowledge', 'twin'])('submits one selected model in %s mode', async mode => {
    const wrapper = mount(CanvasComposer, {
      props: { models, parent: { tileId: 'parent-tile', modelId: 'parent-model' } },
    })

    await wrapper.get('[aria-label="Canvas prompt"]').setValue(`Question for ${mode}`)
    await wrapper.get('[aria-label="Canvas model"]').setValue('model-b')
    await wrapper.get('[aria-label="Canvas context"]').setValue(mode)
    await wrapper.get('form').trigger('submit')

    expect(wrapper.emitted('submit')).toEqual([[{
      prompt: `Question for ${mode}`,
      modelId: 'model-b',
      provider: 'ollama',
      mode,
      parentTileId: 'parent-tile',
      parentModelId: 'parent-model',
    }]])
  })

  it('surfaces the selected parent and can clear it explicitly', async () => {
    const wrapper = mount(CanvasComposer, {
      props: { models, parent: { tileId: 'parent-tile', modelId: 'parent-model' } },
    })

    expect(wrapper.text()).toContain('Following parent-model')
    await wrapper.get('[aria-label="Clear Canvas follow-up"]').trigger('click')
    expect(wrapper.emitted('clear-parent')).toHaveLength(1)
  })

  it('routes catalog vendors through OpenRouter instead of treating the vendor as a runtime', async () => {
    const wrapper = mount(CanvasComposer, { props: { models } })

    await wrapper.get('[aria-label="Canvas prompt"]').setValue('Ask Anthropic through OpenRouter')
    await wrapper.get('[aria-label="Canvas model"]').setValue('model-a')
    await wrapper.get('form').trigger('submit')

    expect(wrapper.emitted('submit')[0][0]).toMatchObject({
      modelId: 'model-a',
      provider: 'openrouter',
    })
  })

  it('keeps the submitted draft until its owner explicitly changes it', async () => {
    const wrapper = mount(CanvasComposer, { props: { models, draft: 'Original draft' } })
    const prompt = wrapper.get('[aria-label="Canvas prompt"]')

    expect(prompt.element.value).toBe('Original draft')
    await wrapper.get('form').trigger('submit')
    expect(prompt.element.value).toBe('Original draft')

    await prompt.setValue('New draft typed while sending')
    expect(wrapper.emitted('update:draft').at(-1)).toEqual(['New draft typed while sending'])

    await wrapper.setProps({ draft: 'Session-owned replacement' })
    await wrapper.vm.$nextTick()
    expect(prompt.element.value).toBe('Session-owned replacement')
  })

  it('does not apply an old owner draft after the prop switches to a new session', async () => {
    const wrapper = mount(CanvasComposer, { props: { models, draft: 'Session A draft' } })
    const prompt = wrapper.get('[aria-label="Canvas prompt"]')

    await wrapper.setProps({ draft: '' })
    await wrapper.vm.$nextTick()
    expect(prompt.element.value).toBe('')
  })
})
