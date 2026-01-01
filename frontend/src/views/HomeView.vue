<template>
  <div class="home-view">
    <div class="app-container">
      <!-- Header -->
      <header class="app-header">
        <div class="header-left">
          <h1 class="logo">Seedream</h1>
        </div>
        <div class="header-center">
          <SearchBar @select="handleSearchSelect" />
        </div>
        <div class="header-right">
          <button class="btn btn-primary" @click="handleNewNote">
            + New Note
          </button>
        </div>
      </header>

      <!-- Main Content -->
      <div class="app-main">
        <!-- Sidebar -->
        <aside class="sidebar">
          <NoteList
            :notes="notes"
            :selected="selectedNoteId"
            @select="handleNoteSelect"
          />
        </aside>

        <!-- Editor Area -->
        <main class="editor-area">
          <NoteEditor
            v-if="selectedNote"
            :note="selectedNote"
            @save="handleSaveNote"
            @delete="handleDeleteNote"
          />
          <div v-else class="empty-state">
            <h2>Select a note or create a new one</h2>
            <p class="text-muted">Your knowledge awaits</p>
          </div>
        </main>

        <!-- Right Panel -->
        <aside v-if="selectedNoteId" class="right-panel">
          <BacklinksPanel :note-id="selectedNoteId" @navigate="handleNoteSelect" />
        </aside>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { notes as notesApi } from '../api/client'
import SearchBar from '../components/SearchBar.vue'
import NoteList from '../components/NoteList.vue'
import NoteEditor from '../components/NoteEditor.vue'
import BacklinksPanel from '../components/BacklinksPanel.vue'

const notes = ref([])
const selectedNoteId = ref(null)
const selectedNote = ref(null)

// Load notes on mount
onMounted(async () => {
  await loadNotes()
})

async function loadNotes() {
  try {
    const data = await notesApi.list()
    notes.value = data
  } catch (error) {
    console.error('Failed to load notes:', error)
  }
}

function handleSearchSelect(noteId) {
  selectedNoteId.value = noteId
  loadSelectedNote()
}

function handleNoteSelect(noteId) {
  selectedNoteId.value = noteId
  loadSelectedNote()
}

async function loadSelectedNote() {
  if (!selectedNoteId.value) {
    selectedNote.value = null
    return
  }

  try {
    const note = await notesApi.get(selectedNoteId.value)
    selectedNote.value = note
  } catch (error) {
    console.error('Failed to load note:', error)
  }
}

function handleNewNote() {
  selectedNoteId.value = null
  selectedNote.value = {
    id: '',
    title: '',
    content: '',
    status: 'draft',
    tags: []
  }
}

async function handleSaveNote(id, data) {
  try {
    if (id) {
      await notesApi.update(id, data)
    } else {
      const created = await notesApi.create(data)
      selectedNoteId.value = created.id
    }
    await loadNotes()
    await loadSelectedNote()
  } catch (error) {
    console.error('Failed to save note:', error)
  }
}

async function handleDeleteNote(id) {
  if (!confirm('Are you sure you want to delete this note?')) {
    return
  }

  try {
    await notesApi.delete(id)
    selectedNoteId.value = null
    selectedNote.value = null
    await loadNotes()
  } catch (error) {
    console.error('Failed to delete note:', error)
  }
}
</script>

<style scoped>
.home-view {
  width: 100%;
  height: 100vh;
}

.app-container {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.app-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--bg-tertiary);
  height: 64px;
}

.header-left {
  flex: 0 0 auto;
}

.logo {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--accent-primary);
  margin: 0;
}

.header-center {
  flex: 1;
  max-width: 600px;
  margin: 0 var(--spacing-lg);
}

.header-right {
  flex: 0 0 auto;
}

.app-main {
  display: flex;
  flex: 1;
  overflow: hidden;
}

.sidebar {
  width: 280px;
  background: var(--bg-secondary);
  border-right: 1px solid var(--bg-tertiary);
  overflow-y: auto;
}

.editor-area {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-lg);
}

.right-panel {
  width: 300px;
  background: var(--bg-secondary);
  border-left: 1px solid var(--bg-tertiary);
  overflow-y: auto;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  text-align: center;
}

.empty-state h2 {
  margin-bottom: var(--spacing-sm);
  color: var(--text-secondary);
}
</style>
