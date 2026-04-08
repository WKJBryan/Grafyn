<template>
  <div class="inbox-panel">
    <div class="inbox-header">
      <div>
        <h3>Link Inbox</h3>
        <p>Background suggestions waiting for review.</p>
      </div>
      <button
        class="refresh-btn"
        :disabled="loading"
        @click="loadInbox"
      >
        {{ loading ? 'Refreshing...' : 'Refresh' }}
      </button>
    </div>

    <div
      v-if="status"
      class="status-row"
    >
      <span class="status-pill">
        Queue {{ status.queue_size }}
      </span>
      <span class="status-pill">
        Pending {{ status.pending_suggestions }}
      </span>
      <span
        v-if="status.is_running"
        class="status-pill running"
      >
        Running {{ status.current_note_title || status.current_note_id }}
      </span>
    </div>

    <div
      v-if="error"
      class="inbox-empty"
    >
      {{ error }}
    </div>

    <div
      v-else-if="entries.length === 0"
      class="inbox-empty"
    >
      No pending link suggestions right now.
    </div>

    <div
      v-else
      class="inbox-list"
    >
      <div
        v-for="entry in entries"
        :key="entry.note_id"
        class="inbox-entry"
      >
        <div class="entry-header">
          <button
            class="entry-note"
            @click="$emit('navigate', entry.note_id)"
          >
            {{ entry.note_title }}
          </button>
          <span class="entry-meta">
            {{ entry.pending_count }} suggestion{{ entry.pending_count !== 1 ? 's' : '' }}
          </span>
        </div>

        <div
          v-if="entry.links?.length"
          class="entry-section"
        >
          <div class="entry-section-title">Strong Matches</div>
          <div
            v-for="candidate in entry.links"
            :key="entry.note_id + '-strong-' + candidate.target_id"
            class="entry-candidate"
          >
            <div class="candidate-copy">
              <div class="candidate-title">{{ candidate.target_title }}</div>
              <div class="candidate-reason">{{ candidate.reason }}</div>
            </div>
            <div class="candidate-controls">
              <span class="candidate-confidence">{{ Math.round(candidate.confidence * 100) }}%</span>
              <button
                class="mini-btn"
                @click="applySingle(entry.note_id, candidate)"
              >
                Apply
              </button>
              <button
                class="mini-btn ghost"
                @click="dismiss(entry.note_id, candidate.target_id)"
              >
                Dismiss
              </button>
            </div>
          </div>
        </div>

        <div
          v-if="entry.exploratory_links?.length"
          class="entry-section"
        >
          <div class="entry-section-title">Exploratory</div>
          <div
            v-for="candidate in entry.exploratory_links"
            :key="entry.note_id + '-exploratory-' + candidate.target_id"
            class="entry-candidate"
          >
            <div class="candidate-copy">
              <div class="candidate-title">{{ candidate.target_title }}</div>
              <div class="candidate-reason">{{ candidate.reason }}</div>
            </div>
            <div class="candidate-controls">
              <span class="candidate-confidence">{{ Math.round(candidate.confidence * 100) }}%</span>
              <button
                class="mini-btn"
                @click="applySingle(entry.note_id, candidate)"
              >
                Apply
              </button>
              <button
                class="mini-btn ghost"
                @click="dismiss(entry.note_id, candidate.target_id)"
              >
                Dismiss
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { zettelkasten } from '@/api/client'
import { useToast } from '@/composables/useToast'

const emit = defineEmits(['navigate'])
const toast = useToast()

const entries = ref([])
const status = ref(null)
const loading = ref(false)
const error = ref('')

let refreshTimer = null

async function loadInbox() {
  loading.value = true
  error.value = ''

  try {
    const [queue, discoveryStatus] = await Promise.all([
      zettelkasten.listSuggestionQueue('pending', 20),
      zettelkasten.getDiscoveryStatus(),
    ])
    entries.value = Array.isArray(queue) ? queue : []
    status.value = discoveryStatus || null
  } catch (e) {
    console.error('Failed to load link inbox:', e)
    error.value = 'Unable to load link suggestions right now.'
  } finally {
    loading.value = false
  }
}

async function applySingle(noteId, candidate) {
  try {
    const result = await zettelkasten.applyLinks(noteId, [candidate])
    toast.success(`Created ${result.links_created} link${result.links_created !== 1 ? 's' : ''}`)
    await loadInbox()
  } catch (e) {
    console.error('Failed to apply inbox suggestion:', e)
    toast.error('Failed to apply link')
  }
}

async function dismiss(noteId, targetId) {
  try {
    await zettelkasten.dismissSuggestion(noteId, targetId)
    await loadInbox()
  } catch (e) {
    console.error('Failed to dismiss inbox suggestion:', e)
    toast.error('Failed to dismiss suggestion')
  }
}

onMounted(() => {
  loadInbox()
  refreshTimer = window.setInterval(loadInbox, 15000)
})

onUnmounted(() => {
  window.clearInterval(refreshTimer)
})
</script>

<style scoped>
.inbox-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.inbox-header {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  align-items: flex-start;
}

.inbox-header h3 {
  margin: 0;
  font-size: 1rem;
}

.inbox-header p {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 0.82rem;
}

.refresh-btn,
.mini-btn {
  border: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
  color: var(--text-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
}

.refresh-btn {
  padding: 6px 10px;
}

.status-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.status-pill {
  padding: 4px 8px;
  border-radius: 999px;
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.status-pill.running {
  color: var(--accent-primary);
}

.inbox-empty {
  color: var(--text-muted);
  font-size: 0.85rem;
}

.inbox-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.inbox-entry {
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  padding: 12px;
  background: var(--bg-secondary);
}

.entry-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}

.entry-note {
  border: none;
  background: transparent;
  color: var(--text-primary);
  padding: 0;
  cursor: pointer;
  font-weight: 600;
  text-align: left;
}

.entry-meta,
.entry-section-title,
.candidate-reason,
.candidate-confidence {
  color: var(--text-muted);
  font-size: 0.78rem;
}

.entry-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 10px;
}

.candidate-title {
  color: var(--text-primary);
  font-size: 0.88rem;
}

.entry-candidate {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  align-items: center;
}

.candidate-copy {
  min-width: 0;
}

.candidate-controls {
  display: flex;
  align-items: center;
  gap: 8px;
}

.mini-btn {
  padding: 4px 8px;
}

.mini-btn.ghost {
  background: transparent;
  color: var(--text-secondary);
}
</style>
