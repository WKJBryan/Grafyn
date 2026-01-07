/**
 * Unit tests for NoteList component
 *
 * Tests cover:
 * - Component rendering with notes
 * - Empty state display
 * - Note count display
 * - Selection highlighting
 * - Note selection events
 * - Tag display and truncation (first 3 + count)
 * - Status badge display
 * - Link count display
 * - Untitled note handling
 * - Click interactions
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import NoteList from '@/components/NoteList.vue'

describe('NoteList', () => {
  let wrapper
  let mockNotes

  beforeEach(() => {
    mockNotes = [
      {
        id: 'note-1',
        title: 'First Note',
        status: 'draft',
        tags: ['tag1', 'tag2'],
        link_count: 3,
      },
      {
        id: 'note-2',
        title: 'Second Note',
        status: 'canonical',
        tags: ['tag3'],
        link_count: 1,
      },
      {
        id: 'note-3',
        title: 'Third Note',
        status: 'evidence',
        tags: [],
        link_count: 0,
      },
    ]
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
    it('renders the component', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.find('.note-list').exists()).toBe(true)
    })

    it('renders header with title', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.find('.note-list-header h3').text()).toBe('Notes')
    })

    it('renders all notes', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const noteItems = wrapper.findAll('.note-item')
      expect(noteItems).toHaveLength(3)
    })

    it('renders note titles', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const titles = wrapper.findAll('.note-item-title')
      expect(titles[0].text()).toBe('First Note')
      expect(titles[1].text()).toBe('Second Note')
      expect(titles[2].text()).toBe('Third Note')
    })

    it('renders Untitled for notes without title', () => {
      const notesWithoutTitle = [{ id: 'note-1', title: '', status: 'draft', tags: [] }]

      wrapper = mount(NoteList, {
        props: { notes: notesWithoutTitle },
      })

      expect(wrapper.find('.note-item-title').text()).toBe('Untitled')
    })

    it('renders Untitled for notes with null title', () => {
      const notesWithNullTitle = [{ id: 'note-1', title: null, status: 'draft', tags: [] }]

      wrapper = mount(NoteList, {
        props: { notes: notesWithNullTitle },
      })

      expect(wrapper.find('.note-item-title').text()).toBe('Untitled')
    })
  })

  // ============================================================================
  // Note Count Tests
  // ============================================================================

  describe('Note Count', () => {
    it('displays correct note count', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.find('.note-count').text()).toBe('3')
    })

    it('displays 0 for empty list', () => {
      wrapper = mount(NoteList, {
        props: { notes: [] },
      })

      expect(wrapper.find('.note-count').text()).toBe('0')
    })

    it('updates count when notes change', async () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.find('.note-count').text()).toBe('3')

      await wrapper.setProps({ notes: mockNotes.slice(0, 1) })

      expect(wrapper.find('.note-count').text()).toBe('1')
    })
  })

  // ============================================================================
  // Empty State Tests
  // ============================================================================

  describe('Empty State', () => {
    it('shows empty message when no notes', () => {
      wrapper = mount(NoteList, {
        props: { notes: [] },
      })

      expect(wrapper.find('.empty-list').exists()).toBe(true)
      expect(wrapper.text()).toContain('No notes yet')
    })

    it('does not show empty message when notes exist', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.find('.empty-list').exists()).toBe(false)
    })

    it('does not show note items when empty', () => {
      wrapper = mount(NoteList, {
        props: { notes: [] },
      })

      expect(wrapper.findAll('.note-item')).toHaveLength(0)
    })
  })

  // ============================================================================
  // Selection Tests
  // ============================================================================

  describe('Selection', () => {
    it('highlights selected note', () => {
      wrapper = mount(NoteList, {
        props: {
          notes: mockNotes,
          selected: 'note-2',
        },
      })

      const noteItems = wrapper.findAll('.note-item')
      expect(noteItems[0].classes()).not.toContain('selected')
      expect(noteItems[1].classes()).toContain('selected')
      expect(noteItems[2].classes()).not.toContain('selected')
    })

    it('works without selection', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const noteItems = wrapper.findAll('.note-item')
      noteItems.forEach((item) => {
        expect(item.classes()).not.toContain('selected')
      })
    })

    it('updates selection when prop changes', async () => {
      wrapper = mount(NoteList, {
        props: {
          notes: mockNotes,
          selected: 'note-1',
        },
      })

      let noteItems = wrapper.findAll('.note-item')
      expect(noteItems[0].classes()).toContain('selected')

      await wrapper.setProps({ selected: 'note-3' })

      noteItems = wrapper.findAll('.note-item')
      expect(noteItems[0].classes()).not.toContain('selected')
      expect(noteItems[2].classes()).toContain('selected')
    })

    it('emits select event when note is clicked', async () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const firstNote = wrapper.findAll('.note-item')[0]
      await firstNote.trigger('click')

      expect(wrapper.emitted('select')).toBeTruthy()
      expect(wrapper.emitted('select')[0]).toEqual(['note-1'])
    })

    it('emits correct note ID for each click', async () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const noteItems = wrapper.findAll('.note-item')

      await noteItems[0].trigger('click')
      expect(wrapper.emitted('select')[0]).toEqual(['note-1'])

      await noteItems[1].trigger('click')
      expect(wrapper.emitted('select')[1]).toEqual(['note-2'])

      await noteItems[2].trigger('click')
      expect(wrapper.emitted('select')[2]).toEqual(['note-3'])
    })
  })

  // ============================================================================
  // Status Display Tests
  // ============================================================================

  describe('Status Display', () => {
    it('displays note status', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const statuses = wrapper.findAll('.status')
      expect(statuses[0].text()).toBe('draft')
      expect(statuses[1].text()).toBe('canonical')
      expect(statuses[2].text()).toBe('evidence')
    })

    it('applies status-specific CSS classes', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const statuses = wrapper.findAll('.status')
      expect(statuses[0].classes()).toContain('status-draft')
      expect(statuses[1].classes()).toContain('status-canonical')
      expect(statuses[2].classes()).toContain('status-evidence')
    })
  })

  // ============================================================================
  // Link Count Tests
  // ============================================================================

  describe('Link Count Display', () => {
    it('displays link count when present', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const linkCounts = wrapper.findAll('.link-count')
      expect(linkCounts[0].text()).toBe('3 links')
      expect(linkCounts[1].text()).toBe('1 links')
      expect(linkCounts[2].text()).toBe('0 links')
    })

    it('does not display link count when undefined', () => {
      const notesWithoutLinkCount = [
        { id: 'note-1', title: 'Test', status: 'draft', tags: [] },
      ]

      wrapper = mount(NoteList, {
        props: { notes: notesWithoutLinkCount },
      })

      expect(wrapper.find('.link-count').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Tag Display Tests
  // ============================================================================

  describe('Tag Display', () => {
    it('displays tags for notes', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const noteItems = wrapper.findAll('.note-item')

      // First note has 2 tags
      const firstNoteTags = noteItems[0].findAll('.tag')
      expect(firstNoteTags).toHaveLength(2)
      expect(firstNoteTags[0].text()).toBe('tag1')
      expect(firstNoteTags[1].text()).toBe('tag2')

      // Second note has 1 tag
      const secondNoteTags = noteItems[1].findAll('.tag')
      expect(secondNoteTags).toHaveLength(1)
      expect(secondNoteTags[0].text()).toBe('tag3')
    })

    it('does not show tags container when note has no tags', () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      const noteItems = wrapper.findAll('.note-item')

      // Third note has empty tags array
      expect(noteItems[2].find('.note-item-tags').exists()).toBe(false)
    })

    it('truncates tags to first 3 and shows count', () => {
      const noteWithManyTags = [
        {
          id: 'note-1',
          title: 'Note with many tags',
          status: 'draft',
          tags: ['tag1', 'tag2', 'tag3', 'tag4', 'tag5'],
          link_count: 0,
        },
      ]

      wrapper = mount(NoteList, {
        props: { notes: noteWithManyTags },
      })

      const tags = wrapper.findAll('.tag')

      // Should show first 3 tags + count indicator
      expect(tags).toHaveLength(4)
      expect(tags[0].text()).toBe('tag1')
      expect(tags[1].text()).toBe('tag2')
      expect(tags[2].text()).toBe('tag3')
      expect(tags[3].text()).toBe('+2') // 5 - 3 = 2 more tags
    })

    it('does not show count indicator when 3 or fewer tags', () => {
      const noteWithThreeTags = [
        {
          id: 'note-1',
          title: 'Note',
          status: 'draft',
          tags: ['tag1', 'tag2', 'tag3'],
          link_count: 0,
        },
      ]

      wrapper = mount(NoteList, {
        props: { notes: noteWithThreeTags },
      })

      const tags = wrapper.findAll('.tag')

      // Should show exactly 3 tags, no count indicator
      expect(tags).toHaveLength(3)
      expect(tags[0].text()).toBe('tag1')
      expect(tags[1].text()).toBe('tag2')
      expect(tags[2].text()).toBe('tag3')
    })

    it('handles notes with null or undefined tags', () => {
      const notesWithNullTags = [
        { id: 'note-1', title: 'Test', status: 'draft', tags: null },
        { id: 'note-2', title: 'Test 2', status: 'draft', tags: undefined },
      ]

      wrapper = mount(NoteList, {
        props: { notes: notesWithNullTags },
      })

      const noteItems = wrapper.findAll('.note-item')
      expect(noteItems[0].find('.note-item-tags').exists()).toBe(false)
      expect(noteItems[1].find('.note-item-tags').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Props Update Tests
  // ============================================================================

  describe('Props Updates', () => {
    it('updates displayed notes when notes prop changes', async () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      expect(wrapper.findAll('.note-item')).toHaveLength(3)

      const newNotes = [
        { id: 'note-4', title: 'New Note', status: 'draft', tags: [] },
      ]

      await wrapper.setProps({ notes: newNotes })

      const noteItems = wrapper.findAll('.note-item')
      expect(noteItems).toHaveLength(1)
      expect(noteItems[0].find('.note-item-title').text()).toBe('New Note')
    })

    it('clears selection when selected note is removed', async () => {
      wrapper = mount(NoteList, {
        props: {
          notes: mockNotes,
          selected: 'note-2',
        },
      })

      const initialSelected = wrapper.findAll('.note-item').filter((item) =>
        item.classes().includes('selected')
      )
      expect(initialSelected).toHaveLength(1)

      // Remove the selected note
      await wrapper.setProps({
        notes: mockNotes.filter((n) => n.id !== 'note-2'),
      })

      const selectedAfterRemoval = wrapper.findAll('.note-item').filter((item) =>
        item.classes().includes('selected')
      )
      expect(selectedAfterRemoval).toHaveLength(0)
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles very long note titles gracefully', () => {
      const longTitleNote = [
        {
          id: 'note-1',
          title: 'A'.repeat(200),
          status: 'draft',
          tags: [],
        },
      ]

      wrapper = mount(NoteList, {
        props: { notes: longTitleNote },
      })

      const title = wrapper.find('.note-item-title')
      expect(title.text()).toBe('A'.repeat(200))
      // CSS should handle overflow with ellipsis
    })

    it('handles notes with many tags', () => {
      const manyTagsNote = [
        {
          id: 'note-1',
          title: 'Note',
          status: 'draft',
          tags: Array.from({ length: 20 }, (_, i) => `tag${i + 1}`),
        },
      ]

      wrapper = mount(NoteList, {
        props: { notes: manyTagsNote },
      })

      const tags = wrapper.findAll('.tag')
      // Should show 3 tags + count
      expect(tags).toHaveLength(4)
      expect(tags[3].text()).toBe('+17')
    })

    it('handles notes with special characters in title', () => {
      const specialCharsNote = [
        {
          id: 'note-1',
          title: '特殊字符 🎉 @#$%',
          status: 'draft',
          tags: [],
        },
      ]

      wrapper = mount(NoteList, {
        props: { notes: specialCharsNote },
      })

      expect(wrapper.find('.note-item-title').text()).toBe('特殊字符 🎉 @#$%')
    })

    it('handles rapid prop changes', async () => {
      wrapper = mount(NoteList, {
        props: { notes: mockNotes },
      })

      for (let i = 0; i < 10; i++) {
        await wrapper.setProps({
          notes: mockNotes.slice(0, (i % 3) + 1),
        })
      }

      // Should still render correctly
      const noteItems = wrapper.findAll('.note-item')
      expect(noteItems.length).toBeGreaterThan(0)
    })
  })
})
