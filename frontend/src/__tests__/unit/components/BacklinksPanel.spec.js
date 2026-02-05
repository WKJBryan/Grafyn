/**
 * Unit tests for BacklinksPanel component
 *
 * Tests cover:
 * - Component rendering
 * - Backlink count badge display
 * - Loading state
 * - Empty state display
 * - Backlinks list rendering
 * - Navigation events on click
 * - Context display and truncation
 * - API integration (graphApi.backlinks)
 * - Error handling
 * - Props watching (noteId changes)
 * - onMounted lifecycle
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import BacklinksPanel from '@/components/BacklinksPanel.vue'
import * as apiClient from '@/api/client'

describe('BacklinksPanel', () => {
  let wrapper

  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
  })

  // ============================================================================
  // Rendering Tests
  // ============================================================================

  describe('Rendering', () => {
    it('renders the component', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(wrapper.find('.backlinks-panel').exists()).toBe(true)
    })

    it('renders panel header with title', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(wrapper.find('.panel-header h3').text()).toBe('Backlinks')
    })

    it('does not show count badge when no backlinks', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick() // Wait for loading to finish

      expect(wrapper.find('.backlink-count').exists()).toBe(false)
    })

    it('shows count badge when backlinks exist', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'Note 1', context: 'Context 1' },
        { note_id: 'note-2', title: 'Note 2', context: 'Context 2' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const badge = wrapper.find('.backlink-count')
      expect(badge.exists()).toBe(true)
      expect(badge.text()).toBe('2')
    })
  })

  // ============================================================================
  // Loading State Tests
  // ============================================================================

  describe('Loading State', () => {
    it('shows loading state initially', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockImplementation(
        () => new Promise(() => {}) // Never resolves
      )

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(wrapper.find('.loading-state').exists()).toBe(true)
      expect(wrapper.text()).toContain('Loading...')
    })

    it('hides loading state after data loads', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.loading-state').exists()).toBe(false)
    })

    it('does not show backlinks list during loading', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockImplementation(
        () => new Promise(() => {})
      )

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(wrapper.find('.backlinks-list').exists()).toBe(false)
      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Empty State Tests
  // ============================================================================

  describe('Empty State', () => {
    it('shows empty state when no backlinks', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.empty-state').exists()).toBe(true)
      expect(wrapper.text()).toContain('No backlinks yet')
    })

    it('does not show empty state when backlinks exist', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'Note 1', context: 'Context' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })

    it('does not show backlinks list when empty', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.backlinks-list').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Backlinks Display Tests
  // ============================================================================

  describe('Backlinks Display', () => {
    it('renders all backlinks', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'First Note', context: 'Context 1' },
        { note_id: 'note-2', title: 'Second Note', context: 'Context 2' },
        { note_id: 'note-3', title: 'Third Note', context: 'Context 3' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const backlinkItems = wrapper.findAll('.backlink-item')
      expect(backlinkItems).toHaveLength(3)
    })

    it('displays backlink titles', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'First Note', context: 'Context 1' },
        { note_id: 'note-2', title: 'Second Note', context: 'Context 2' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const titles = wrapper.findAll('.backlink-title')
      expect(titles[0].text()).toBe('First Note')
      expect(titles[1].text()).toBe('Second Note')
    })

    it('displays backlink context when available', async () => {
      const mockBacklinks = [
        {
          note_id: 'note-1',
          title: 'Note 1',
          context: 'This is the context around the wikilink',
        },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const context = wrapper.find('.backlink-context')
      expect(context.exists()).toBe(true)
      expect(context.text()).toBe('This is the context around the wikilink')
    })

    it('does not show context element when context is empty', async () => {
      const mockBacklinks = [{ note_id: 'note-1', title: 'Note 1', context: '' }]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.backlink-context').exists()).toBe(false)
    })

    it('does not show context element when context is null', async () => {
      const mockBacklinks = [{ note_id: 'note-1', title: 'Note 1', context: null }]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.backlink-context').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Navigation Tests
  // ============================================================================

  describe('Navigation', () => {
    it('emits navigate event when backlink is clicked', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'Note 1', context: 'Context 1' },
        { note_id: 'note-2', title: 'Note 2', context: 'Context 2' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const firstBacklink = wrapper.findAll('.backlink-item')[0]
      await firstBacklink.trigger('click')

      expect(wrapper.emitted('navigate')).toBeTruthy()
      expect(wrapper.emitted('navigate')[0]).toEqual(['note-1'])
    })

    it('emits correct note ID for each backlink click', async () => {
      const mockBacklinks = [
        { note_id: 'note-1', title: 'Note 1', context: 'Context 1' },
        { note_id: 'note-2', title: 'Note 2', context: 'Context 2' },
        { note_id: 'note-3', title: 'Note 3', context: 'Context 3' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const backlinkItems = wrapper.findAll('.backlink-item')

      await backlinkItems[0].trigger('click')
      expect(wrapper.emitted('navigate')[0]).toEqual(['note-1'])

      await backlinkItems[1].trigger('click')
      expect(wrapper.emitted('navigate')[1]).toEqual(['note-2'])

      await backlinkItems[2].trigger('click')
      expect(wrapper.emitted('navigate')[2]).toEqual(['note-3'])
    })
  })

  // ============================================================================
  // API Integration Tests
  // ============================================================================

  describe('API Integration', () => {
    it('calls graphApi.backlinks on mount', async () => {
      const mockBacklinks = vi
        .spyOn(apiClient.graph, 'backlinks')
        .mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(mockBacklinks).toHaveBeenCalledTimes(1)
      expect(mockBacklinks).toHaveBeenCalledWith('note-1')
    })

    it('does not call API when noteId is empty', async () => {
      const mockBacklinks = vi.spyOn(apiClient.graph, 'backlinks')

      wrapper = mount(BacklinksPanel, {
        props: { noteId: '' },
      })

      await nextTick()

      expect(mockBacklinks).not.toHaveBeenCalled()
    })

    it('handles API errors gracefully', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.graph, 'backlinks').mockRejectedValue(
        new Error('API error')
      )

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(consoleError).toHaveBeenCalledWith(
        'Failed to load backlinks:',
        expect.any(Error)
      )
      expect(wrapper.find('.empty-state').exists()).toBe(true)
    })

    it('clears backlinks on API error', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.graph, 'backlinks').mockRejectedValue(
        new Error('API error')
      )

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.findAll('.backlink-item')).toHaveLength(0)
    })
  })

  // ============================================================================
  // Props Watching Tests
  // ============================================================================

  describe('Props Watching', () => {
    it('reloads backlinks when noteId changes', async () => {
      const mockBacklinks = vi
        .spyOn(apiClient.graph, 'backlinks')
        .mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()

      expect(mockBacklinks).toHaveBeenCalledWith('note-1')

      await wrapper.setProps({ noteId: 'note-2' })
      await nextTick()

      expect(mockBacklinks).toHaveBeenCalledWith('note-2')
      expect(mockBacklinks).toHaveBeenCalledTimes(2)
    })

    it('displays new backlinks after noteId change', async () => {
      const mockBacklinks = vi.spyOn(apiClient.graph, 'backlinks')

      mockBacklinks.mockResolvedValueOnce([
        { note_id: 'backlink-1', title: 'First Set', context: 'Context 1' },
      ])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.findAll('.backlink-item')).toHaveLength(1)
      expect(wrapper.find('.backlink-title').text()).toBe('First Set')

      mockBacklinks.mockResolvedValueOnce([
        { note_id: 'backlink-2', title: 'Second Set', context: 'Context 2' },
        { note_id: 'backlink-3', title: 'Third Set', context: 'Context 3' },
      ])

      await wrapper.setProps({ noteId: 'note-2' })
      await nextTick()
      await nextTick()

      expect(wrapper.findAll('.backlink-item')).toHaveLength(2)
      expect(wrapper.findAll('.backlink-title')[0].text()).toBe('Second Set')
      expect(wrapper.findAll('.backlink-title')[1].text()).toBe('Third Set')
    })

    it('shows loading state during reload', async () => {
      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.loading-state').exists()).toBe(false)

      const slowPromise = new Promise((resolve) => {
        setTimeout(() => resolve([]), 100)
      })

      vi.spyOn(apiClient.graph, 'backlinks').mockReturnValue(slowPromise)

      await wrapper.setProps({ noteId: 'note-2' })
      await nextTick()

      expect(wrapper.find('.loading-state').exists()).toBe(true)

      await slowPromise
      await nextTick()

      expect(wrapper.find('.loading-state').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles very long backlink titles', async () => {
      const longTitle = 'A'.repeat(200)
      const mockBacklinks = [
        { note_id: 'note-1', title: longTitle, context: 'Context' },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const title = wrapper.find('.backlink-title')
      expect(title.text()).toBe(longTitle)
      // CSS should handle overflow
    })

    it('handles very long context text', async () => {
      const longContext =
        'This is a very long context that should be truncated by CSS. '.repeat(
          10
        )
      const mockBacklinks = [
        { note_id: 'note-1', title: 'Note', context: longContext },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      const context = wrapper.find('.backlink-context')
      expect(context.text()).toBe(longContext.trim())
      // CSS -webkit-line-clamp should handle truncation
    })

    it('handles special characters in titles', async () => {
      const mockBacklinks = [
        {
          note_id: 'note-1',
          title: '特殊字符 🎉 @#$%',
          context: 'Context with émojis 🚀',
        },
      ]

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(mockBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'target-note' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.find('.backlink-title').text()).toBe('特殊字符 🎉 @#$%')
      expect(wrapper.find('.backlink-context').text()).toBe(
        'Context with émojis 🚀'
      )
    })

    it('handles large number of backlinks', async () => {
      const manyBacklinks = Array.from({ length: 100 }, (_, i) => ({
        note_id: `note-${i}`,
        title: `Note ${i}`,
        context: `Context ${i}`,
      }))

      vi.spyOn(apiClient.graph, 'backlinks').mockResolvedValue(manyBacklinks)

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'popular-note' },
      })

      await nextTick()
      await nextTick()

      expect(wrapper.findAll('.backlink-item')).toHaveLength(100)
      expect(wrapper.find('.backlink-count').text()).toBe('100')
    })

    it('handles rapid noteId changes', async () => {
      const mockBacklinks = vi
        .spyOn(apiClient.graph, 'backlinks')
        .mockResolvedValue([])

      wrapper = mount(BacklinksPanel, {
        props: { noteId: 'note-1' },
      })

      for (let i = 2; i <= 10; i++) {
        await wrapper.setProps({ noteId: `note-${i}` })
      }

      await nextTick()
      await nextTick()

      // Should still render correctly despite rapid changes
      expect(wrapper.find('.backlinks-panel').exists()).toBe(true)
      expect(mockBacklinks).toHaveBeenCalledTimes(10)
    })
  })
})
