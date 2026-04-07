import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import PinnedNotesPanel from '@/components/canvas/PinnedNotesPanel.vue'

const {
  searchQuery,
  getNote,
  updatePinnedNotes,
  canvasStore
} = vi.hoisted(() => ({
  searchQuery: vi.fn(),
  getNote: vi.fn(),
  updatePinnedNotes: vi.fn().mockResolvedValue(),
  canvasStore: {
    hasSession: true,
    currentSession: {
      id: 'session-1',
      pinned_note_ids: ['note-1', 'note-2']
    },
    updatePinnedNotes: vi.fn().mockResolvedValue()
  }
}))

vi.mock('@/api/client', () => ({
  search: {
    query: searchQuery
  },
  notes: {
    get: getNote
  }
}))

vi.mock('@/stores/canvas', () => ({
  useCanvasStore: () => canvasStore
}))

describe('PinnedNotesPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    canvasStore.currentSession = {
      id: 'session-1',
      pinned_note_ids: ['note-1', 'note-2']
    }
    canvasStore.updatePinnedNotes = updatePinnedNotes
    getNote.mockImplementation(async (id) => ({
      id,
      title: `Pinned ${id.toUpperCase()}`
    }))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('restores pinned note titles after a session reload', async () => {
    const wrapper = mount(PinnedNotesPanel, {
      global: {
        stubs: {
          GIcon: { template: '<span />' }
        }
      }
    })

    await flushPromises()
    await wrapper.find('button').trigger('click')
    await flushPromises()

    expect(getNote).toHaveBeenCalledWith('note-1')
    expect(getNote).toHaveBeenCalledWith('note-2')
    expect(wrapper.text()).toContain('Pinned NOTE-1')
    expect(wrapper.text()).toContain('Pinned NOTE-2')
  })

  it('maps nested search results into pin-ready note entries', async () => {
    searchQuery.mockResolvedValue([
      {
        note: {
          id: 'note-3',
          title: 'Third note'
        },
        score: 12
      }
    ])

    const wrapper = mount(PinnedNotesPanel, {
      global: {
        stubs: {
          GIcon: { template: '<span />' }
        }
      }
    })

    await wrapper.find('button').trigger('click')
    await wrapper.find('.search-input').setValue('third')
    await vi.advanceTimersByTimeAsync(250)
    await flushPromises()

    expect(searchQuery).toHaveBeenCalledWith('third', { limit: 8 })
    expect(wrapper.text()).toContain('Third note')
    expect(wrapper.find('.pin-btn').exists()).toBe(true)
  })
})
