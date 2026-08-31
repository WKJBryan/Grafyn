<template>
  <section
    class="sync-status-card"
    aria-labelledby="sync-foundation-title"
  >
    <header class="sync-card-header">
      <div>
        <h3
          id="sync-foundation-title"
          data-testid="sync-title"
        >
          Sync foundation
        </h3>
        <p
          class="relay-status"
          data-testid="relay-status"
        >
          Relay not configured
        </p>
      </div>
      <span
        class="status-badge"
        :class="`is-${syncStore.status}`"
      >{{ statusLabel }}</span>
    </header>

    <p class="status-description">
      {{ statusDescription }}
    </p>

    <p
      v-if="syncStore.error || syncStore.actionError"
      class="sync-error"
      role="alert"
    >
      {{ syncStore.actionError || syncStore.error }}
    </p>

    <dl class="sync-metrics">
      <div>
        <dt>Encrypted outbox</dt>
        <dd data-testid="outbox-count">
          {{ syncStore.outboxOperations }}
        </dd>
      </div>
      <div>
        <dt>Pending locally</dt>
        <dd data-testid="pending-count">
          {{ syncStore.pendingOperations }}
        </dd>
      </div>
      <div>
        <dt>Conflicts</dt>
        <dd data-testid="conflict-count">
          {{ syncStore.conflictCount }}
        </dd>
      </div>
    </dl>

    <div class="sync-actions">
      <button
        type="button"
        :disabled="syncStore.loading"
        @click="loadStatus"
      >
        {{ syncStore.loading ? 'Refreshing…' : 'Refresh' }}
      </button>
      <button
        type="button"
        :disabled="syncStore.actionInProgress !== null"
        @click="syncStore.listConflicts"
      >
        Inspect conflicts
      </button>
      <button
        type="button"
        data-testid="export-sync"
        :disabled="syncStore.actionInProgress !== null"
        @click="handleExport"
      >
        Export encrypted bundle
      </button>
      <button
        type="button"
        :disabled="syncStore.actionInProgress !== null"
        @click="handleRebuild"
      >
        Rebuild local state
      </button>
    </div>

    <section
      v-if="syncStore.conflicts.length > 0"
      class="conflict-list"
      aria-label="Sync conflicts"
    >
      <h4>Operation conflicts</h4>
      <article
        v-for="conflict in syncStore.conflicts"
        :key="conflict.selectedOperationId"
        class="conflict-row"
      >
        <span>Selected operation</span>
        <code>{{ conflict.selectedOperationId }}</code>
        <span>Concurrent operations</span>
        <code
          v-for="operationId in conflict.headOperationIds"
          :key="operationId"
        >{{ operationId }}</code>
      </article>
    </section>

    <label
      v-if="exportText"
      class="bundle-field"
    >
      Encrypted outbox bundle
      <textarea
        data-testid="exported-bundle"
        :value="exportText"
        readonly
        rows="5"
      />
    </label>

    <label class="bundle-field">
      Import encrypted bundle
      <textarea
        v-model="importText"
        data-testid="imported-bundle"
        rows="5"
        placeholder="Paste encrypted sync bundle JSON"
      />
    </label>
    <button
      type="button"
      data-testid="import-sync"
      :disabled="!importText.trim() || syncStore.actionInProgress !== null"
      @click="handleImport"
    >
      Import encrypted bundle
    </button>
    <p
      v-if="inputError"
      class="sync-error"
      role="alert"
    >
      {{ inputError }}
    </p>
    <p
      v-if="importMessage"
      class="sync-success"
      role="status"
    >
      {{ importMessage }}
    </p>
  </section>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { useSyncStore } from '@/stores/sync'

const syncStore = useSyncStore()
const exportText = ref('')
const importText = ref('')
const inputError = ref(null)
const importMessage = ref(null)

const STATUS_LABELS = {
  not_provisioned: 'Not provisioned',
  local_only: 'Local only',
  pending: 'Pending',
  conflict: 'Conflict',
  error: 'Unavailable'
}

const STATUS_DESCRIPTIONS = {
  not_provisioned: 'Sync is off until this vault has a shared encryption key.',
  local_only: 'Encrypted operations stay on this device unless you move a bundle manually.',
  pending: 'Encrypted operations are waiting for manual transport or local application.',
  conflict: 'Concurrent encrypted revisions need review. Grafyn has not merged their content.',
  error: 'Local sync state is not available right now.'
}

const statusLabel = computed(() => STATUS_LABELS[syncStore.status])
const statusDescription = computed(() => STATUS_DESCRIPTIONS[syncStore.status])

async function loadStatus() {
  await syncStore.refresh()
  if (syncStore.conflictCount > 0) {
    await syncStore.listConflicts()
  }
}

async function handleExport() {
  const bundle = await syncStore.exportOutbox()
  exportText.value = bundle ? JSON.stringify(bundle, null, 2) : ''
}

async function handleImport() {
  inputError.value = null
  importMessage.value = null

  let bundle
  try {
    bundle = JSON.parse(importText.value)
  } catch (_error) {
    inputError.value = 'Paste a valid encrypted sync bundle.'
    return
  }

  const result = await syncStore.importEnvelopes(bundle)
  if (!result) return
  if (result.recoveryPending) return

  const noun = result.received === 1 ? 'operation' : 'operations'
  importMessage.value = `${result.received} encrypted ${noun} imported`
  if (syncStore.conflictCount > 0) {
    await syncStore.listConflicts()
  }
}

async function handleRebuild() {
  const status = await syncStore.rebuild()
  if (status?.conflicts > 0) {
    await syncStore.listConflicts()
  }
}

onMounted(() => {
  void loadStatus()
})
</script>

<style scoped>
.sync-status-card {
  display: grid;
  gap: 0.9rem;
  padding: 1rem;
  border: 1px solid var(--border-color, #d7d7d7);
  border-radius: 0.75rem;
  background: var(--surface-color, #fff);
}

.sync-card-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
}

.sync-card-header h3,
.sync-card-header p,
.status-description,
.sync-error,
.sync-success,
.conflict-list h4 {
  margin: 0;
}

.relay-status,
.status-description,
.conflict-row span {
  color: var(--text-secondary, #606060);
}

.status-badge {
  padding: 0.25rem 0.55rem;
  border-radius: 999px;
  background: var(--background-secondary, #ededed);
  font-size: 0.78rem;
  font-weight: 600;
  white-space: nowrap;
}

.status-badge.is-conflict,
.status-badge.is-error,
.sync-error {
  color: var(--danger-color, #a12b2b);
}

.sync-metrics {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0.5rem;
  margin: 0;
}

.sync-metrics div {
  padding: 0.65rem;
  border-radius: 0.55rem;
  background: var(--background-secondary, #f3f3f3);
}

.sync-metrics dt {
  font-size: 0.72rem;
  color: var(--text-secondary, #606060);
}

.sync-metrics dd {
  margin: 0.2rem 0 0;
  font-size: 1.2rem;
  font-weight: 700;
}

.sync-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.sync-actions button,
.sync-status-card > button {
  padding: 0.5rem 0.7rem;
  border: 1px solid var(--border-color, #d7d7d7);
  border-radius: 0.45rem;
  background: transparent;
  color: inherit;
  cursor: pointer;
}

.sync-actions button:disabled,
.sync-status-card > button:disabled {
  cursor: default;
  opacity: 0.55;
}

.conflict-list,
.conflict-row,
.bundle-field {
  display: grid;
  gap: 0.45rem;
}

.conflict-row {
  padding: 0.7rem;
  border-radius: 0.55rem;
  background: var(--background-secondary, #f3f3f3);
}

.conflict-row code {
  overflow-wrap: anywhere;
}

.bundle-field {
  font-size: 0.82rem;
  font-weight: 600;
}

.bundle-field textarea {
  width: 100%;
  box-sizing: border-box;
  resize: vertical;
  padding: 0.65rem;
  border: 1px solid var(--border-color, #d7d7d7);
  border-radius: 0.45rem;
  background: var(--background-color, #fff);
  color: inherit;
  font: 0.75rem/1.4 ui-monospace, SFMono-Regular, Consolas, monospace;
}

.sync-success {
  color: var(--success-color, #2f7351);
}

@media (max-width: 520px) {
  .sync-metrics {
    grid-template-columns: 1fr;
  }
}
</style>
