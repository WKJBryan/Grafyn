import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import SyncStatusCard from '@/components/sync/SyncStatusCard.vue'

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

describe('SyncStatusCard', () => {
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

  it('labels the feature as a local foundation with no configured relay', async () => {
    getStatus.mockResolvedValue(statusFixture({
      status: 'pending',
      outboxOperations: 5,
      pendingOperations: 2,
      conflicts: 1
    }))

    const wrapper = mount(SyncStatusCard, {
      global: { plugins: [createPinia()] }
    })
    await flushPromises()

    expect(wrapper.get('[data-testid="sync-title"]').text()).toBe('Sync foundation')
    expect(wrapper.get('[data-testid="relay-status"]').text()).toBe('Relay not configured')
    expect(wrapper.get('[data-testid="outbox-count"]').text()).toBe('5')
    expect(wrapper.get('[data-testid="pending-count"]').text()).toBe('2')
    expect(wrapper.get('[data-testid="conflict-count"]').text()).toBe('1')
    expect(wrapper.text()).not.toMatch(/connected|subscription|account/i)
  })

  it('shows conflict operation IDs without note names or content', async () => {
    getStatus.mockResolvedValue(statusFixture({ status: 'conflict', conflicts: 1 }))
    listConflicts.mockResolvedValue([{
      noteKey: 'private/reflection.md',
      selectedOperationId: 'operation-selected',
      headOperationIds: ['operation-a', 'operation-b'],
      plaintextPreview: 'private thought'
    }])

    const wrapper = mount(SyncStatusCard, {
      global: { plugins: [createPinia()] }
    })
    await flushPromises()

    expect(wrapper.text()).toContain('operation-selected')
    expect(wrapper.text()).toContain('operation-a')
    expect(wrapper.text()).toContain('operation-b')
    expect(wrapper.text()).not.toContain('private/reflection.md')
    expect(wrapper.text()).not.toContain('private thought')
  })

  it('supports manual encrypted bundle export and import without file or relay claims', async () => {
    const bundle = {
      schemaVersion: 1,
      envelopes: ['{"ciphertext":"opaque-envelope"}']
    }
    exportOutbox.mockResolvedValue(bundle)
    importEnvelopes.mockResolvedValue({
      received: 1,
      duplicates: 0,
      applied: 1,
      deferred: 0,
      recoveryPending: false,
      warning: null,
      status: statusFixture()
    })

    const wrapper = mount(SyncStatusCard, {
      global: { plugins: [createPinia()] }
    })
    await flushPromises()

    await wrapper.get('[data-testid="export-sync"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[data-testid="exported-bundle"]').element.value).toContain('opaque-envelope')

    await wrapper.get('[data-testid="imported-bundle"]').setValue(JSON.stringify(bundle))
    await wrapper.get('[data-testid="import-sync"]').trigger('click')
    await flushPromises()

    expect(importEnvelopes).toHaveBeenCalledWith(bundle)
    expect(wrapper.text()).toContain('1 encrypted operation imported')
    expect(wrapper.text()).not.toMatch(/upload|download|relay connected/i)
  })

  it('does not present a committed import as healthy while recovery is pending', async () => {
    const bundle = {
      schemaVersion: 1,
      envelopes: ['{"ciphertext":"opaque-envelope"}']
    }
    importEnvelopes.mockResolvedValue({
      received: 1,
      duplicates: 0,
      applied: 1,
      deferred: 0,
      recoveryPending: true,
      warning: { code: 'derived_state_unavailable' },
      status: statusFixture({
        status: 'error',
        error: 'Sync recovery is pending.'
      })
    })

    const wrapper = mount(SyncStatusCard, {
      global: { plugins: [createPinia()] }
    })
    await flushPromises()
    await wrapper.get('[data-testid="imported-bundle"]').setValue(JSON.stringify(bundle))
    await wrapper.get('[data-testid="import-sync"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Encrypted operations were imported, but local recovery is pending.')
    expect(wrapper.text()).not.toContain('1 encrypted operation imported')
    expect(wrapper.find('.sync-success').exists()).toBe(false)
  })

  it('does not render raw transport errors', async () => {
    getStatus.mockRejectedValue(new Error('C:\\private\\vault\\sync-secret.key'))

    const wrapper = mount(SyncStatusCard, {
      global: { plugins: [createPinia()] }
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Sync status is unavailable.')
    expect(wrapper.text()).not.toContain('sync-secret.key')
  })
})
