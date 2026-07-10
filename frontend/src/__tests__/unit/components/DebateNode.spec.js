import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import DebateNode from '@/components/canvas/DebateNode.vue'

function mountNode(debate = {}) {
  return mount(DebateNode, {
    props: {
      debate: {
        id: 'debate-1',
        debate_mode: 'standard',
        status: 'active',
        participating_models: ['openai/gpt-4o'],
        rounds: [],
        position: { x: 0, y: 0, width: 320, height: 240 },
        ...debate
      }
    },
    global: {
      stubs: {
        GIcon: { template: '<span />' }
      }
    }
  })
}

describe('DebateNode', () => {
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
