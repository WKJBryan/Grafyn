<template>
  <div class="import-view">
    <div class="import-header">
      <h1>Import Conversations</h1>
      <p class="subtitle">
        Import conversations from ChatGPT, Claude, Grok, or Gemini as evidence notes
      </p>
    </div>

    <!-- Step 1: File Selection -->
    <div
      v-if="!preview"
      class="import-step"
    >
      <button
        class="btn btn-primary file-btn"
        data-guide="import-file-btn"
        :disabled="loading"
        @click="handlePickFile"
      >
        {{ loading ? 'Reading file...' : 'Choose Export File' }}
      </button>
      <p class="help-text">
        Supported formats: ChatGPT conversations.json, Claude .dms/JSON, Grok export, Gemini export
      </p>
      <p
        v-if="error"
        class="error-text"
      >
        {{ error }}
      </p>
    </div>

    <!-- Step 2: Preview & Select -->
    <div
      v-if="preview"
      class="import-step"
    >
      <div class="preview-header">
        <div class="preview-info">
          <span class="platform-badge">{{ preview.platform }}</span>
          <span>{{ preview.total_conversations }} conversation{{ preview.total_conversations === 1 ? '' : 's' }} found</span>
        </div>
        <div class="preview-actions">
          <button
            class="btn btn-secondary"
            @click="resetImport"
          >
            Choose Different File
          </button>
          <button
            class="btn btn-primary"
            :disabled="selectedIds.length === 0 || importing"
            @click="handleImport"
          >
            {{ importing ? 'Importing...' : `Import ${selectedIds.length} Selected` }}
          </button>
        </div>
      </div>

      <div class="select-controls">
        <button
          class="btn btn-ghost btn-sm"
          @click="selectAll"
        >
          Select All
        </button>
        <button
          class="btn btn-ghost btn-sm"
          @click="selectNone"
        >
          Select None
        </button>
      </div>

      <div class="conversation-list">
        <label
          v-for="conv in preview.conversations"
          :key="conv.id"
          class="conversation-item"
        >
          <input
            v-model="selectedIds"
            type="checkbox"
            :value="conv.id"
          >
          <div class="conv-details">
            <div class="conv-title">
              {{ conv.title }}
            </div>
            <div class="conv-meta">
              {{ conv.messages.length }} messages
              <span v-if="conv.metadata.model_info?.length">
                &middot; {{ conv.metadata.model_info.join(', ') }}
              </span>
              <span v-if="conv.metadata.created_at">
                &middot; {{ formatDate(conv.metadata.created_at) }}
              </span>
            </div>
            <div class="conv-tags">
              <span
                v-for="tag in conv.suggested_tags"
                :key="tag"
                class="tag"
              >{{ tag }}</span>
            </div>
          </div>
        </label>
      </div>
    </div>

    <!-- Step 3: Results -->
    <div
      v-if="result"
      class="import-step result-step"
    >
      <div class="result-message success">
        {{ result.message }}
      </div>
      <div
        v-if="result.errors?.length"
        class="result-errors"
      >
        <p
          v-for="(err, i) in result.errors"
          :key="i"
          class="error-text"
        >
          {{ err }}
        </p>
      </div>
      <div class="result-actions">
        <button
          v-if="!discoveryStarted && result.note_ids?.length"
          class="btn btn-primary"
          @click="startLinkDiscovery(result.note_ids)"
        >
          Discover Links
        </button>
        <button
          class="btn btn-secondary"
          @click="resetImport"
        >
          Import More
        </button>
        <router-link
          to="/"
          class="btn btn-secondary"
        >
          Go to Notes
        </router-link>
      </div>

      <!-- Link Discovery Section -->
      <div
        v-if="discoveryState.size > 0"
        class="discovery-section"
      >
        <div class="discovery-header">
          <div class="discovery-title-row">
            <h3>Link Discovery</h3>
            <span class="discovery-counter">
              Found {{ totalCandidatesFound }} candidate{{ totalCandidatesFound !== 1 ? 's' : '' }}
              &middot; {{ discoveryDoneCount }}/{{ discoveryState.size }} notes scanned
            </span>
          </div>
          <button
            v-if="totalSelectedCount > 0 && !batchApplying"
            class="btn btn-primary btn-sm"
            @click="applyAllSelected"
          >
            Apply All Selected ({{ totalSelectedCount }})
          </button>
          <span
            v-if="batchApplying"
            class="batch-applying"
          >
            <span class="loading-spinner loading-spinner-sm" />
            Applying...
          </span>
        </div>

        <!-- Note Status Chip Bar -->
        <div class="note-status-bar">
          <div
            v-for="[noteId, entry] in discoveryState"
            :key="noteId"
            class="note-chip"
            :class="{
              pending: entry.status === 'pending',
              loading: entry.status === 'loading',
              done: entry.status === 'done' && entry.candidates.length > 0 && !entry.applied,
              empty: entry.status === 'done' && entry.candidates.length === 0,
              error: entry.status === 'error',
              applied: entry.applied,
            }"
            :title="entry.noteTitle || noteId"
            @click="entry.status === 'error' ? retryNote(noteId) : null"
          >
            <span class="chip-icon">
              <span
                v-if="entry.status === 'loading'"
                class="loading-spinner loading-spinner-chip"
              />
              <template v-else-if="entry.applied">&#10003;&#10003;</template>
              <template v-else-if="entry.status === 'done' && entry.candidates.length > 0">&#10003;</template>
              <template v-else-if="entry.status === 'done' && entry.candidates.length === 0">&ndash;</template>
              <template v-else-if="entry.status === 'error'">&#10005;</template>
              <template v-else>&#9675;</template>
            </span>
            <span class="chip-title">{{ entry.noteTitle || noteId }}</span>
            <span
              v-if="entry.status === 'done' && entry.candidates.length > 0 && !entry.applied"
              class="chip-count"
            >
              {{ entry.candidates.length }}
            </span>
          </div>
        </div>

        <!-- Candidate Feed -->
        <TransitionGroup
          v-if="flatCandidates.length > 0"
          name="feed"
          tag="div"
          class="candidate-feed"
        >
          <div
            v-for="item in flatCandidates"
            :key="`${item.noteId}-${item.target_id}`"
            class="candidate-row"
            :class="{ applied: item.applied, selected: !item.applied && isSelected(item.noteId, item.target_id) }"
          >
            <label class="candidate-check">
              <input
                v-if="!item.applied"
                type="checkbox"
                :checked="isSelected(item.noteId, item.target_id)"
                @change="toggleCandidate(item.noteId, item.target_id)"
              >
              <span
                v-else
                class="applied-check"
              >&#10003;</span>
            </label>
            <div class="candidate-info">
              <span class="source-label">from {{ item.noteTitle }}</span>
              <div class="candidate-header">
                <span class="candidate-title">{{ item.target_title }}</span>
                <span
                  class="confidence-badge"
                  :class="confidenceClass(item.confidence)"
                >
                  {{ Math.round(item.confidence * 100) }}%
                </span>
              </div>
              <div
                v-if="item.reason"
                class="candidate-reason"
              >
                {{ item.reason }}
              </div>
              <span
                class="link-type-badge"
                :class="'type-' + item.link_type"
              >
                {{ item.link_type }}
              </span>
            </div>
          </div>
        </TransitionGroup>

        <!-- Empty state -->
        <div
          v-if="discoveryDoneCount === discoveryState.size && flatCandidates.length === 0 && erroredNotes.length === 0"
          class="feed-empty"
        >
          No link candidates found across {{ discoveryState.size }} note{{ discoveryState.size !== 1 ? 's' : '' }}.
        </div>

        <!-- Errors section -->
        <div
          v-if="erroredNotes.length > 0"
          class="errors-section"
        >
          <div
            v-for="[noteId, entry] in erroredNotes"
            :key="noteId"
            class="error-row"
          >
            <span class="error-row-title">{{ entry.noteTitle || noteId }}</span>
            <span class="error-row-message">{{ entry.error }}</span>
            <button
              class="btn btn-secondary btn-sm"
              @click="retryNote(noteId)"
            >
              Retry
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed } from 'vue'
import { open } from '@tauri-apps/api/dialog'
import { importApi, zettelkasten, notes } from '@/api/client'
import { useToast } from '@/composables/useToast'

const toast = useToast()

const loading = ref(false)
const importing = ref(false)
const error = ref(null)
const preview = ref(null)
const selectedIds = ref([])
const result = ref(null)
const filePath = ref(null)

// Link discovery state
const discoveryState = reactive(new Map())
const discoveryStarted = ref(false)
const batchApplying = ref(false)

const discoveryDoneCount = computed(() => {
  let count = 0
  for (const entry of discoveryState.values()) {
    if (entry.status === 'done' || entry.status === 'error') count++
  }
  return count
})

const totalSelectedCount = computed(() => {
  let count = 0
  for (const entry of discoveryState.values()) {
    if (entry.status === 'done' && !entry.applied) count += entry.selected.size
  }
  return count
})

const flatCandidates = computed(() => {
  const items = []
  for (const [noteId, entry] of discoveryState) {
    for (const c of entry.candidates) {
      items.push({
        ...c,
        noteId,
        noteTitle: entry.noteTitle,
        applied: entry.applied,
      })
    }
  }
  return items
})

const totalCandidatesFound = computed(() => flatCandidates.value.length)

const erroredNotes = computed(() => {
  const errors = []
  for (const [noteId, entry] of discoveryState) {
    if (entry.status === 'error') errors.push([noteId, entry])
  }
  return errors
})

function isSelected(noteId, targetId) {
  const entry = discoveryState.get(noteId)
  return entry ? entry.selected.has(targetId) : false
}

async function handlePickFile() {
  error.value = null

  const selected = await open({
    multiple: false,
    filters: [{ name: 'JSON', extensions: ['json', 'dms'] }],
  })

  if (!selected) return

  loading.value = true
  filePath.value = selected

  try {
    preview.value = await importApi.preview(selected)
    // Auto-select all conversations
    selectedIds.value = preview.value.conversations.map(c => c.id)
  } catch (e) {
    error.value = e.message || e.toString() || 'Failed to parse file'
  } finally {
    loading.value = false
  }
}

async function handleImport() {
  if (!filePath.value || selectedIds.value.length === 0) return

  importing.value = true
  error.value = null

  try {
    result.value = await importApi.apply(filePath.value, selectedIds.value)
    preview.value = null
  } catch (e) {
    error.value = e.message || e.toString() || 'Import failed'
  } finally {
    importing.value = false
  }
}

function selectAll() {
  if (preview.value) {
    selectedIds.value = preview.value.conversations.map(c => c.id)
  }
}

function selectNone() {
  selectedIds.value = []
}

// --- Link Discovery ---

function startLinkDiscovery(noteIds) {
  discoveryStarted.value = true
  for (const id of noteIds) {
    discoveryState.set(id, {
      status: 'pending',
      noteTitle: '',
      candidates: [],
      selected: new Set(),
      error: null,
      applied: false,
      linksCreated: 0,
    })
  }

  // Concurrency-limited processing (max 3 in parallel)
  const queue = [...noteIds]
  let active = 0
  const MAX_CONCURRENT = 3

  function processNext() {
    while (active < MAX_CONCURRENT && queue.length > 0) {
      const noteId = queue.shift()
      active++
      discoverForNote(noteId).finally(() => {
        active--
        processNext()
      })
    }
  }
  processNext()
}

async function discoverForNote(noteId) {
  const entry = discoveryState.get(noteId)
  if (!entry) return
  entry.status = 'loading'

  try {
    const note = await notes.get(noteId)
    entry.noteTitle = note.title || noteId

    const candidates = await zettelkasten.discoverLinks(noteId, 'algorithm', 10)
    entry.candidates = candidates
    entry.selected = new Set(
      candidates.filter(c => c.confidence >= 0.7).map(c => c.target_id)
    )
    entry.status = 'done'
  } catch (e) {
    entry.status = 'error'
    entry.error = e.message || e.toString() || 'Discovery failed'
  }
}

function retryNote(noteId) {
  const entry = discoveryState.get(noteId)
  if (!entry) return
  entry.status = 'pending'
  entry.error = null
  entry.candidates = []
  entry.selected = new Set()
  discoverForNote(noteId)
}

function toggleCandidate(noteId, targetId) {
  const entry = discoveryState.get(noteId)
  if (!entry) return
  const next = new Set(entry.selected)
  if (next.has(targetId)) {
    next.delete(targetId)
  } else {
    next.add(targetId)
  }
  entry.selected = next
}

function confidenceClass(confidence) {
  if (confidence >= 0.8) return 'confidence-high'
  if (confidence >= 0.5) return 'confidence-medium'
  return 'confidence-low'
}

async function applyAllSelected() {
  batchApplying.value = true
  let totalLinks = 0
  let noteCount = 0

  for (const [noteId, entry] of discoveryState) {
    if (entry.status !== 'done' || entry.applied || entry.selected.size === 0) continue
    try {
      const selectedCandidates = entry.candidates.filter(c => entry.selected.has(c.target_id))
      const applyResult = await zettelkasten.applyLinks(noteId, selectedCandidates)
      entry.applied = true
      entry.linksCreated = applyResult.links_created
      totalLinks += applyResult.links_created
      noteCount++
    } catch (e) {
      toast.error(`Failed to apply links for "${entry.noteTitle}"`)
    }
  }

  batchApplying.value = false
  if (totalLinks > 0) {
    toast.success(`Created ${totalLinks} link${totalLinks !== 1 ? 's' : ''} across ${noteCount} note${noteCount !== 1 ? 's' : ''}`)
  }
}

function resetImport() {
  preview.value = null
  selectedIds.value = []
  result.value = null
  error.value = null
  filePath.value = null
  discoveryState.clear()
  discoveryStarted.value = false
  batchApplying.value = false
}

function formatDate(dateStr) {
  try {
    return new Date(dateStr).toLocaleDateString()
  } catch {
    return dateStr
  }
}
</script>

<style scoped>
.import-view {
  max-width: 800px;
  margin: 0 auto;
  padding: var(--spacing-xl);
}

.import-header {
  margin-bottom: var(--spacing-xl);
}

.import-header h1 {
  font-size: 1.75rem;
  font-weight: 700;
  color: var(--text-primary);
  margin-bottom: var(--spacing-xs);
}

.subtitle {
  color: var(--text-secondary);
  font-size: 0.95rem;
}

.import-step {
  margin-bottom: var(--spacing-xl);
}

.file-btn {
  font-size: 1rem;
  padding: var(--spacing-md) var(--spacing-xl);
}

.help-text {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin-top: var(--spacing-sm);
}

.error-text {
  color: var(--error-bg, #dc2626);
  font-size: 0.85rem;
  margin-top: var(--spacing-sm);
}

.preview-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--spacing-md);
  flex-wrap: wrap;
  gap: var(--spacing-sm);
}

.preview-info {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  color: var(--text-secondary);
}

.platform-badge {
  background: var(--accent-primary);
  color: white;
  padding: 2px 10px;
  border-radius: var(--radius-sm);
  font-size: 0.8rem;
  font-weight: 600;
  text-transform: uppercase;
}

.preview-actions {
  display: flex;
  gap: var(--spacing-sm);
}

.select-controls {
  display: flex;
  gap: var(--spacing-xs);
  margin-bottom: var(--spacing-md);
}

.conversation-list {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  max-height: 500px;
  overflow-y: auto;
}

.conversation-item {
  display: flex;
  align-items: flex-start;
  gap: var(--spacing-md);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--transition-fast);
}

.conversation-item:hover {
  background: var(--bg-hover);
}

.conversation-item input[type="checkbox"] {
  margin-top: 4px;
  flex-shrink: 0;
}

.conv-details {
  flex: 1;
  min-width: 0;
}

.conv-title {
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.conv-meta {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin-bottom: 4px;
}

.conv-tags {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}

.tag {
  font-size: 0.7rem;
  padding: 1px 6px;
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
}

.result-step {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--spacing-md);
}

.result-message.success {
  padding: var(--spacing-md);
  background: var(--accent-primary);
  color: white;
  border-radius: var(--radius-md);
  font-weight: 600;
  width: 100%;
}

.result-errors {
  width: 100%;
}

.result-actions {
  display: flex;
  gap: var(--spacing-sm);
  flex-wrap: wrap;
}

/* Discovery Section */
.discovery-section {
  width: 100%;
  border-top: 1px solid var(--bg-tertiary);
  margin-top: var(--spacing-md);
  padding-top: var(--spacing-lg);
}

.discovery-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--spacing-md);
  flex-wrap: wrap;
  gap: var(--spacing-sm);
}

.discovery-title-row {
  display: flex;
  align-items: baseline;
  gap: var(--spacing-md);
}

.discovery-title-row h3 {
  margin: 0;
  font-size: 1.1rem;
  color: var(--text-primary);
}

.discovery-counter {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.batch-applying {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  font-size: 0.85rem;
  color: var(--text-secondary);
}

/* Note Status Chip Bar */
.note-status-bar {
  display: flex;
  gap: 6px;
  padding: var(--spacing-sm) 0;
  margin-bottom: var(--spacing-md);
  overflow-x: auto;
  scrollbar-width: thin;
}

.note-chip {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  border-radius: 999px;
  font-size: 0.75rem;
  white-space: nowrap;
  flex-shrink: 0;
  border: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  transition: all var(--transition-fast);
}

.note-chip.loading {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
}

.note-chip.done {
  border-color: rgba(99, 102, 241, 0.3);
  color: var(--accent-primary);
  background: rgba(99, 102, 241, 0.05);
}

.note-chip.empty {
  color: var(--text-muted);
  opacity: 0.7;
}

.note-chip.error {
  border-color: rgba(239, 68, 68, 0.3);
  color: #ef4444;
  background: rgba(239, 68, 68, 0.05);
  cursor: pointer;
}

.note-chip.error:hover {
  background: rgba(239, 68, 68, 0.1);
}

.note-chip.applied {
  border-color: rgba(34, 197, 94, 0.3);
  color: #22c55e;
  background: rgba(34, 197, 94, 0.05);
}

.chip-icon {
  display: flex;
  align-items: center;
  font-size: 0.7rem;
  line-height: 1;
}

.chip-title {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.chip-count {
  font-weight: 600;
  font-size: 0.65rem;
  background: var(--accent-primary);
  color: white;
  padding: 0 5px;
  border-radius: 999px;
  line-height: 1.4;
}

.loading-spinner-chip {
  width: 10px;
  height: 10px;
  border-width: 1.5px;
}

/* Candidate Feed */
.candidate-feed {
  display: flex;
  flex-direction: column;
  max-height: 500px;
  overflow-y: auto;
}

.candidate-row {
  display: flex;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-xs);
  border-radius: var(--radius-md);
  transition: background var(--transition-fast), opacity var(--transition-fast);
}

.candidate-row:hover {
  background: var(--bg-hover);
}

.candidate-row.selected {
  background: rgba(99, 102, 241, 0.05);
}

.candidate-row.applied {
  opacity: 0.6;
  background: rgba(34, 197, 94, 0.03);
}

.applied-check {
  color: #22c55e;
  font-size: 0.85rem;
  font-weight: 600;
  display: inline-block;
  width: 13px;
  text-align: center;
}

.source-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  margin-bottom: 2px;
  display: block;
}

/* Feed transition */
.feed-enter-active {
  transition: all 0.3s ease;
}

.feed-enter-from {
  opacity: 0;
  transform: translateY(-8px);
}

/* Empty & Error states */
.feed-empty {
  color: var(--text-muted);
  font-size: 0.85rem;
  padding: var(--spacing-md) 0;
  text-align: center;
}

.errors-section {
  margin-top: var(--spacing-md);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.error-row {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  background: rgba(239, 68, 68, 0.05);
  border: 1px solid rgba(239, 68, 68, 0.15);
  border-radius: var(--radius-md);
  font-size: 0.82rem;
}

.error-row-title {
  font-weight: 500;
  color: var(--text-primary);
  flex-shrink: 0;
}

.error-row-message {
  color: #ef4444;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.candidate-check {
  padding-top: 2px;
  cursor: pointer;
}

.candidate-check input {
  accent-color: var(--accent-primary);
}

.candidate-info {
  flex: 1;
  min-width: 0;
}

.candidate-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--spacing-sm);
}

.candidate-title {
  font-size: 0.9rem;
  font-weight: 500;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.confidence-badge {
  font-size: 0.7rem;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
}

.confidence-high {
  color: #22c55e;
  background: rgba(34, 197, 94, 0.1);
}

.confidence-medium {
  color: #f59e0b;
  background: rgba(245, 158, 11, 0.1);
}

.confidence-low {
  color: var(--text-muted);
  background: var(--bg-tertiary);
}

.candidate-reason {
  font-size: 0.78rem;
  color: var(--text-secondary);
  margin: 2px 0 4px;
  line-height: 1.4;
}

.link-type-badge {
  display: inline-block;
  font-size: 0.65rem;
  text-transform: uppercase;
  font-weight: 600;
  letter-spacing: 0.03em;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  background: var(--bg-tertiary);
}

.type-supports { color: #22c55e; background: rgba(34, 197, 94, 0.1); }
.type-contradicts { color: #ef4444; background: rgba(239, 68, 68, 0.1); }
.type-expands { color: #6366f1; background: rgba(99, 102, 241, 0.1); }
.type-related { color: #3b82f6; background: rgba(59, 130, 246, 0.1); }
.type-questions { color: #f59e0b; background: rgba(245, 158, 11, 0.1); }
.type-answers { color: #22c55e; background: rgba(34, 197, 94, 0.1); }
.type-example { color: #8b5cf6; background: rgba(139, 92, 246, 0.1); }
.type-part_of { color: #06b6d4; background: rgba(6, 182, 212, 0.1); }

/* Spinner */
.loading-spinner {
  width: 24px;
  height: 24px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.loading-spinner-sm {
  width: 14px;
  height: 14px;
  border-width: 2px;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.btn-sm {
  font-size: 0.8rem;
  padding: 2px 8px;
}
</style>
