import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { sync as syncApi } from '@/api/client'

const SYNC_STATUSES = new Set([
  'not_provisioned',
  'local_only',
  'pending',
  'conflict',
  'error'
])

function unavailableStatus() {
  return {
    status: 'error',
    provisioned: false,
    outboxOperations: 0,
    pendingOperations: 0,
    conflicts: 0,
    error: 'Sync status is unavailable.'
  }
}

function initialStatus() {
  return {
    status: 'not_provisioned',
    provisioned: false,
    outboxOperations: 0,
    pendingOperations: 0,
    conflicts: 0,
    error: null
  }
}

function operationCount(value) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error('invalid sync operation count')
  }
  return value
}

function normalizeStatus(value) {
  if (!value || typeof value !== 'object' || !SYNC_STATUSES.has(value.status)) {
    throw new Error('invalid sync status')
  }

  return {
    status: value.status,
    provisioned: value.provisioned === true,
    outboxOperations: operationCount(value.outboxOperations),
    pendingOperations: operationCount(value.pendingOperations),
    conflicts: operationCount(value.conflicts),
    error: value.status === 'error' && value.error === 'Sync recovery is pending.'
      ? 'Sync recovery is pending.'
      : value.status === 'error' ? 'Sync status is unavailable.' : null
  }
}

function normalizeImportResult(value) {
  if (
    !value ||
    typeof value !== 'object' ||
    typeof value.recoveryPending !== 'boolean'
  ) {
    throw new Error('invalid sync import result')
  }

  const warning = value.warning === null
    ? null
    : value.warning && typeof value.warning.code === 'string'
      ? { code: value.warning.code }
      : null

  return {
    received: operationCount(value.received),
    duplicates: operationCount(value.duplicates),
    applied: operationCount(value.applied),
    deferred: operationCount(value.deferred),
    recoveryPending: value.recoveryPending,
    warning,
    status: normalizeStatus(value.status)
  }
}

function normalizeBundle(value) {
  if (
    !value ||
    typeof value !== 'object' ||
    value.schemaVersion !== 1 ||
    !Array.isArray(value.envelopes) ||
    !value.envelopes.every(envelope => typeof envelope === 'string')
  ) {
    throw new Error('invalid encrypted sync bundle')
  }

  return {
    schemaVersion: 1,
    envelopes: [...value.envelopes]
  }
}

function normalizeConflicts(value) {
  if (!Array.isArray(value)) {
    throw new Error('invalid sync conflicts')
  }

  return value.map(conflict => {
    if (
      !conflict ||
      typeof conflict.selectedOperationId !== 'string' ||
      !Array.isArray(conflict.headOperationIds) ||
      !conflict.headOperationIds.every(operationId => typeof operationId === 'string')
    ) {
      throw new Error('invalid sync conflict')
    }

    return {
      selectedOperationId: conflict.selectedOperationId,
      headOperationIds: [...conflict.headOperationIds]
    }
  })
}

export const useSyncStore = defineStore('sync', () => {
  const snapshot = ref(initialStatus())
  const conflicts = ref([])
  const exportedBundle = ref(null)
  const lastImportResult = ref(null)
  const loading = ref(false)
  const actionInProgress = ref(null)
  const actionError = ref(null)

  const status = computed(() => snapshot.value.status)
  const provisioned = computed(() => snapshot.value.provisioned)
  const outboxOperations = computed(() => snapshot.value.outboxOperations)
  const pendingOperations = computed(() => snapshot.value.pendingOperations)
  const conflictCount = computed(() => snapshot.value.conflicts)
  const error = computed(() => snapshot.value.error)

  function applyStatus(value) {
    snapshot.value = normalizeStatus(value)
    if (snapshot.value.conflicts === 0) {
      conflicts.value = []
    }
    return snapshot.value
  }

  async function refresh() {
    loading.value = true
    actionError.value = null
    try {
      return applyStatus(await syncApi.getStatus())
    } catch (_error) {
      return applyStatus(unavailableStatus())
    } finally {
      loading.value = false
    }
  }

  async function listConflicts() {
    actionInProgress.value = 'conflicts'
    actionError.value = null
    try {
      conflicts.value = normalizeConflicts(await syncApi.listConflicts())
      return conflicts.value
    } catch (_error) {
      conflicts.value = []
      actionError.value = 'Sync conflicts are unavailable.'
      return conflicts.value
    } finally {
      actionInProgress.value = null
    }
  }

  async function exportOutbox() {
    actionInProgress.value = 'export'
    actionError.value = null
    try {
      exportedBundle.value = normalizeBundle(await syncApi.exportOutbox())
      return exportedBundle.value
    } catch (_error) {
      exportedBundle.value = null
      actionError.value = 'Encrypted sync export is unavailable.'
      return null
    } finally {
      actionInProgress.value = null
    }
  }

  async function importEnvelopes(value) {
    actionInProgress.value = 'import'
    actionError.value = null
    lastImportResult.value = null
    try {
      const bundle = normalizeBundle(value)
      const result = normalizeImportResult(await syncApi.importEnvelopes(bundle))
      applyStatus(result.status)
      lastImportResult.value = result
      if (result.recoveryPending) {
        actionError.value = 'Encrypted operations were imported, but local recovery is pending.'
      }
      return result
    } catch (_error) {
      actionError.value = 'Encrypted sync bundle could not be imported.'
      return null
    } finally {
      actionInProgress.value = null
    }
  }

  async function rebuild() {
    actionInProgress.value = 'rebuild'
    actionError.value = null
    try {
      const result = normalizeImportResult(await syncApi.rebuildState())
      if (result.recoveryPending) {
        actionError.value = 'Local sync recovery is still pending.'
      }
      return applyStatus(result.status)
    } catch (_error) {
      actionError.value = 'Local sync state could not be rebuilt.'
      return null
    } finally {
      actionInProgress.value = null
    }
  }

  return {
    status,
    provisioned,
    outboxOperations,
    pendingOperations,
    conflictCount,
    error,
    conflicts,
    exportedBundle,
    lastImportResult,
    loading,
    actionInProgress,
    actionError,
    refresh,
    listConflicts,
    exportOutbox,
    importEnvelopes,
    rebuild
  }
})
