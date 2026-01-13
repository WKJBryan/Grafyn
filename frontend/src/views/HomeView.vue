<template>
  <div class="home-view">
    <div class="app-container">
      <!-- Header -->
      <header class="app-header">
        <div class="header-left">
          <div class="logo-wrapper">
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
            <button class="btn btn-ghost" @click="handleThemeToggle" title="Toggle Theme">
              {{ themeIcon }}
            </button>
            <button class="btn btn-primary" @click="handleNewNote">
              + New Note
            </button>
          </div>
        </div>
      </header>

      <!-- Main Content -->
      <div class="app-main">
        <!-- Left Sidebar: Navigation & Tags -->
        <aside class="sidebar-left">
          <TreeNav 
            :notes="filteredNotes" 
            :selected-id="selectedNoteId"
            @select="handleNoteSelect"
          />
          <div class="sidebar-section">
            <TagTree 
              :tags="allTags" 
              @filter="handleTagFilter" 
            />
          </div>
        </aside>

        <!-- Center: Graph (Always Visible) -->
        <main class="main-content">
          <div class="full-graph-container">
            <div class="graph-header">
              <h2>Knowledge Graph</h2>
            </div>
            <GraphView @node-click="handleGraphNodeClick" />
          </div>
          
          <!-- Editor Panel (Overlay) -->
          <div v-if="selectedNote" class="editor-panel-overlay">
            <div class="editor-panel">
              <div class="editor-panel-header">
                <input
                  v-model="selectedNote.title"
                  type="text"
                  class="title-input"
                  placeholder="Note title..."
                  @input="handleDirty"
                />
                <button class="close-btn" @click="handleCloseNote">×</button>
              </div>
              <NoteEditor
                :note="selectedNote"
                @save="handleSaveNote"
                @delete="handleDeleteNote"
                @close="handleCloseNote"
              />
            </div>
          </div>
        </main>

        <!-- Right Sidebar: Info, Graph, Backlinks & Mentions -->
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

          <div class="sidebar-section" v-if="selectedNoteId">
            <UnlinkedMentions 
              :note-id="selectedNoteId" 
              :note-title="selectedNote.title"
              @navigate="handleNoteSelect"
              @link-created="handleCreateLink"
            />
          </div>
        </aside>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, computed, watch } from 'vue'
import { notes as notesApi } from '../api/client'
import SearchBar from '../components/SearchBar.vue'
import NoteEditor from '../components/NoteEditor.vue'
import BacklinksPanel from '../components/BacklinksPanel.vue'
import TreeNav from '../components/TreeNav.vue'
import TagTree from '../components/TagTree.vue'
import UnlinkedMentions from '../components/UnlinkedMentions.vue'
import MiniGraph from '../components/MiniGraph.vue'
import OnThisPage from '../components/OnThisPage.vue'
import GraphView from '../components/GraphView.vue'
import { useThemeStore } from '../stores/theme'

const notes = ref([])
const selectedNoteId = ref(null)
const selectedNote = ref(null)
const themeStore = useThemeStore()
const selectedTags = ref([])
const isDirty = ref(false)

// Computed property to get the current theme icon
const themeIcon = computed(() => {
  return themeStore.theme === 'dark' ? '🌙' : '☀️'
})

// Extract all unique tags
const allTags = computed(() => {
  const tags = new Set()
  notes.value.forEach(note => {
    if (note.tags && Array.isArray(note.tags)) {
      note.tags.forEach(tag => tags.add(tag))
    }
  })
  return Array.from(tags).sort()
})

// Filter notes based on selected tags
const filteredNotes = computed(() => {
  if (selectedTags.value.length === 0) return notes.value
  
  return notes.value.filter(note => {
    // Check if note has ANY of the selected tags (or children of them)
    // In a real app, this might be ALL or ANY depending on preference
    if (!note.tags) return false
    
    return selectedTags.value.some(selectedTag => {
      return note.tags.some(noteTag => 
        noteTag === selectedTag || noteTag.startsWith(selectedTag + '/')
      )
    })
  })
})

function handleTagFilter(tags) {
  selectedTags.value = tags
}

// Function to handle creating a link from unlinked mentions
async function handleCreateLink({ sourceNoteId, targetTitle, context }) {
  try {
    // 1. Get the source note content
    const sourceNote = await notesApi.get(sourceNoteId)
    if (!sourceNote) return
    
    // 2. Replace the mention with a wikilink
    // We replace the first occurrence of the title (case-insensitive)
    const titleRegex = new RegExp(targetTitle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'i')
    const newContent = sourceNote.content.replace(titleRegex, `[[${targetTitle}]]`)
    
    // 3. Update the note
    await notesApi.update(sourceNoteId, {
      content: newContent
    })
    
    // 4. Refresh everything
    if (selectedNoteId.value === sourceNoteId) {
      await loadSelectedNote()
    }
    // Also refresh the collection to update link counts etc
    await loadNotes()
    
    // 5. Notify user (could be toast)
    console.log(`Linked "${targetTitle}" in "${sourceNote.title}"`)
    
  } catch (error) {
    console.error('Failed to create link:', error)
  }
}

// Function to toggle theme
function handleThemeToggle() {
  themeStore.toggleTheme()
}

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

function handleGraphNodeClick(noteId) {
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

function handleDirty() {
  isDirty.value = true
}

async function handleSaveNote(id, data) {
  try {
    const saveData = {
      ...data,
      title: selectedNote.value.title
    }
    if (id) {
      await notesApi.update(id, saveData)
    } else {
      const created = await notesApi.create(saveData)
      selectedNoteId.value = created.id
    }
    await loadNotes()
    await loadSelectedNote()
    isDirty.value = false
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

function handleCloseNote() {
  selectedNoteId.value = null
  selectedNote.value = null
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

/* Full Graph View (Always Visible) */
.full-graph-container {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: var(--spacing-md);
  height: 100%;
}

.graph-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--spacing-md);
}

.graph-header h2 {
  margin: 0;
  font-size: 1.25rem;
  color: var(--text-primary);
}

/* Editor Panel Overlay */
.editor-panel-overlay {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 50%;
  max-width: 800px;
  background: var(--bg-primary);
  border-left: 1px solid var(--bg-tertiary);
  box-shadow: -4px 0 16px rgba(0, 0, 0, 0.3);
  display: flex;
  flex-direction: column;
  z-index: 100;
  animation: slideIn 0.3s ease-out;
}

@keyframes slideIn {
  from {
    transform: translateX(100%);
  }
  to {
    transform: translateX(0);
  }
}

.editor-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.editor-panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
}

.editor-panel-header .title-input {
  flex: 1;
  font-size: 1.25rem;
  font-weight: 600;
  background: transparent;
  border: none;
  color: var(--text-primary);
  padding: 0;
  margin-right: var(--spacing-md);
}

.editor-panel-header .title-input:focus {
  outline: none;
}

.editor-panel-header .title-input::placeholder {
  color: var(--text-muted);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0 8px;
  line-height: 1;
  transition: color var(--transition-fast);
}

.close-btn:hover {
  color: var(--text-primary);
}

/* Responsive Design */
@media (max-width: 1024px) {
  .editor-panel-overlay {
    width: 70%;
  }
}

@media (max-width: 768px) {
  .editor-panel-overlay {
    width: 100%;
    max-width: none;
  }
}
</style>
