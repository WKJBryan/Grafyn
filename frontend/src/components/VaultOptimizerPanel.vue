<template>
  <div class="optimizer-panel">
    <div class="panel-header">
      <div>
        <div class="section-title">Vault Optimizer</div>
        <div class="panel-subtitle">
          {{ status?.enabled ? `${status.edit_mode.replace('_', ' ')} mode` : 'Paused' }}
          <span v-if="status"> · {{ status.llm_enabled ? 'LLM enabled' : 'Local only' }}</span>
        </div>
      </div>
      <button
        class="refresh-btn"
        @click="load"
      >
        ↻
      </button>
    </div>

    <div
      v-if="status"
      class="metrics-grid"
    >
      <div class="metric">
        <span class="metric-value">{{ status.queue_size }}</span>
        <span class="metric-label">Queued</span>
      </div>
      <div class="metric">
        <span class="metric-value">{{ status.inbox_count }}</span>
        <span class="metric-label">Inbox</span>
      </div>
      <div class="metric">
        <span class="metric-value">{{ status.accepted_count }}</span>
        <span class="metric-label">Applied</span>
      </div>
      <div class="metric">
        <span class="metric-value">{{ status.rollback_count }}</span>
        <span class="metric-label">Rollbacks</span>
      </div>
    </div>

    <div
      v-if="recentInbox.length"
      class="optimizer-list"
    >
      <div
        v-for="entry in recentInbox"
        :key="entry.id"
        class="optimizer-item"
      >
        <div class="item-copy">
          <strong>{{ entry.title || entry.note_id }}</strong>
          <div class="item-reason">{{ entry.reason }}</div>
          <div class="item-diff">{{ entry.diff_preview }}</div>
        </div>
        <button
          v-if="entry.change_id"
          class="rollback-btn"
          @click="rollback(entry.change_id)"
        >
          Rollback
        </button>
      </div>
    </div>

    <div
      v-else
      class="empty-state"
    >
      No optimizer activity yet.
    </div>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { optimizer as optimizerApi } from '@/api/client'
import { useToast } from '@/composables/useToast'

const toast = useToast()
const status = ref(null)
const recentInbox = ref([])

async function load() {
  if (typeof window === 'undefined' || typeof window.__TAURI_IPC__ !== 'function') {
    return
  }
  try {
    status.value = await optimizerApi.status()
    recentInbox.value = await optimizerApi.inbox(null, 5)
  } catch (error) {
    console.error('Failed to load optimizer panel', error)
  }
}

async function rollback(changeId) {
  try {
    await optimizerApi.rollbackChange(changeId)
    toast.success('Optimizer change rolled back.')
    await load()
  } catch (error) {
    console.error('Failed to rollback optimizer change', error)
    toast.error(`Rollback failed: ${error.message}`)
  }
}

onMounted(load)
</script>

<style scoped>
.optimizer-panel {
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  align-items: flex-start;
}

.panel-subtitle {
  color: var(--text-secondary);
  font-size: 0.85rem;
}

.refresh-btn,
.rollback-btn {
  border: 1px solid var(--border-color);
  background: var(--bg-secondary);
  color: var(--text-primary);
  border-radius: var(--radius-sm);
  padding: 0.35rem 0.65rem;
  cursor: pointer;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.6rem;
}

.metric {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 0.7rem;
}

.metric-value {
  display: block;
  font-size: 1.15rem;
  font-weight: 700;
}

.metric-label {
  color: var(--text-secondary);
  font-size: 0.8rem;
}

.optimizer-list {
  display: grid;
  gap: 0.65rem;
}

.optimizer-item {
  display: flex;
  gap: 0.75rem;
  justify-content: space-between;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 0.8rem;
}

.item-copy {
  min-width: 0;
}

.item-reason,
.item-diff,
.empty-state {
  color: var(--text-secondary);
  font-size: 0.85rem;
}
</style>
