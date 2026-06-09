import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { notes as notesApi } from '../api/client'
import { useAsyncOperation } from '../composables/useAsyncOperation'

export const useNotesStore = defineStore('notes', () => {
  // State
  const notes = ref([])
  const selectedNoteId = ref(null)
  const selectedNote = ref(null)
  const loading = ref(false)
  const error = ref(null)
  const { run } = useAsyncOperation(loading, error)

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
    return run(async () => {
      notes.value = await notesApi.list()
    })
  }

  async function loadNote(id) {
    return run(async () => {
      const note = await notesApi.get(id)
      selectedNote.value = note
      selectedNoteId.value = id
      return note
    })
  }

  async function createNote(data) {
    return run(async () => {
      const created = await notesApi.create(data)
      notes.value.push(created)
      return created
    })
  }

  async function updateNote(id, data) {
    return run(async () => {
      const updated = await notesApi.update(id, data)
      const index = notes.value.findIndex(note => note.id === id)
      if (index !== -1) {
        notes.value[index] = updated
      }
      if (selectedNoteId.value === id) {
        selectedNote.value = updated
      }
      return updated
    })
  }

  async function deleteNote(id) {
    return run(async () => {
      await notesApi.delete(id)
      notes.value = notes.value.filter(note => note.id !== id)
      if (selectedNoteId.value === id) {
        selectedNoteId.value = null
        selectedNote.value = null
      }
    })
  }

  async function reindexNotes() {
    return run(async () => {
      await notesApi.reindex()
      await loadNotes()
    })
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
