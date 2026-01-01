import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { notes as notesApi } from '../api/client'

export const useNotesStore = defineStore('notes', () => {
  // State
  const notes = ref([])
  const selectedNoteId = ref(null)
  const selectedNote = ref(null)
  const loading = ref(false)
  const error = ref(null)

  // Getters
  const selectedNoteComputed = computed(() => {
    return notes.value.find(note => note.id === selectedNoteId.value) || null
  })

  const notesByStatus = computed(() => {
    const grouped = {}
    notes.value.forEach(note => {
      if (!grouped[note.status]) {
        grouped[note.status] = []
      }
      grouped[note.status].push(note)
    })
    return grouped
  })

  const notesCount = computed(() => notes.value.length)

  // Actions
  async function loadNotes() {
    loading.value = true
    error.value = null
    try {
      const data = await notesApi.list()
      notes.value = data
    } catch (err) {
      error.value = err.message || 'Failed to load notes'
      console.error('Failed to load notes:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function loadNote(id) {
    loading.value = true
    error.value = null
    try {
      const note = await notesApi.get(id)
      selectedNote.value = note
      selectedNoteId.value = id
      return note
    } catch (err) {
      error.value = err.message || 'Failed to load note'
      console.error('Failed to load note:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function createNote(data) {
    loading.value = true
    error.value = null
    try {
      const created = await notesApi.create(data)
      notes.value.push(created)
      return created
    } catch (err) {
      error.value = err.message || 'Failed to create note'
      console.error('Failed to create note:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function updateNote(id, data) {
    loading.value = true
    error.value = null
    try {
      const updated = await notesApi.update(id, data)
      const index = notes.value.findIndex(note => note.id === id)
      if (index !== -1) {
        notes.value[index] = updated
      }
      if (selectedNoteId.value === id) {
        selectedNote.value = updated
      }
      return updated
    } catch (err) {
      error.value = err.message || 'Failed to update note'
      console.error('Failed to update note:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function deleteNote(id) {
    loading.value = true
    error.value = null
    try {
      await notesApi.delete(id)
      notes.value = notes.value.filter(note => note.id !== id)
      if (selectedNoteId.value === id) {
        selectedNoteId.value = null
        selectedNote.value = null
      }
    } catch (err) {
      error.value = err.message || 'Failed to delete note'
      console.error('Failed to delete note:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  async function reindexNotes() {
    loading.value = true
    error.value = null
    try {
      await notesApi.reindex()
      await loadNotes()
    } catch (err) {
      error.value = err.message || 'Failed to reindex notes'
      console.error('Failed to reindex notes:', err)
      throw err
    } finally {
      loading.value = false
    }
  }

  function selectNote(id) {
    selectedNoteId.value = id
    selectedNote.value = notes.value.find(note => note.id === id) || null
  }

  function clearSelection() {
    selectedNoteId.value = null
    selectedNote.value = null
  }

  function reset() {
    notes.value = []
    selectedNoteId.value = null
    selectedNote.value = null
    loading.value = false
    error.value = null
  }

  return {
    // State
    notes,
    selectedNoteId,
    selectedNote,
    loading,
    error,
    // Getters
    selectedNoteComputed,
    notesByStatus,
    notesCount,
    // Actions
    loadNotes,
    loadNote,
    createNote,
    updateNote,
    deleteNote,
    reindexNotes,
    selectNote,
    clearSelection,
    reset,
  }
})
