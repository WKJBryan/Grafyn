/**
 * Unit tests for NoteEditor component
 *
 * Tests cover:
 * - Component rendering with note props
 * - Edit/Preview mode switching
 * - Dirty state tracking
 * - Markdown rendering
 * - Wikilink rendering [[Note Title]] and [[Title|Display]]
 * - Save validation (title required)
 * - Delete confirmation
 * - Tag parsing (comma-separated)
 * - Status selection
 * - Event emissions
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import NoteEditor from '@/components/NoteEditor.vue'

describe('NoteEditor', () => {
  let wrapper
  let mockNote

  beforeEach(() => {
    mockNote = {
      id: 'test-note-1',
      title: 'Test Note',
      content: 'Test content',
      status: 'draft',
      tags: ['test', 'sample'],
    }
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
    it('renders the component with note data', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      expect(wrapper.find('.note-editor').exists()).toBe(true)
      expect(wrapper.find('.title-input').element.value).toBe('Test Note')
      expect(wrapper.find('.editor-textarea').element.value).toBe('Test content')
    })

    it('renders title input field', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const titleInput = wrapper.find('.title-input')
      expect(titleInput.exists()).toBe(true)
      expect(titleInput.attributes('placeholder')).toBe('Note title...')
    })

    it('renders save and delete buttons', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      expect(wrapper.text()).toContain('Save')
      expect(wrapper.text()).toContain('Delete')
    })

    it('does not render delete button for new note without ID', () => {
      const newNote = { ...mockNote, id: null }
      wrapper = mount(NoteEditor, {
        props: { note: newNote },
      })

      const deleteButtons = wrapper.findAll('button').filter((btn) =>
        btn.text().includes('Delete')
      )
      expect(deleteButtons).toHaveLength(0)
    })

    it('renders status select with options', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const statusSelect = wrapper.find('.status-select')
      expect(statusSelect.exists()).toBe(true)

      const options = statusSelect.findAll('option')
      expect(options).toHaveLength(3)
      expect(options[0].text()).toBe('Draft')
      expect(options[1].text()).toBe('Canonical')
      expect(options[2].text()).toBe('Evidence')
    })

    it('renders tags input with comma-separated tags', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const tagsInput = wrapper.find('.tags-input')
      expect(tagsInput.element.value).toBe('test, sample')
    })

    it('renders empty tags input when note has no tags', () => {
      const noteWithoutTags = { ...mockNote, tags: [] }
      wrapper = mount(NoteEditor, {
        props: { note: noteWithoutTags },
      })

      const tagsInput = wrapper.find('.tags-input')
      expect(tagsInput.element.value).toBe('')
    })
  })

  // ============================================================================
  // Mode Switching Tests
  // ============================================================================

  describe('Mode Switching', () => {
    it('starts in edit mode by default', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const editTab = wrapper.findAll('.tab-btn')[0]
      expect(editTab.classes()).toContain('active')
      expect(wrapper.find('.editor-textarea').exists()).toBe(true)
    })

    it('switches to preview mode when preview tab is clicked', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const previewTab = wrapper.findAll('.tab-btn')[1]
      await previewTab.trigger('click')

      expect(previewTab.classes()).toContain('active')
      expect(wrapper.find('.editor-preview').exists()).toBe(true)
      expect(wrapper.find('.editor-textarea').exists()).toBe(false)
    })

    it('switches back to edit mode when edit tab is clicked', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      // Switch to preview
      const previewTab = wrapper.findAll('.tab-btn')[1]
      await previewTab.trigger('click')

      // Switch back to edit
      const editTab = wrapper.findAll('.tab-btn')[0]
      await editTab.trigger('click')

      expect(editTab.classes()).toContain('active')
      expect(wrapper.find('.editor-textarea').exists()).toBe(true)
    })

    it('shows correct tab as active', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const tabs = wrapper.findAll('.tab-btn')
      const editTab = tabs[0]
      const previewTab = tabs[1]

      // Initially edit is active
      expect(editTab.classes()).toContain('active')
      expect(previewTab.classes()).not.toContain('active')

      // Click preview
      await previewTab.trigger('click')

      expect(editTab.classes()).not.toContain('active')
      expect(previewTab.classes()).toContain('active')
    })
  })

  // ============================================================================
  // Dirty State Tests
  // ============================================================================

  describe('Dirty State Tracking', () => {
    it('save button is disabled when not dirty', () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeDefined()
    })

    it('save button is enabled after title change', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const titleInput = wrapper.find('.title-input')
      await titleInput.setValue('New Title')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeUndefined()
    })

    it('save button is enabled after content change', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const textarea = wrapper.find('.editor-textarea')
      await textarea.setValue('New content')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeUndefined()
    })

    it('save button is enabled after status change', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const statusSelect = wrapper.find('.status-select')
      await statusSelect.setValue('canonical')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeUndefined()
    })

    it('save button is enabled after tags change', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const tagsInput = wrapper.find('.tags-input')
      await tagsInput.setValue('new, tags')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeUndefined()
    })

    it('resets dirty state when props update', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      // Make it dirty
      await wrapper.find('.title-input').setValue('Changed')

      // Update props
      await wrapper.setProps({
        note: { ...mockNote, title: 'Updated from parent' },
      })

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      expect(saveButton.attributes('disabled')).toBeDefined()
    })
  })

  // ============================================================================
  // Markdown Rendering Tests
  // ============================================================================

  describe('Markdown Rendering', () => {
    it('renders markdown in preview mode', async () => {
      const noteWithMarkdown = {
        ...mockNote,
        content: '# Heading\n\n**Bold text**\n\n- List item',
      }

      wrapper = mount(NoteEditor, {
        props: { note: noteWithMarkdown },
      })

      // Switch to preview
      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      const html = preview.html()

      expect(html).toContain('<h1')
      expect(html).toContain('Heading')
      expect(html).toContain('<strong>')
      expect(html).toContain('Bold text')
      expect(html).toContain('<li>')
    })

    it('renders code blocks in preview', async () => {
      const noteWithCode = {
        ...mockNote,
        content: '```javascript\nconst x = 5;\n```',
      }

      wrapper = mount(NoteEditor, {
        props: { note: noteWithCode },
      })

      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      const html = preview.html()

      expect(html).toContain('<code>')
    })

    it('handles empty content in preview', async () => {
      const emptyNote = { ...mockNote, content: '' }

      wrapper = mount(NoteEditor, {
        props: { note: emptyNote },
      })

      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      expect(preview.html()).toBeTruthy()
    })
  })

  // ============================================================================
  // Wikilink Rendering Tests
  // ============================================================================

  describe('Wikilink Rendering', () => {
    it('renders simple wikilinks [[Note Title]]', async () => {
      const noteWithWikilink = {
        ...mockNote,
        content: 'This links to [[Target Note]]',
      }

      wrapper = mount(NoteEditor, {
        props: { note: noteWithWikilink },
      })

      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      const html = preview.html()

      expect(html).toContain('class="wikilink"')
      expect(html).toContain('data-target="Target Note"')
      expect(html).toContain('Target Note')
    })

    it('renders wikilinks with display text [[Target|Display]]', async () => {
      const noteWithWikilink = {
        ...mockNote,
        content: 'This links to [[Actual Target|Display Text]]',
      }

      wrapper = mount(NoteEditor, {
        props: { note: noteWithWikilink },
      })

      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      const html = preview.html()

      expect(html).toContain('class="wikilink"')
      expect(html).toContain('data-target="Actual Target"')
      expect(html).toContain('Display Text')
      expect(html).not.toContain('Actual Target')
    })

    it('renders multiple wikilinks', async () => {
      const noteWithMultipleLinks = {
        ...mockNote,
        content: 'Links to [[Note A]], [[Note B]], and [[Note C|C]]',
      }

      wrapper = mount(NoteEditor, {
        props: { note: noteWithMultipleLinks },
      })

      await wrapper.findAll('.tab-btn')[1].trigger('click')

      const preview = wrapper.find('.editor-preview')
      const html = preview.html()

      expect(html).toContain('data-target="Note A"')
      expect(html).toContain('data-target="Note B"')
      expect(html).toContain('data-target="Note C"')
    })
  })

  // ============================================================================
  // Save Functionality Tests
  // ============================================================================

  describe('Save Functionality', () => {
    it('emits save event with note data when save is clicked', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      // Make changes
      await wrapper.find('.title-input').setValue('Updated Title')
      await wrapper.find('.editor-textarea').setValue('Updated content')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')).toBeTruthy()
      expect(wrapper.emitted('save')[0]).toEqual([
        'test-note-1',
        {
          title: 'Updated Title',
          content: 'Updated content',
          status: 'draft',
          tags: ['test', 'sample'],
        },
      ])
    })

    it('does not save when title is empty', async () => {
      // Mock alert
      global.alert = vi.fn()

      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      // Clear title
      await wrapper.find('.title-input').setValue('')
      await wrapper.find('.editor-textarea').setValue('Some content')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(global.alert).toHaveBeenCalledWith('Please enter a title')
      expect(wrapper.emitted('save')).toBeFalsy()
    })

    it('does not save when title is only whitespace', async () => {
      global.alert = vi.fn()

      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      await wrapper.find('.title-input').setValue('   ')
      await wrapper.find('.editor-textarea').setValue('Content')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(global.alert).toHaveBeenCalled()
      expect(wrapper.emitted('save')).toBeFalsy()
    })

    it('resets dirty state after successful save', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      await wrapper.find('.title-input').setValue('Updated')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      // Check that save button is disabled again
      expect(saveButton.attributes('disabled')).toBeDefined()
    })
  })

  // ============================================================================
  // Delete Functionality Tests
  // ============================================================================

  describe('Delete Functionality', () => {
    it('shows confirmation dialog when delete is clicked', async () => {
      global.confirm = vi.fn(() => false)

      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const deleteButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Delete')
      )
      await deleteButton.trigger('click')

      expect(global.confirm).toHaveBeenCalledWith(
        'Are you sure you want to delete this note?'
      )
    })

    it('emits delete event when confirmed', async () => {
      global.confirm = vi.fn(() => true)

      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const deleteButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Delete')
      )
      await deleteButton.trigger('click')

      expect(wrapper.emitted('delete')).toBeTruthy()
      expect(wrapper.emitted('delete')[0]).toEqual(['test-note-1'])
    })

    it('does not emit delete event when cancelled', async () => {
      global.confirm = vi.fn(() => false)

      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const deleteButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Delete')
      )
      await deleteButton.trigger('click')

      expect(wrapper.emitted('delete')).toBeFalsy()
    })
  })

  // ============================================================================
  // Tag Parsing Tests
  // ============================================================================

  describe('Tag Parsing', () => {
    it('parses comma-separated tags correctly', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: { ...mockNote, tags: [] } },
      })

      const tagsInput = wrapper.find('.tags-input')
      await tagsInput.setValue('tag1, tag2, tag3')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')[0][1].tags).toEqual(['tag1', 'tag2', 'tag3'])
    })

    it('trims whitespace from tags', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: { ...mockNote, tags: [] } },
      })

      await wrapper.find('.tags-input').setValue('  tag1  ,  tag2  ,  tag3  ')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')[0][1].tags).toEqual(['tag1', 'tag2', 'tag3'])
    })

    it('filters out empty tags', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: { ...mockNote, tags: [] } },
      })

      await wrapper.find('.tags-input').setValue('tag1, , tag2, ,tag3')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')[0][1].tags).toEqual(['tag1', 'tag2', 'tag3'])
    })

    it('handles empty tags input', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      await wrapper.find('.tags-input').setValue('')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')[0][1].tags).toEqual([])
    })
  })

  // ============================================================================
  // Status Selection Tests
  // ============================================================================

  describe('Status Selection', () => {
    it('updates status when changed', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const statusSelect = wrapper.find('.status-select')
      await statusSelect.setValue('canonical')

      const saveButton = wrapper.findAll('button').find((btn) =>
        btn.text().includes('Save')
      )
      await saveButton.trigger('click')

      expect(wrapper.emitted('save')[0][1].status).toBe('canonical')
    })

    it('shows current status as selected', () => {
      wrapper = mount(NoteEditor, {
        props: { note: { ...mockNote, status: 'canonical' } },
      })

      const statusSelect = wrapper.find('.status-select')
      expect(statusSelect.element.value).toBe('canonical')
    })
  })

  // ============================================================================
  // Props Update Tests
  // ============================================================================

  describe('Props Updates', () => {
    it('updates local state when note prop changes', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      const newNote = {
        ...mockNote,
        title: 'New Title from Props',
        content: 'New content from props',
      }

      await wrapper.setProps({ note: newNote })

      expect(wrapper.find('.title-input').element.value).toBe('New Title from Props')
      expect(wrapper.find('.editor-textarea').element.value).toBe(
        'New content from props'
      )
    })

    it('updates tags input when note tags change', async () => {
      wrapper = mount(NoteEditor, {
        props: { note: mockNote },
      })

      await wrapper.setProps({
        note: { ...mockNote, tags: ['new', 'tags'] },
      })

      expect(wrapper.find('.tags-input').element.value).toBe('new, tags')
    })
  })
})
