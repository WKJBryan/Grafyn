import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import CanvasSessionSheet from '@/components/companion/CanvasSessionSheet.vue'

describe('CanvasSessionSheet', () => {
  const sessions = [
    { id: 'b', title: 'B', updated_at: '2026-09-01T01:00:00Z' },
    { id: 'z', title: 'Z', updated_at: '2026-09-01T00:00:00Z' },
    { id: 'a', title: 'A', updated_at: '2026-09-01T01:00:00Z' },
  ]

  it('sorts recent sessions by updated time descending and stable id', () => {
    const wrapper = mount(CanvasSessionSheet, {
      props: { sessions, currentSessionId: 'a' },
    })

    expect(wrapper.findAll('[data-session-id]').map(node => node.attributes('data-session-id')))
      .toEqual(['a', 'b', 'z'])
  })

  it('emits create, select, rename, and delete operations with exact session ids', async () => {
    const wrapper = mount(CanvasSessionSheet, {
      props: { sessions, currentSessionId: 'a' },
    })

    await wrapper.get('[aria-label="Create Canvas session"]').trigger('click')
    await wrapper.get('[aria-label="Open Canvas session b"]').trigger('click')
    await wrapper.get('[aria-label="Rename Canvas session a"]').trigger('click')
    await wrapper.get('[aria-label="Session title a"]').setValue('Renamed')
    await wrapper.get('[aria-label="Save Canvas session a"]').trigger('click')
    await wrapper.get('[aria-label="Delete Canvas session z"]').trigger('click')

    expect(wrapper.emitted('create')).toHaveLength(1)
    expect(wrapper.emitted('select')).toEqual([['b']])
    expect(wrapper.emitted('rename')).toEqual([[{ id: 'a', title: 'Renamed' }]])
    expect(wrapper.emitted('delete')).toEqual([['z']])
  })
})
