<template>
  <div class="pinned-notes-wrapper">
    <button
      class="btn btn-secondary btn-sm"
      :class="{ active: showPanel }"
      :disabled="!hasSession"
      title="Pin notes for context"
      @click="showPanel = !showPanel"
    >
      <span class="icon">&#128204;</span>
      Notes
      <span
        v-if="pinnedCount > 0"
        class="pin-count"
      >{{ pinnedCount }}</span>
    </button>

    <div
      v-if="showPanel"
      class="pinned-panel"
    >
      <div class="panel-header">
        <h4>Pinned Notes</h4>
        <button
          class="close-btn"
          @click="showPanel = false"
        >
          &#10005;
        </button>
      </div>

      <!-- Search Input -->
      <div class="search-section">
        <input
          v-model="searchQuery"
          type="text"
          placeholder="Search notes to pin..."
          class="search-input"
          @input="handleSearch"
        >
      </div>

      <!-- Search Results -->
      <div
        v-if="searchResults.length > 0"
        class="search-results"
      >
        <div
          v-for="note in searchResults"
          :key="note.id"
          class="search-result-item"
        >
          <span class="result-title">{{ note.title }}</span>
          <button
            v-if="!isPinned(note.id)"
            class="pin-btn"
            @click="pinNote(note)"
          >
            Pin
          </button>
          <span
            v-else
            class="already-pinned"
          >Pinned</span>
        </div>
      </div>

      <!-- Currently Pinned Notes -->
      <div
        v-if="pinnedNotes.length > 0"
        class="pinned-list"
      >
        <div class="pinned-list-header">
          Pinned ({{ pinnedNotes.length }})
        </div>
        <div
          v-for="note in pinnedNotes"
          :key="note.id"
          class="pinned-item"
        >
          <span class="pinned-title">{{ note.title }}</span>
          <button
            class="unpin-btn"
            title="Unpin"
            @click="unpinNote(note.id)"
          >
            &#10005;
          </button>
        </div>
      </div>

      <div
        v-else-if="searchResults.length === 0"
        class="empty-state"
      >
        Pin notes to always include them as context in Knowledge Search mode.
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { search as searchApi } from '@/api/client'
import { useCanvasStore } from '@/stores/canvas'

const canvasStore = useCanvasStore()

const showPanel = ref(false)
const searchQuery = ref('')
const searchResults = ref([])
const pinnedNotes = ref([])
let searchTimer = null

const hasSession = computed(() => canvasStore.hasSession)

const pinnedCount = computed(() => {
  return canvasStore.currentSession?.pinned_note_ids?.length || 0
})

function isPinned(noteId) {
  return canvasStore.currentSession?.pinned_note_ids?.includes(noteId) || false
}

// Sync pinned notes display when session changes
watch(() => canvasStore.currentSession?.pinned_note_ids, async (ids) => {
  if (!ids || ids.length === 0) {
    pinnedNotes.value = []
    return
  }
  // We only need titles for display — search for each pinned ID
  // For efficiency, just store id+title pairs from when they were pinned
  // Keep existing ones, remove unpinned
  pinnedNotes.value = pinnedNotes.value.filter(n => ids.includes(n.id))
}, { immediate: true })

function handleSearch() {
  clearTimeout(searchTimer)
  if (!searchQuery.value.trim()) {
    searchResults.value = []
    return
  }
  searchTimer = setTimeout(async () => {
    try {
      const results = await searchApi.query(searchQuery.value, { limit: 8 })
      searchResults.value = results.map(r => ({
        id: r.id,
        title: r.title
      }))
    } catch (_err) {
      searchResults.value = []
    }
  }, 250)
}

async function pinNote(note) {
  const currentIds = canvasStore.currentSession?.pinned_note_ids || []
  if (currentIds.includes(note.id)) return
  const newIds = [...currentIds, note.id]
  pinnedNotes.value.push({ id: note.id, title: note.title })
  await canvasStore.updatePinnedNotes(newIds)
}

async function unpinNote(noteId) {
  const currentIds = canvasStore.currentSession?.pinned_note_ids || []
  const newIds = currentIds.filter(id => id !== noteId)
  pinnedNotes.value = pinnedNotes.value.filter(n => n.id !== noteId)
  await canvasStore.updatePinnedNotes(newIds)
}
</script>

<style scoped>
.pinned-notes-wrapper {
  position: relative;
}

.btn.active {
  background: var(--accent-primary);
  color: white;
  border-color: var(--accent-primary);
}

.pin-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border-radius: 9px;
  background: rgba(255, 255, 255, 0.2);
  font-size: 0.6875rem;
  font-weight: 600;
}

.pinned-panel {
  position: absolute;
  top: 100%;
  right: 0;
  margin-top: 4px;
  width: 280px;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
  z-index: 100;
  max-height: 400px;
  display: flex;
  flex-direction: column;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-sm) var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
}

.panel-header h4 {
  margin: 0;
  font-size: 0.875rem;
  color: var(--text-primary);
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
}

.close-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.search-section {
  padding: var(--spacing-sm);
}

.search-input {
  width: 100%;
  padding: 6px 8px;
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8125rem;
}

.search-input:focus {
  outline: none;
  border-color: var(--accent-primary);
}

.search-results {
  max-height: 160px;
  overflow-y: auto;
  border-bottom: 1px solid var(--bg-tertiary);
}

.search-result-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px var(--spacing-sm);
  gap: var(--spacing-sm);
}

.search-result-item:hover {
  background: var(--bg-hover);
}

.result-title {
  font-size: 0.8125rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
}

.pin-btn {
  padding: 2px 8px;
  border: 1px solid var(--accent-primary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--accent-primary);
  font-size: 0.6875rem;
  cursor: pointer;
  flex-shrink: 0;
}

.pin-btn:hover {
  background: var(--accent-primary);
  color: white;
}

.already-pinned {
  font-size: 0.6875rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.pinned-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm);
}

.pinned-list-header {
  font-size: 0.6875rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
  margin-bottom: var(--spacing-xs);
  letter-spacing: 0.05em;
}

.pinned-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  gap: var(--spacing-sm);
}

.pinned-item:hover {
  background: var(--bg-hover);
}

.pinned-title {
  font-size: 0.8125rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
}

.unpin-btn {
  width: 20px;
  height: 20px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  font-size: 0.6875rem;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.unpin-btn:hover {
  background: rgba(248, 113, 113, 0.1);
  color: var(--accent-red, #f87171);
}

.empty-state {
  padding: var(--spacing-md);
  font-size: 0.8125rem;
  color: var(--text-muted);
  text-align: center;
  line-height: 1.5;
}
</style>
