import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import PromptNode from '@/components/canvas/PromptNode.vue'

function mountNode(tile = {}) {
  return mount(PromptNode, {
    props: {
      tile: {
        id: 'tile-1',
        prompt: 'Should I use Twin Mode?',
        created_at: '2026-05-07T00:00:00Z',
        position: { x: 0, y: 0, width: 260, height: 140 },
        responses: {},
        context_notes: [],
        approved_twin_records: [],
        candidate_twin_records: [],
        ...tile
      }
    },
    global: {
      stubs: {
        GIcon: { template: '<span />' }
      }
    }
  })
}

describe('PromptNode', () => {
  it('shows notes, approved twin records, and candidate twin records separately', () => {
    const wrapper = mountNode({
      context_notes: [
        { id: 'note-1', title: 'Decision Notes', snippet: 'Use evidence.', score: 8, pinned: false }
      ],
      approved_twin_records: [
        {
          id: 'record-1',
          content: 'Prefers evidence-backed implementation detail.',
          confidence: 0.9,
          evidence_count: 4
        }
      ],
      candidate_twin_records: [
        {
          id: 'record-2',
          content: 'May prefer red-team critique before shipping.',
          confidence: 0.6,
          evidence_count: 1
        }
      ]
    })

    expect(wrapper.text()).toContain('Notes')
    expect(wrapper.text()).toContain('Decision Notes')
    expect(wrapper.text()).toContain('Approved')
    expect(wrapper.text()).toContain('Prefers evidence-backed')
    expect(wrapper.text()).toContain('Candidate')
    expect(wrapper.text()).toContain('May prefer red-team')
  })

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
