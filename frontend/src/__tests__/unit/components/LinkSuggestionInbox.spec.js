import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import LinkSuggestionInbox from '@/components/LinkSuggestionInbox.vue'

const mockToast = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}))

const mockZettelkastenApi = vi.hoisted(() => ({
  listSuggestionQueue: vi.fn(),
  getDiscoveryStatus: vi.fn(),
  applyLinks: vi.fn(),
  dismissSuggestion: vi.fn(),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => mockToast,
}))

vi.mock('@/api/client', () => ({
  zettelkasten: mockZettelkastenApi,
}))

describe('LinkSuggestionInbox', () => {
  let wrapper

  const strongCandidate = {
    target_id: 'n2',
    target_title: 'Strong Match',
    link_type: 'related',
    confidence: 0.82,
    reason: 'Shared topic and overlapping tags',
  }

  const exploratoryCandidate = {
    target_id: 'n3',
    target_title: 'Exploratory Match',
    link_type: 'questions',
    confidence: 0.41,
    reason: 'Distant but interesting bridge',
  }

  beforeEach(() => {
    vi.useFakeTimers()

    mockZettelkastenApi.listSuggestionQueue.mockResolvedValue([
      {
        note_id: 'n1',
        note_title: 'Source Note',
        pending_count: 2,
        links: [strongCandidate],
        exploratory_links: [exploratoryCandidate],
      },
    ])
    mockZettelkastenApi.getDiscoveryStatus.mockResolvedValue({
      queue_size: 1,
      pending_suggestions: 2,
      is_running: true,
      current_note_id: 'n1',
      current_note_title: 'Source Note',
    })
    mockZettelkastenApi.applyLinks.mockResolvedValue({
      note_id: 'n1',
      links_created: 1,
      links_attempted: 1,
    })
    mockZettelkastenApi.dismissSuggestion.mockResolvedValue({
      note_id: 'n1',
      target_id: 'n2',
      status: 'dismissed',
    })
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
    vi.runOnlyPendingTimers()
    vi.useRealTimers()
    vi.clearAllMocks()
  })

  it('renders grouped strong and exploratory suggestions with queue status', async () => {
    wrapper = mount(LinkSuggestionInbox)
    await flushPromises()

    expect(mockZettelkastenApi.listSuggestionQueue).toHaveBeenCalledWith('pending', 20)
    expect(mockZettelkastenApi.getDiscoveryStatus).toHaveBeenCalled()
    expect(wrapper.text()).toContain('Link Inbox')
    expect(wrapper.text()).toContain('Queue 1')
    expect(wrapper.text()).toContain('Pending 2')
    expect(wrapper.text()).toContain('Strong Matches')
    expect(wrapper.text()).toContain('Exploratory')
    expect(wrapper.text()).toContain('Strong Match')
    expect(wrapper.text()).toContain('Exploratory Match')
  })

  it('applies and dismisses suggestions from the inbox', async () => {
    wrapper = mount(LinkSuggestionInbox)
    await flushPromises()

    const applyButton = wrapper.findAll('button').find((button) => button.text() === 'Apply')
    await applyButton.trigger('click')
    await flushPromises()

    expect(mockZettelkastenApi.applyLinks).toHaveBeenCalledWith('n1', [strongCandidate])
    expect(mockToast.success).toHaveBeenCalledWith('Created 1 link')

    const dismissButton = wrapper.findAll('button').find((button) => button.text() === 'Dismiss')
    await dismissButton.trigger('click')
    await flushPromises()

    expect(mockZettelkastenApi.dismissSuggestion).toHaveBeenCalledWith('n1', 'n2')
  })
})
