import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useSyncStore } from '@/stores/sync'

const {
  getStatus,
  listConflicts,
  exportOutbox,
  importEnvelopes,
  rebuildState
} = vi.hoisted(() => ({
  getStatus: vi.fn(),
  listConflicts: vi.fn(),
  exportOutbox: vi.fn(),
  importEnvelopes: vi.fn(),
  rebuildState: vi.fn()
}))

vi.mock('@/api/client', () => ({
  sync: {
    getStatus,
    listConflicts,
    exportOutbox,
    importEnvelopes,
    rebuildState
  }
}))

function statusFixture(overrides = {}) {
  return {
    status: 'local_only',
    provisioned: true,
    outboxOperations: 0,
    pendingOperations: 0,
    conflicts: 0,
    error: null,
    ...overrides
  }
}

describe('Sync Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    getStatus.mockResolvedValue(statusFixture())
    listConflicts.mockResolvedValue([])
    exportOutbox.mockResolvedValue({ schemaVersion: 1, envelopes: [] })
    importEnvelopes.mockResolvedValue({
      received: 0,
      duplicates: 0,
      applied: 0,
      deferred: 0,
      recoveryPending: false,
      warning: null,
      status: statusFixture()
    })
    rebuildState.mockResolvedValue({
      received: 0,
      duplicates: 0,
      applied: 0,
      deferred: 0,
      recoveryPending: false,
      warning: null,
      status: statusFixture()
    })
  })

  it('refreshes the exact local sync state and operation counts', async () => {
    getStatus.mockResolvedValue(statusFixture({
      status: 'pending',
      outboxOperations: 4,
      pendingOperations: 2,
      conflicts: 1
    }))

    const store = useSyncStore()
    const result = await store.refresh()

    expect(result).toEqual({
      status: 'pending',
      provisioned: true,
      outboxOperations: 4,
      pendingOperations: 2,
      conflicts: 1,
      error: null
    })
    expect(store.status).toBe('pending')
    expect(store.outboxOperations).toBe(4)
    expect(store.pendingOperations).toBe(2)
    expect(store.conflictCount).toBe(1)
  })

  it('turns unknown states and transport failures into a safe error status', async () => {
    getStatus
      .mockResolvedValueOnce(statusFixture({ status: 'connected' }))
      .mockRejectedValueOnce(new Error('C:\\private\\vault\\root.key'))

    const store = useSyncStore()

    await store.refresh()
    expect(store.status).toBe('error')
    expect(store.error).toBe('Sync status is unavailable.')

    await store.refresh()
    expect(store.status).toBe('error')
    expect(store.error).toBe('Sync status is unavailable.')
    expect(JSON.stringify(store.$state)).not.toContain('root.key')
  })

  it('keeps only operation IDs when loading conflicts', async () => {
    listConflicts.mockResolvedValue([{
      noteKey: 'private/journal.md',
      selectedOperationId: 'selected-operation-id',
      headOperationIds: ['head-operation-a', 'head-operation-b'],
      plaintextPreview: 'never expose this'
    }])

    const store = useSyncStore()
    const conflicts = await store.listConflicts()

    expect(conflicts).toEqual([{
      selectedOperationId: 'selected-operation-id',
      headOperationIds: ['head-operation-a', 'head-operation-b']
    }])
    expect(JSON.stringify(store.conflicts)).not.toContain('private/journal.md')
    expect(JSON.stringify(store.conflicts)).not.toContain('never expose this')
  })

  it('clears resolved conflict IDs when an authoritative refresh reaches zero', async () => {
    listConflicts.mockResolvedValue([{
      selectedOperationId: 'resolved-operation',
      headOperationIds: ['resolved-a', 'resolved-b']
    }])
    const store = useSyncStore()
    await store.listConflicts()
    expect(store.conflicts).toHaveLength(1)

    getStatus.mockResolvedValue(statusFixture({ conflicts: 0 }))
    await store.refresh()

    expect(store.conflictCount).toBe(0)
    expect(store.conflicts).toEqual([])
  })

  it('passes encrypted bundle strings through export and import without decoding them', async () => {
    const bundle = {
      schemaVersion: 1,
      envelopes: ['{"ciphertext":"opaque-a"}', '{"ciphertext":"opaque-b"}']
    }
    exportOutbox.mockResolvedValue(bundle)
    importEnvelopes.mockResolvedValue({
      received: 2,
      duplicates: 0,
      applied: 1,
      deferred: 1,
      recoveryPending: false,
      warning: null,
      status: statusFixture({ status: 'pending', pendingOperations: 1 })
    })

    const store = useSyncStore()

    expect(await store.exportOutbox()).toEqual(bundle)
    const result = await store.importEnvelopes(bundle)

    expect(importEnvelopes).toHaveBeenCalledWith(bundle)
    expect(result.received).toBe(2)
    expect(store.lastImportResult).toEqual(result)
    expect(store.status).toBe('pending')
    expect(store.pendingOperations).toBe(1)
  })

  it('surfaces committed import recovery as a sanitized error state', async () => {
    const bundle = { schemaVersion: 1, envelopes: ['{"ciphertext":"opaque"}'] }
    importEnvelopes.mockResolvedValue({
      received: 1,
      duplicates: 0,
      applied: 1,
      deferred: 0,
      recoveryPending: true,
      warning: { code: 'derived_state_unavailable', privatePath: 'C:\\private\\vault' },
      status: statusFixture({
        status: 'error',
        error: 'Sync recovery is pending.'
      })
    })

    const store = useSyncStore()
    const result = await store.importEnvelopes(bundle)

    expect(result.recoveryPending).toBe(true)
    expect(store.status).toBe('error')
    expect(store.actionError).toBe('Encrypted operations were imported, but local recovery is pending.')
    expect(JSON.stringify(store.$state)).not.toContain('private\\vault')
  })

  it('rebuilds local sync state and adopts the returned status', async () => {
    rebuildState.mockResolvedValue({
      received: 0,
      duplicates: 0,
      applied: 2,
      deferred: 3,
      recoveryPending: false,
      warning: null,
      status: statusFixture({
        status: 'conflict',
        pendingOperations: 3,
        conflicts: 2
      })
    })

    const store = useSyncStore()
    const result = await store.rebuild()

    expect(rebuildState).toHaveBeenCalledOnce()
    expect(result.status).toBe('conflict')
    expect(store.status).toBe('conflict')
    expect(store.conflictCount).toBe(2)
  })
})
