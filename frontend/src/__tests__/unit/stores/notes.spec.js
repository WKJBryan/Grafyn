/**
 * Unit tests for notes Pinia store
 *
 * Tests cover:
 * - Initial state
 * - Computed properties (selectedNoteComputed, notesByStatus, notesCount)
 * - loadNotes() action with loading/error states
 * - loadNote() action
 * - createNote() action with state updates
 * - updateNote() action with array mutation and selectedNote sync
 * - deleteNote() action with cleanup
 * - reindexNotes() action
 * - selectNote() synchronous action
 * - clearSelection() action
 * - reset() action
 * - Error handling for all async actions
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useNotesStore } from '@/stores/notes'
import * as apiClient from '@/api/client'

describe('Notes Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  // ============================================================================
  // Initial State Tests
  // ============================================================================

  describe('Initial State', () => {
    it('has empty notes array', () => {
      const store = useNotesStore()

      expect(store.notes).toEqual([])
    })

    it('has null selectedNoteId', () => {
      const store = useNotesStore()

      expect(store.selectedNoteId).toBeNull()
    })

    it('has null selectedNote', () => {
      const store = useNotesStore()

      expect(store.selectedNote).toBeNull()
    })

    it('has loading set to false', () => {
      const store = useNotesStore()

      expect(store.loading).toBe(false)
    })

    it('has null error', () => {
      const store = useNotesStore()

      expect(store.error).toBeNull()
    })
  })

  // ============================================================================
  // Computed Properties Tests
  // ============================================================================

  describe('Computed Properties', () => {
    it('selectedNoteComputed returns null when no selection', () => {
      const store = useNotesStore()

      expect(store.selectedNoteComputed).toBeNull()
    })

    it('selectedNoteComputed finds note by selectedNoteId', () => {
      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
        { id: 'note-3', title: 'Third' },
      ]
      store.selectedNoteId = 'note-2'

      expect(store.selectedNoteComputed).toEqual({
        id: 'note-2',
        title: 'Second',
      })
    })

    it('selectedNoteComputed returns null for invalid ID', () => {
      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'First' }]
      store.selectedNoteId = 'nonexistent'

      expect(store.selectedNoteComputed).toBeNull()
    })

    it('notesByStatus groups notes by status', () => {
      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First', status: 'draft' },
        { id: 'note-2', title: 'Second', status: 'canonical' },
        { id: 'note-3', title: 'Third', status: 'draft' },
        { id: 'note-4', title: 'Fourth', status: 'evidence' },
      ]

      const grouped = store.notesByStatus

      expect(grouped.draft).toHaveLength(2)
      expect(grouped.canonical).toHaveLength(1)
      expect(grouped.evidence).toHaveLength(1)
      expect(grouped.draft[0].id).toBe('note-1')
      expect(grouped.draft[1].id).toBe('note-3')
    })

    it('notesByStatus returns empty object when no notes', () => {
      const store = useNotesStore()

      expect(store.notesByStatus).toEqual({})
    })

    it('notesCount returns number of notes', () => {
      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
        { id: 'note-3', title: 'Third' },
      ]

      expect(store.notesCount).toBe(3)
    })

    it('notesCount returns 0 when no notes', () => {
      const store = useNotesStore()

      expect(store.notesCount).toBe(0)
    })
  })

  // ============================================================================
  // loadNotes() Tests
  // ============================================================================

  describe('loadNotes()', () => {
    it('sets loading to true during request', async () => {
      let resolve
      vi.spyOn(apiClient.notes, 'list').mockImplementation(
        () => new Promise((r) => { resolve = r })
      )

      const store = useNotesStore()
      const promise = store.loadNotes()

      expect(store.loading).toBe(true)

      resolve([])
      await promise
    })

    it('populates notes array on success', async () => {
      const mockNotes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
      ]
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue(mockNotes)

      const store = useNotesStore()
      await store.loadNotes()

      expect(store.notes).toEqual(mockNotes)
    })

    it('sets loading to false after success', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      const store = useNotesStore()
      await store.loadNotes()

      expect(store.loading).toBe(false)
    })

    it('clears error on success', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      const store = useNotesStore()
      store.error = 'Previous error'

      await store.loadNotes()

      expect(store.error).toBeNull()
    })

    it('sets error message on failure', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'list').mockRejectedValue(
        new Error('Network error')
      )

      const store = useNotesStore()

      await expect(store.loadNotes()).rejects.toThrow()

      expect(store.error).toBe('Network error')
    })

    it('sets loading to false after failure', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'list').mockRejectedValue(new Error('Failed'))

      const store = useNotesStore()

      await expect(store.loadNotes()).rejects.toThrow()

      expect(store.loading).toBe(false)
    })
  })

  // ============================================================================
  // loadNote() Tests
  // ============================================================================

  describe('loadNote()', () => {
    it('loads single note and sets selectedNote', async () => {
      const mockNote = { id: 'note-1', title: 'Test Note', content: 'Content' }
      vi.spyOn(apiClient.notes, 'get').mockResolvedValue(mockNote)

      const store = useNotesStore()
      const result = await store.loadNote('note-1')

      expect(store.selectedNote).toEqual(mockNote)
      expect(result).toEqual(mockNote)
    })

    it('sets selectedNoteId', async () => {
      const mockNote = { id: 'note-1', title: 'Test' }
      vi.spyOn(apiClient.notes, 'get').mockResolvedValue(mockNote)

      const store = useNotesStore()
      await store.loadNote('note-1')

      expect(store.selectedNoteId).toBe('note-1')
    })

    it('sets loading state during request', async () => {
      let resolve
      vi.spyOn(apiClient.notes, 'get').mockImplementation(
        () => new Promise((r) => { resolve = r })
      )

      const store = useNotesStore()
      const promise = store.loadNote('note-1')

      expect(store.loading).toBe(true)

      resolve({ id: 'note-1', title: 'Test' })
      await promise
    })

    it('handles errors correctly', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'get').mockRejectedValue(
        new Error('Note not found')
      )

      const store = useNotesStore()

      await expect(store.loadNote('nonexistent')).rejects.toThrow()

      expect(store.error).toBe('Note not found')
    })
  })

  // ============================================================================
  // createNote() Tests
  // ============================================================================

  describe('createNote()', () => {
    it('creates note and adds to notes array', async () => {
      const newNoteData = { title: 'New Note', content: 'Content' }
      const createdNote = { id: 'note-1', ...newNoteData }
      vi.spyOn(apiClient.notes, 'create').mockResolvedValue(createdNote)

      const store = useNotesStore()
      const result = await store.createNote(newNoteData)

      expect(store.notes).toHaveLength(1)
      expect(store.notes[0]).toEqual(createdNote)
      expect(result).toEqual(createdNote)
    })

    it('appends to existing notes array', async () => {
      const existingNote = { id: 'note-1', title: 'Existing' }
      const newNote = { id: 'note-2', title: 'New' }
      vi.spyOn(apiClient.notes, 'create').mockResolvedValue(newNote)

      const store = useNotesStore()
      store.notes = [existingNote]

      await store.createNote({ title: 'New' })

      expect(store.notes).toHaveLength(2)
      expect(store.notes[1]).toEqual(newNote)
    })

    it('handles creation errors', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'create').mockRejectedValue(
        new Error('Validation error')
      )

      const store = useNotesStore()

      await expect(store.createNote({ title: '' })).rejects.toThrow()

      expect(store.error).toBe('Validation error')
      expect(store.notes).toHaveLength(0)
    })
  })

  // ============================================================================
  // updateNote() Tests
  // ============================================================================

  describe('updateNote()', () => {
    it('updates note in notes array', async () => {
      const updatedNote = {
        id: 'note-2',
        title: 'Updated Title',
        content: 'Updated',
      }
      vi.spyOn(apiClient.notes, 'update').mockResolvedValue(updatedNote)

      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Original' },
        { id: 'note-3', title: 'Third' },
      ]

      await store.updateNote('note-2', { title: 'Updated Title' })

      expect(store.notes[1]).toEqual(updatedNote)
      expect(store.notes[0].title).toBe('First') // Others unchanged
      expect(store.notes[2].title).toBe('Third')
    })

    it('updates selectedNote if selected note is updated', async () => {
      const updatedNote = { id: 'note-1', title: 'Updated' }
      vi.spyOn(apiClient.notes, 'update').mockResolvedValue(updatedNote)

      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'Original' }]
      store.selectedNoteId = 'note-1'
      store.selectedNote = { id: 'note-1', title: 'Original' }

      await store.updateNote('note-1', { title: 'Updated' })

      expect(store.selectedNote).toEqual(updatedNote)
    })

    it('does not update selectedNote if different note is updated', async () => {
      const updatedNote = { id: 'note-2', title: 'Updated' }
      vi.spyOn(apiClient.notes, 'update').mockResolvedValue(updatedNote)

      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Original' },
      ]
      store.selectedNoteId = 'note-1'
      store.selectedNote = { id: 'note-1', title: 'First' }

      await store.updateNote('note-2', { title: 'Updated' })

      expect(store.selectedNote.title).toBe('First')
    })

    it('handles update errors', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'update').mockRejectedValue(
        new Error('Update failed')
      )

      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'Original' }]

      await expect(
        store.updateNote('note-1', { title: 'New' })
      ).rejects.toThrow()

      expect(store.error).toBe('Update failed')
      expect(store.notes[0].title).toBe('Original') // Unchanged
    })
  })

  // ============================================================================
  // deleteNote() Tests
  // ============================================================================

  describe('deleteNote()', () => {
    it('removes note from notes array', async () => {
      vi.spyOn(apiClient.notes, 'delete').mockResolvedValue()

      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
        { id: 'note-3', title: 'Third' },
      ]

      await store.deleteNote('note-2')

      expect(store.notes).toHaveLength(2)
      expect(store.notes.find((n) => n.id === 'note-2')).toBeUndefined()
    })

    it('clears selection if selected note is deleted', async () => {
      vi.spyOn(apiClient.notes, 'delete').mockResolvedValue()

      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
      ]
      store.selectedNoteId = 'note-2'
      store.selectedNote = { id: 'note-2', title: 'Second' }

      await store.deleteNote('note-2')

      expect(store.selectedNoteId).toBeNull()
      expect(store.selectedNote).toBeNull()
    })

    it('does not clear selection if different note is deleted', async () => {
      vi.spyOn(apiClient.notes, 'delete').mockResolvedValue()

      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
      ]
      store.selectedNoteId = 'note-1'
      store.selectedNote = { id: 'note-1', title: 'First' }

      await store.deleteNote('note-2')

      expect(store.selectedNoteId).toBe('note-1')
      expect(store.selectedNote).toEqual({ id: 'note-1', title: 'First' })
    })

    it('handles delete errors', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'delete').mockRejectedValue(
        new Error('Delete failed')
      )

      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'First' }]

      await expect(store.deleteNote('note-1')).rejects.toThrow()

      expect(store.error).toBe('Delete failed')
      expect(store.notes).toHaveLength(1) // Unchanged
    })
  })

  // ============================================================================
  // reindexNotes() Tests
  // ============================================================================

  describe('reindexNotes()', () => {
    it('calls reindex API and reloads notes', async () => {
      const reindexSpy = vi
        .spyOn(apiClient.notes, 'reindex')
        .mockResolvedValue()
      const listSpy = vi.spyOn(apiClient.notes, 'list').mockResolvedValue([
        { id: 'note-1', title: 'Refreshed' },
      ])

      const store = useNotesStore()
      await store.reindexNotes()

      expect(reindexSpy).toHaveBeenCalledTimes(1)
      expect(listSpy).toHaveBeenCalledTimes(1)
      expect(store.notes).toHaveLength(1)
    })

    it('handles reindex errors', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'reindex').mockRejectedValue(
        new Error('Reindex failed')
      )

      const store = useNotesStore()

      await expect(store.reindexNotes()).rejects.toThrow()

      expect(store.error).toBe('Reindex failed')
    })
  })

  // ============================================================================
  // selectNote() Tests
  // ============================================================================

  describe('selectNote()', () => {
    it('sets selectedNoteId and selectedNote', () => {
      const store = useNotesStore()
      store.notes = [
        { id: 'note-1', title: 'First' },
        { id: 'note-2', title: 'Second' },
      ]

      store.selectNote('note-2')

      expect(store.selectedNoteId).toBe('note-2')
      expect(store.selectedNote).toEqual({ id: 'note-2', title: 'Second' })
    })

    it('sets selectedNote to null if note not found', () => {
      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'First' }]

      store.selectNote('nonexistent')

      expect(store.selectedNoteId).toBe('nonexistent')
      expect(store.selectedNote).toBeNull()
    })
  })

  // ============================================================================
  // clearSelection() Tests
  // ============================================================================

  describe('clearSelection()', () => {
    it('clears selectedNoteId and selectedNote', () => {
      const store = useNotesStore()
      store.selectedNoteId = 'note-1'
      store.selectedNote = { id: 'note-1', title: 'Test' }

      store.clearSelection()

      expect(store.selectedNoteId).toBeNull()
      expect(store.selectedNote).toBeNull()
    })
  })

  // ============================================================================
  // reset() Tests
  // ============================================================================

  describe('reset()', () => {
    it('resets all state to initial values', () => {
      const store = useNotesStore()
      store.notes = [{ id: 'note-1', title: 'Test' }]
      store.selectedNoteId = 'note-1'
      store.selectedNote = { id: 'note-1', title: 'Test' }
      store.loading = true
      store.error = 'Some error'

      store.reset()

      expect(store.notes).toEqual([])
      expect(store.selectedNoteId).toBeNull()
      expect(store.selectedNote).toBeNull()
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles API returning null', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue(null)

      const store = useNotesStore()
      await store.loadNotes()

      expect(store.notes).toBeNull()
    })

    it('handles error without message property', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      vi.spyOn(apiClient.notes, 'list').mockRejectedValue('String error')

      const store = useNotesStore()

      await expect(store.loadNotes()).rejects.toBe('String error')

      expect(store.error).toBe('Failed to load notes')
    })

    it('handles concurrent operations', async () => {
      const mockNotes = [{ id: 'note-1', title: 'Test' }]
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue(mockNotes)
      vi.spyOn(apiClient.notes, 'create').mockResolvedValue({
        id: 'note-2',
        title: 'New',
      })

      const store = useNotesStore()

      await Promise.all([store.loadNotes(), store.createNote({ title: 'New' })])

      // Both operations should complete
      expect(store.notes.length).toBeGreaterThan(0)
    })
  })
})
