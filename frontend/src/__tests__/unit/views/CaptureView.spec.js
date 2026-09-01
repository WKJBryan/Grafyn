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
    props: {
      attachmentDigests: {
        type: Array,
        default: () => [],
      },
    },
    emits: ['captured', 'capture-uncertain'],
    template: `
      <button data-test="capture" @click="$emit('captured', { note: { id: 'new' } })">Capture</button>
      <button data-test="uncertain" @click="$emit('capture-uncertain')">Uncertain</button>
    `,
  },
}))

vi.mock('@/components/companion/QuickImageComposer.vue', () => ({
  default: {
    name: 'QuickImageComposer',
    props: {
      collapsible: Boolean,
    },
    emits: ['saved'],
    template: '<div data-test="image-composer" />',
  },
}))

const DIGEST_A = 'a'.repeat(64)
const DIGEST_B = 'b'.repeat(64)

function quickCapture(wrapper) {
  return wrapper.getComponent({ name: 'QuickCaptureCard' })
}

function imageComposer(wrapper) {
  return wrapper.getComponent({ name: 'QuickImageComposer' })
}

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

  it('passes only a validated governed image digest into the next capture', async () => {
    vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    imageComposer(wrapper).vm.$emit('saved', {
      attachmentDigest: DIGEST_A,
      localPath: 'C:\\private\\vault\\image.png',
      providerResponse: { requestId: 'provider-secret' },
    })
    await wrapper.vm.$nextTick()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_A])

    imageComposer(wrapper).vm.$emit('saved', {
      attachmentDigest: '../private/image.png',
      localPath: 'C:\\private\\vault\\image.png',
    })
    await wrapper.vm.$nextTick()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_A])
    expect(wrapper.text()).not.toContain('provider-secret')
    expect(wrapper.text()).not.toContain('private')
  })

  it('replaces the pending attachment when a later governed image save succeeds', async () => {
    vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    imageComposer(wrapper).vm.$emit('saved', { attachmentDigest: DIGEST_A })
    await wrapper.vm.$nextTick()
    imageComposer(wrapper).vm.$emit('saved', { attachmentDigest: DIGEST_B })
    await wrapper.vm.$nextTick()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_B])
  })

  it('clears only the digest confirmed in the successful capture evidence', async () => {
    vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    imageComposer(wrapper).vm.$emit('saved', { attachmentDigest: DIGEST_A })
    await wrapper.vm.$nextTick()
    imageComposer(wrapper).vm.$emit('saved', { attachmentDigest: DIGEST_B })
    await wrapper.vm.$nextTick()

    quickCapture(wrapper).vm.$emit('captured', {
      note: {
        id: 'capture-a',
        properties: { attachment_digests: [DIGEST_A] },
      },
    })
    await flushPromises()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_B])

    quickCapture(wrapper).vm.$emit('captured', {
      note: {
        id: 'capture-b',
        properties: { attachment_digests: [DIGEST_B] },
      },
    })
    await flushPromises()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([])
  })

  it('retains the pending digest when capture success cannot confirm it or is uncertain', async () => {
    vi.spyOn(notes, 'list').mockResolvedValue([])
    const wrapper = mount(CaptureView)
    await flushPromises()

    imageComposer(wrapper).vm.$emit('saved', { attachmentDigest: DIGEST_A })
    await wrapper.vm.$nextTick()
    quickCapture(wrapper).vm.$emit('captured', { note: { id: 'capture-without-evidence' } })
    await flushPromises()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_A])

    quickCapture(wrapper).vm.$emit('capture-uncertain')
    await flushPromises()

    expect(quickCapture(wrapper).props('attachmentDigests')).toEqual([DIGEST_A])
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
