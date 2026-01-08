<template>
  <div class="home-view">
    <div class="app-container">
      <!-- Header -->
      <header class="app-header">
        <div class="header-left">
          <div class="logo-wrapper" @click="toggleFullGraph">
            <svg class="logo-icon" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor">
              <circle cx="12" cy="12" r="10"></circle>
              <line x1="2" y1="12" x2="22" y2="12"></line>
              <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path>
            </svg>
            <h1 class="logo">Seedream</h1>
          </div>
        </div>
        <div class="header-center">
          <SearchBar @select="handleSearchSelect" />
        </div>
        <div class="header-right">
          <div class="action-buttons">
            <router-link to="/canvas" class="btn btn-secondary" title="Multi-LLM Canvas">
              Canvas
            </router-link>
            <button class="btn btn-ghost" title="Toggle Theme">🌙</button>
            <button class="btn btn-primary" @click="handleNewNote">
              + New Note
            </button>
          </div>
        </div>
      </header>

      <!-- Main Content -->
      <div class="app-main">
        <!-- Left Sidebar: Navigation -->
        <aside class="sidebar-left">
          <TreeNav 
            :notes="notes" 
            :selected-id="selectedNoteId"
            @select="handleNoteSelect"
          />
        </aside>

        <!-- Center: Editor or Full Graph -->
        <main class="main-content">
          <div v-if="showFullGraph" class="full-graph-container">
            <div class="graph-header">
              <h2>Knowledge Graph</h2>
              <button class="close-btn" @click="showFullGraph = false">×</button>
            </div>
            <GraphView @node-click="handleGraphNodeClick" />
          </div>
          
          <div v-else class="editor-container">
            <NoteEditor
              v-if="selectedNote"
              :note="selectedNote"
              @save="handleSaveNote"
              @delete="handleDeleteNote"
            />
            <div v-else class="empty-state">
              <div class="empty-icon">📝</div>
              <h2>Select a note to view</h2>
              <p class="text-muted">Or create a new one to start writing</p>
            </div>
          </div>
        </main>

        <!-- Right Sidebar: Info & Mini Graph -->
        <aside class="sidebar-right">
          <div class="sidebar-section">
            <div class="section-title">Interactive Graph</div>
            <MiniGraph @navigate="handleNoteSelect" />
          </div>
          
          <div class="sidebar-section" v-if="selectedNote">
            <OnThisPage :content="selectedNote.content" />
          </div>
          
          <div class="sidebar-section" v-if="selectedNoteId">
            <div class="section-title">Backlinks</div>
            <BacklinksPanel :note-id="selectedNoteId" @navigate="handleNoteSelect" />
          </div>
        </aside>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { notes as notesApi } from '../api/client'
import SearchBar from '../components/SearchBar.vue'
import NoteEditor from '../components/NoteEditor.vue'
import BacklinksPanel from '../components/BacklinksPanel.vue'
import TreeNav from '../components/TreeNav.vue'
import MiniGraph from '../components/MiniGraph.vue'
import OnThisPage from '../components/OnThisPage.vue'
import GraphView from '../components/GraphView.vue'

const notes = ref([])
const selectedNoteId = ref(null)
const selectedNote = ref(null)
const showFullGraph = ref(false)

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
  showFullGraph.value = false
  loadSelectedNote()
}

function handleNoteSelect(noteId) {
  selectedNoteId.value = noteId
  showFullGraph.value = false
  loadSelectedNote()
}

function handleGraphNodeClick(noteId) {
  selectedNoteId.value = noteId
  showFullGraph.value = false
  loadSelectedNote()
}

function toggleFullGraph() {
  showFullGraph.value = !showFullGraph.value
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
  showFullGraph.value = false
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
  display: flex;
  flex-direction: column;
}

.app-container {
  display: flex;
  flex-direction: column;
  height: 100%;
}

/* Header */
.app-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 var(--spacing-lg);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--bg-tertiary);
  height: 56px;
  flex: 0 0 auto;
}

.header-left {
  flex: 0 0 250px; /* Match sidebar width */
}

.logo-wrapper {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: var(--radius-md);
  transition: background var(--transition-fast);
}

.logo-wrapper:hover {
  background: var(--bg-hover);
}

.logo-icon {
  color: var(--accent-primary);
}

.logo {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.header-center {
  flex: 1;
  max-width: 600px;
}

.header-right {
  flex: 0 0 280px; /* Match right sidebar width */
  display: flex;
  justify-content: flex-end;
}

/* Main Layout */
.app-main {
  display: flex;
  flex: 1;
  overflow: hidden;
}

.sidebar-left {
  width: 250px;
  background: var(--bg-secondary);
  border-right: 1px solid var(--bg-tertiary);
  overflow-y: auto;
  flex: 0 0 auto;
}

.main-content {
  flex: 1;
  background: var(--bg-primary);
  position: relative;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.sidebar-right {
  width: 280px;
  background: var(--bg-secondary);
  border-left: 1px solid var(--bg-tertiary);
  overflow-y: auto;
  flex: 0 0 auto;
  padding: var(--spacing-md);
}

/* Sidebar Sections */
.sidebar-section {
  margin-bottom: var(--spacing-xl);
}

.section-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
  margin-bottom: var(--spacing-sm);
  letter-spacing: 0.05em;
}

/* Full Graph View */
.full-graph-container {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: var(--spacing-md);
}

.graph-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--spacing-md);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0 8px;
}

.close-btn:hover {
  color: var(--text-primary);
}

/* Editor Container */
.editor-container {
  flex: 1;
  padding: var(--spacing-xl) 10%;
  max-width: 900px;
  margin: 0 auto;
  width: 100%;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 60vh;
  text-align: center;
  opacity: 0.6;
}

.empty-icon {
  font-size: 3rem;
  margin-bottom: var(--spacing-md);
}
</style>
