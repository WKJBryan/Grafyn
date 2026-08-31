import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CaptureView from '@/views/companion/CaptureView.vue'
import { notes } from '@/api/client'

function deferred() {
  let resolve
  let reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

vi.mock('@/components/companion/QuickCaptureCard.vue', () => ({
  default: {
    name: 'QuickCaptureCard',
    template: `
      <button data-test="capture" @click="$emit('captured', { note: { id: 'new' } })">Capture</button>
      <button data-test="uncertain" @click="$emit('capture-uncertain')">Uncertain</button>
    `,
  },
}))

describe('CaptureView', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('shows only the five most recent inbox captures in newest-first order', async () => {
    vi.spyOn(notes, 'list').mockResolvedValue([
      { id: 'old', title: 'Old inbox', tags: ['inbox'], created_at: '2026-08-30T10:00:00Z' },
      { id: 'other', title: 'Not inbox', tags: ['work'], created_at: '2026-09-01T12:00:00Z' },
      { id: 'new', title: 'Newest inbox', tags: ['inbox'], created_at: '2026-09-01T10:00:00Z' },
      { id: 'b', title: 'Second', tags: ['inbox'], created_at: '2026-08-31T10:00:00Z' },
      { id: 'a', title: 'Tie by id', tags: ['inbox'], created_at: '2026-08-31T10:00:00Z' },
      { id: 'three', title: 'Third', tags: ['inbox'], created_at: '2026-08-31T09:00:00Z' },
      { id: 'four', title: 'Fourth', tags: ['inbox'], created_at: '2026-08-31T08:00:00Z' },
      { id: 'five', title: 'Fifth', tags: ['inbox'], created_at: '2026-08-31T07:00:00Z' },
    ])

    const wrapper = mount(CaptureView)
    await flushPromises()

    expect(wrapper.findAll('.recent-capture-title').map(item => item.text())).toEqual([
      'Newest inbox',
      'Tie by id',
      'Second',
      'Third',
      'Fourth',
    ])
    expect(wrapper.text()).not.toContain('Not inbox')
    expect(wrapper.text()).not.toContain('Old inbox')
    expect(wrapper.text()).not.toContain('Fifth')
  })

  it('refreshes recent captures after a successful capture', async () => {
    const list = vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    await wrapper.get('[data-test="capture"]').trigger('click')
    await flushPromises()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('refreshes recent captures after an uncertain capture outcome', async () => {
    const list = vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    await wrapper.get('[data-test="uncertain"]').trigger('click')
    await flushPromises()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('does not let a slow older load overwrite a newer refresh', async () => {
    const first = deferred()
    const second = deferred()
    const list = vi.spyOn(notes, 'list')
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const wrapper = mount(CaptureView)

    expect(list).toHaveBeenCalledOnce()
    await wrapper.get('[data-test="capture"]').trigger('click')
    expect(list).toHaveBeenCalledTimes(2)

    second.resolve([
      { id: 'new', title: 'Newest state', tags: ['inbox'], created_at: '2026-09-01T10:00:00Z' },
    ])
    await flushPromises()
    expect(wrapper.get('.recent-capture-title').text()).toBe('Newest state')

    first.resolve([
      { id: 'old', title: 'Stale state', tags: ['inbox'], created_at: '2026-08-30T10:00:00Z' },
    ])
    await flushPromises()

    expect(wrapper.get('.recent-capture-title').text()).toBe('Newest state')
    expect(wrapper.text()).not.toContain('Stale state')
  })

  it('uses safe local copy if recent captures cannot load', async () => {
    vi.spyOn(notes, 'list').mockRejectedValue(new Error('C:\\private\\vault'))
    const wrapper = mount(CaptureView)
    await flushPromises()

    expect(wrapper.get('[role="alert"]').text()).toBe('Recent captures are temporarily unavailable.')
    expect(wrapper.text()).not.toContain('private')
  })
})
