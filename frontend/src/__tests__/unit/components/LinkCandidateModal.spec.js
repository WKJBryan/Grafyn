import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import LinkCandidateModal from '@/components/LinkCandidateModal.vue'

const mockToast = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}))

const mockZettelkastenApi = vi.hoisted(() => ({
  applyLinks: vi.fn(),
  dismissSuggestion: vi.fn(),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => mockToast,
}))

vi.mock('@/api/client', () => ({
  zettelkasten: mockZettelkastenApi,
}))

describe('LinkCandidateModal', () => {
  let wrapper

  const candidates = [
    { target_id: 'n2', target_title: 'Note Two', link_type: 'related', confidence: 0.8, reason: 'Shared topic' },
    { target_id: 'n3', target_title: 'Note Three', link_type: 'supports', confidence: 0.7, reason: 'Supports claim' },
  ]

  beforeEach(() => {
    mockZettelkastenApi.applyLinks.mockResolvedValue({
      note_id: 'n1',
      links_created: 1,
      links_attempted: 1,
    })
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
    vi.clearAllMocks()
  })

  it('sends the selected candidate objects when applying links', async () => {
    wrapper = mount(LinkCandidateModal, {
      props: {
        noteId: 'n1',
        candidates,
      },
    })

    const checkboxes = wrapper.findAll('input[type="checkbox"]')
    await checkboxes[1].setValue(true)

    const applyButton = wrapper.findAll('button').find((btn) => btn.text().includes('Apply'))
    await applyButton.trigger('click')

    expect(mockZettelkastenApi.applyLinks).toHaveBeenCalledWith('n1', [candidates[0]])
    expect(mockToast.success).toHaveBeenCalledWith('Created 1 link')
  })

  it('renders strong and exploratory sections separately', () => {
    wrapper = mount(LinkCandidateModal, {
      props: {
        noteId: 'n1',
        candidates: [candidates[0]],
        exploratoryCandidates: [candidates[1]],
      },
    })

    expect(wrapper.text()).toContain('Strong Matches')
    expect(wrapper.text()).toContain('Exploratory')
  })

  it('renders topic hubs separately from manual link candidates', () => {
    wrapper = mount(LinkCandidateModal, {
      props: {
        noteId: 'n1',
        topicHubCandidates: [
          {
            hub_id: 'hub-ai',
            hub_title: 'Hub: AI',
            topic_key: 'ai',
            membership_source: 'auto',
          },
        ],
      },
    })

    expect(wrapper.text()).toContain('Topic Hubs')
    expect(wrapper.text()).toContain('Hub: AI')
    expect(wrapper.text()).not.toContain('Select all')
  })
})
