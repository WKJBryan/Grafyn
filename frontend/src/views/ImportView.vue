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
      <button
        class="btn btn-primary"
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
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { open } from '@tauri-apps/api/dialog'
import { importApi } from '@/api/client'

const loading = ref(false)
const importing = ref(false)
const error = ref(null)
const preview = ref(null)
const selectedIds = ref([])
const result = ref(null)
const filePath = ref(null)

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

function resetImport() {
  preview.value = null
  selectedIds.value = []
  result.value = null
  error.value = null
  filePath.value = null
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

.btn-sm {
  font-size: 0.8rem;
  padding: 2px 8px;
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
</style>
