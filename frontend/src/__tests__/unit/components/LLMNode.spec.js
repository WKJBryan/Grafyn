import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import LLMNode from '@/components/canvas/LLMNode.vue'

function mountNode(response = {}) {
  return mount(LLMNode, {
    props: {
      tileId: 'tile-1',
      modelId: 'openai/gpt-4o',
      response: {
        status: 'completed',
        content: 'Hello world',
        position: { x: 0, y: 0, width: 280, height: 200 },
        color: '#7c5cff',
        ...response
      }
    },
    global: {
      stubs: {
        GIcon: { template: '<span />' }
      }
    }
  })
}

describe('LLMNode', () => {
  it('removes resize listeners on unmount if a resize is in progress', async () => {
    const addSpy = vi.spyOn(document, 'addEventListener')
    const removeSpy = vi.spyOn(document, 'removeEventListener')
    const wrapper = mountNode()

    await wrapper.find('.resize-handle').trigger('mousedown')

    const moveCall = addSpy.mock.calls.find(([evt, fn]) => evt === 'mousemove' && fn.name === 'onResize')
    const upCall = addSpy.mock.calls.find(([evt, fn]) => evt === 'mouseup' && fn.name === 'stopResize')
    expect(moveCall).toBeTruthy()
    expect(upCall).toBeTruthy()

    wrapper.unmount()

    expect(removeSpy).toHaveBeenCalledWith('mousemove', moveCall[1])
    expect(removeSpy).toHaveBeenCalledWith('mouseup', upCall[1])

    addSpy.mockRestore()
    removeSpy.mockRestore()
  })
})
