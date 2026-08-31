import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import QuickCaptureCard from '@/components/companion/QuickCaptureCard.vue'
import { twin } from '@/api/client'

describe('QuickCaptureCard', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('sends the exact companion capture request and never derives a title', async () => {
    const create = vi.spyOn(twin, 'createCompanionCapture').mockResolvedValue({
      note: { id: 'note-1', title: 'Backend title' },
      observationEventId: 'event-1',
    })
    const wrapper = mount(QuickCaptureCard)

    await wrapper.get('textarea').setValue('  Met Priya after lunch.  ')
    await wrapper.get('input[name="person"]').setValue('Priya')
    await wrapper.get('input[name="role"]').setValue('designer')
    await wrapper.get('input[name="relationship"]').setValue('collaborator')
    await wrapper.get('input[name="environment"]').setValue('studio')
    await wrapper.get('input[name="activity"]').setValue('planning')
    await wrapper.get('input[name="goal"]').setValue('ship prototype')
    await wrapper.get('input[name="local-only"]').setValue(true)
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(create).toHaveBeenCalledOnce()
    expect(create).toHaveBeenCalledWith({
      content: '  Met Priya after lunch.  ',
      captureKind: 'text',
      context: {
        person: 'Priya',
        role: 'designer',
        relationship: 'collaborator',
        environment: 'studio',
        activity: 'planning',
        goal: 'ship prototype',
      },
      attachmentDigests: [],
      grafynSync: 'local_only',
    })
    expect(create.mock.calls[0][0]).not.toHaveProperty('title')
    expect(wrapper.get('textarea').element.value).toBe('')
    expect(wrapper.get('[role="status"]').text()).toContain('Backend title')
    expect(wrapper.emitted('captured')?.[0]).toEqual([{
      note: { id: 'note-1', title: 'Backend title' },
      observationEventId: 'event-1',
    }])
  })

  it('keeps the draft and shows safe local copy when capture fails', async () => {
    const create = vi.spyOn(twin, 'createCompanionCapture')
      .mockRejectedValue(new Error('C:\\private\\vault'))
    const wrapper = mount(QuickCaptureCard, {
      props: { attachmentDigests: ['sha256:attachment-1'] },
    })

    await wrapper.get('textarea').setValue('Keep this thought')
    await wrapper.get('input[name="person"]').setValue('Priya')
    await wrapper.get('input[name="local-only"]').setValue(true)
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(wrapper.get('textarea').element.value).toBe('Keep this thought')
    expect(wrapper.get('input[name="person"]').element.value).toBe('Priya')
    expect(wrapper.get('input[name="local-only"]').element.checked).toBe(true)
    expect(create).toHaveBeenCalledWith(expect.objectContaining({
      attachmentDigests: ['sha256:attachment-1'],
      grafynSync: 'local_only',
      context: expect.objectContaining({ person: 'Priya' }),
    }))
    expect(wrapper.get('[role="alert"]').text()).toBe('Capture could not be saved. Your draft is still here.')
    expect(wrapper.text()).not.toContain('private')
  })

  it.each([
    new Error('companion capture committed and recovery is pending; do not retry'),
    'companion capture was not applied; refresh state before deciding whether to retry',
  ])('retains and disables an uncertain capture without exposing backend detail', async (rejection) => {
    const create = vi.spyOn(twin, 'createCompanionCapture').mockRejectedValue(rejection)
    const wrapper = mount(QuickCaptureCard, {
      props: { attachmentDigests: ['sha256:attachment-1'] },
    })

    await wrapper.get('textarea').setValue('Keep this exact uncertain draft')
    await wrapper.get('input[name="person"]').setValue('Priya')
    await wrapper.get('input[name="local-only"]').setValue(true)
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(wrapper.get('textarea').element.value).toBe('Keep this exact uncertain draft')
    expect(wrapper.get('input[name="person"]').element.value).toBe('Priya')
    expect(wrapper.get('input[name="local-only"]').element.checked).toBe(true)
    expect(wrapper.get('button[type="submit"]').attributes()).toHaveProperty('disabled')
    expect(wrapper.get('[role="alert"]').text()).toBe(
      'This capture may already be saved. Do not submit it again. Check Recent captures before continuing.',
    )
    expect(wrapper.text()).not.toContain('recovery is pending')
    expect(wrapper.text()).not.toContain('refresh state')
    expect(wrapper.emitted('capture-uncertain')).toEqual([[]])
    expect(wrapper.emitted('captured')).toBeUndefined()

    await wrapper.get('form').trigger('submit')
    expect(create).toHaveBeenCalledOnce()
  })

  it('keeps an ordinary failure retryable', async () => {
    const create = vi.spyOn(twin, 'createCompanionCapture')
      .mockRejectedValueOnce(new Error('temporary write failure'))
      .mockResolvedValueOnce({
        note: { id: 'note-1', title: 'Saved on retry' },
        observationEventId: 'event-1',
      })
    const wrapper = mount(QuickCaptureCard)

    await wrapper.get('textarea').setValue('Retry this draft')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(wrapper.get('button[type="submit"]').attributes()).not.toHaveProperty('disabled')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(create).toHaveBeenCalledTimes(2)
    expect(wrapper.get('[role="status"]').text()).toContain('Saved on retry')
  })

  it('does not submit whitespace-only content', async () => {
    const create = vi.spyOn(twin, 'createCompanionCapture')
    const wrapper = mount(QuickCaptureCard)

    await wrapper.get('textarea').setValue('   ')

    expect(wrapper.get('button[type="submit"]').attributes()).toHaveProperty('disabled')
    await wrapper.get('form').trigger('submit')
    expect(create).not.toHaveBeenCalled()
  })
})
