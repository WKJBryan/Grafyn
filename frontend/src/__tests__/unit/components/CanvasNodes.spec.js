import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import PromptNode from '@/components/canvas/PromptNode.vue'
import LLMNode from '@/components/canvas/LLMNode.vue'
import DebateNode from '@/components/canvas/DebateNode.vue'

const mountedWrappers = []

function mountAttached(component, options) {
  const wrapper = mount(component, {
    attachTo: document.body,
    ...options
  })
  mountedWrappers.push(wrapper)
  return wrapper
}

function mouse(target, type, coords = {}) {
  target.dispatchEvent(new MouseEvent(type, {
    bubbles: true,
    ...coords
  }))
}

afterEach(() => {
  while (mountedWrappers.length > 0) {
    mountedWrappers.pop().unmount()
  }
})

describe('Canvas Nodes', () => {
  it('renders full completed LLM response content without truncation', () => {
    const content = 'a'.repeat(3501)
    const wrapper = mountAttached(LLMNode, {
      props: {
        tileId: 'tile-1',
        modelId: 'openai/gpt-4',
        response: {
          status: 'completed',
          content,
          model_name: 'GPT-4',
          color: '#7c5cff',
          position: { x: 10, y: 20, width: 280, height: 200 }
        },
        isStreaming: false,
        selected: false,
        availableModels: []
      }
    })

    expect(wrapper.find('.response-text').text()).toBe(content)
    expect(wrapper.html()).not.toContain('Content truncated')
  })

  it('renders resize handles for prompt, response, and debate nodes', () => {
    const promptWrapper = mountAttached(PromptNode, {
      props: {
        tile: {
          id: 'prompt-1',
          prompt: 'Prompt',
          responses: { 'openai/gpt-4': { status: 'completed' } },
          created_at: new Date().toISOString(),
          position: { x: 0, y: 0, width: 200, height: 120 }
        }
      }
    })
    const llmWrapper = mountAttached(LLMNode, {
      props: {
        tileId: 'tile-1',
        modelId: 'openai/gpt-4',
        response: {
          status: 'completed',
          content: 'Response',
          model_name: 'GPT-4',
          color: '#7c5cff',
          position: { x: 0, y: 0, width: 280, height: 200 }
        },
        availableModels: []
      }
    })
    const debateWrapper = mountAttached(DebateNode, {
      props: {
        debate: {
          id: 'debate-1',
          participating_models: ['openai/gpt-4'],
          rounds: [],
          status: 'active',
          position: { x: 0, y: 0, width: 280, height: 200 },
          debate_mode: 'auto'
        }
      }
    })

    expect(promptWrapper.find('.resize-handle').exists()).toBe(true)
    expect(llmWrapper.find('.resize-handle').exists()).toBe(true)
    expect(debateWrapper.find('.resize-handle').exists()).toBe(true)
  })

  it('resizes prompt nodes and keeps x/y fixed when dragging from the resize handle', () => {
    const wrapper = mountAttached(PromptNode, {
      props: {
        tile: {
          id: 'prompt-1',
          prompt: 'Prompt',
          responses: { 'openai/gpt-4': { status: 'completed' } },
          created_at: new Date().toISOString(),
          position: { x: 15, y: 25, width: 200, height: 120 }
        }
      }
    })

    mouse(wrapper.find('.resize-handle').element, 'mousedown', { clientX: 0, clientY: 0, button: 0 })
    mouse(document, 'mousemove', { clientX: 60, clientY: 40, button: 0 })
    mouse(document, 'mouseup', { clientX: 60, clientY: 40, button: 0 })

    const [[tileId, position]] = wrapper.emitted('drag')
    expect(tileId).toBe('prompt-1')
    expect(position).toEqual({ x: 15, y: 25, width: 260, height: 160 })
  })

  it('enforces minimum size when resizing prompt nodes', () => {
    const wrapper = mountAttached(PromptNode, {
      props: {
        tile: {
          id: 'prompt-1',
          prompt: 'Prompt',
          responses: { 'openai/gpt-4': { status: 'completed' } },
          created_at: new Date().toISOString(),
          position: { x: 15, y: 25, width: 200, height: 120 }
        }
      }
    })

    mouse(wrapper.find('.resize-handle').element, 'mousedown', { clientX: 100, clientY: 100, button: 0 })
    mouse(document, 'mousemove', { clientX: -200, clientY: -200, button: 0 })
    mouse(document, 'mouseup', { clientX: -200, clientY: -200, button: 0 })

    const [[, position]] = wrapper.emitted('drag')
    expect(position.width).toBe(200)
    expect(position.height).toBe(120)
  })

  it('resizes LLM response nodes without starting a move drag', () => {
    const wrapper = mountAttached(LLMNode, {
      props: {
        tileId: 'tile-1',
        modelId: 'openai/gpt-4',
        response: {
          status: 'completed',
          content: 'Response',
          model_name: 'GPT-4',
          color: '#7c5cff',
          position: { x: 5, y: 6, width: 280, height: 200 }
        },
        availableModels: []
      }
    })

    mouse(wrapper.find('.resize-handle').element, 'mousedown', { clientX: 10, clientY: 10, button: 0 })
    mouse(document, 'mousemove', { clientX: 50, clientY: 70, button: 0 })
    mouse(document, 'mouseup', { clientX: 50, clientY: 70, button: 0 })

    const [[tileId, modelId, position]] = wrapper.emitted('drag')
    expect(tileId).toBe('tile-1')
    expect(modelId).toBe('openai/gpt-4')
    expect(position).toEqual({ x: 5, y: 6, width: 320, height: 260 })
  })

  it('enforces minimum size when resizing debate nodes', () => {
    const wrapper = mountAttached(DebateNode, {
      props: {
        debate: {
          id: 'debate-1',
          participating_models: ['openai/gpt-4'],
          rounds: [],
          status: 'active',
          position: { x: 30, y: 40, width: 280, height: 200 },
          debate_mode: 'auto'
        }
      }
    })

    mouse(wrapper.find('.resize-handle').element, 'mousedown', { clientX: 100, clientY: 100, button: 0 })
    mouse(document, 'mousemove', { clientX: -400, clientY: -400, button: 0 })
    mouse(document, 'mouseup', { clientX: -400, clientY: -400, button: 0 })

    const [[debateId, position]] = wrapper.emitted('drag')
    expect(debateId).toBe('debate-1')
    expect(position).toEqual({ x: 30, y: 40, width: 280, height: 200 })
  })

  it('shows every selected branch model chip instead of collapsing after three', async () => {
    const availableModels = [
      { id: 'openai/gpt-4', name: 'OpenAI: GPT-4' },
      { id: 'anthropic/claude-3.5-sonnet', name: 'Anthropic: Claude 3.5 Sonnet' },
      { id: 'google/gemini-1.5-pro', name: 'Google: Gemini 1.5 Pro' },
      { id: 'meta-llama/llama-3.1-70b-instruct', name: 'Meta: Llama 3.1 70B Instruct' },
      { id: 'mistral/large', name: 'Mistral: Large' }
    ]
    const wrapper = mountAttached(LLMNode, {
      props: {
        tileId: 'tile-1',
        modelId: 'openai/gpt-4',
        response: {
          status: 'completed',
          content: 'Response',
          model_name: 'GPT-4',
          color: '#7c5cff',
          position: { x: 0, y: 0, width: 280, height: 200 }
        },
        availableModels
      }
    })

    await wrapper.find('.branch-btn').trigger('click')
    await nextTick()
    await wrapper.find('.models-toggle').trigger('click')
    await nextTick()

    const checkboxes = wrapper.findAll('.model-picker-item input[type="checkbox"]')
    for (const checkbox of checkboxes.slice(1)) {
      await checkbox.setValue(true)
    }
    await nextTick()

    expect(wrapper.findAll('.branch-model-tag')).toHaveLength(5)
    expect(wrapper.find('.branch-model-tags').text()).toContain('GPT-4')
    expect(wrapper.find('.branch-model-tags').text()).toContain('Claude 3.5 Sonnet')
    expect(wrapper.find('.branch-model-tags').text()).toContain('Gemini 1.5 Pro')
    expect(wrapper.find('.more-models').exists()).toBe(false)
  })
})
