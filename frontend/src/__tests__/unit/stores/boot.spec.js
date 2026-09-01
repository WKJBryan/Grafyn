import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useBootStore } from '@/stores/boot'
import * as apiClient from '@/api/client'
import { getRuntimeStatus, resetTransport, setRuntimeStatus } from '@/api/transport'
import { CAPABILITY_NAMES, normalizeRuntimeStatus } from '@/platform/capabilities'

const { listenMock, unlistenMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

describe('Boot Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    resetTransport()
    vi.clearAllMocks()
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-03-31T00:00:00Z'))
    listenMock.mockResolvedValue(unlistenMock)
  })

  it('refreshes authoritative runtime health before exposing a terminal boot failure', async () => {
    const healthyStatus = desktopRuntimeStatus(true)
    const revokedStatus = desktopRuntimeStatus(false)
    let resolveRuntimeStatus
    setRuntimeStatus(healthyStatus)
    vi.spyOn(apiClient.boot, 'status').mockResolvedValue({
      phase: 'building_graph',
      message: 'Building graph',
      ready: false,
      error: null,
    })
    const runtimeStatusSpy = vi.spyOn(apiClient.runtime, 'getStatus').mockImplementation(() => (
      new Promise(resolve => {
        resolveRuntimeStatus = resolve
      })
    ))

    const store = useBootStore()
    await store.initialize()
    const handler = listenMock.mock.calls[0][1]
    handler({
      payload: {
        phase: 'failed',
        message: 'Startup failed',
        ready: false,
        error: 'canonical runtime unavailable',
      },
    })
    await Promise.resolve()

    expect(runtimeStatusSpy).toHaveBeenCalledOnce()
    expect(getRuntimeStatus()).toBeNull()
    expect(store.failed).toBe(false)

    resolveRuntimeStatus(revokedStatus)
    await vi.waitFor(() => expect(store.failed).toBe(true))

    expect(getRuntimeStatus()).toEqual(revokedStatus)
    store.dismissSplash()
    expect(store.isVisible).toBe(false)
    store.cleanup()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('loads the initial boot status and reacts to boot-status events', async () => {
    vi.spyOn(apiClient.boot, 'status').mockResolvedValue({
      phase: 'building_search_index',
      message: 'Building search index',
      ready: false,
      error: null,
    })

    const store = useBootStore()
    await store.initialize()

    expect(store.phase).toBe('building_search_index')
    expect(store.isVisible).toBe(true)
    expect(listenMock).toHaveBeenCalledWith('boot-status', expect.any(Function))

    const handler = listenMock.mock.calls[0][1]
    handler({
      payload: {
        phase: 'ready',
        message: 'Grafyn is ready',
        ready: true,
        error: null,
      },
    })

    expect(store.ready).toBe(true)
    expect(store.isVisible).toBe(false)
    store.cleanup()
  })

  it('enters failed state when boot status fetch fails', async () => {
    vi.spyOn(apiClient.boot, 'status').mockRejectedValue(new Error('status unavailable'))

    const store = useBootStore()
    await store.initialize()

    expect(store.failed).toBe(true)
    expect(store.error).toBe('status unavailable')
    expect(store.isVisible).toBe(true)

    store.dismissSplash()
    expect(store.isVisible).toBe(false)
    store.cleanup()
  })

  it('polls boot status and hides the splash when ready is returned without an event', async () => {
    const statusSpy = vi.spyOn(apiClient.boot, 'status')
    statusSpy
      .mockResolvedValueOnce({
        phase: 'building_graph',
        message: 'Building graph from your notes',
        ready: false,
        error: null,
      })
      .mockResolvedValueOnce({
        phase: 'ready',
        message: 'Grafyn is ready',
        ready: true,
        error: null,
      })

    const store = useBootStore()
    await store.initialize()

    await vi.advanceTimersByTimeAsync(2000)

    expect(statusSpy).toHaveBeenCalledTimes(2)
    expect(store.ready).toBe(true)
    expect(store.isVisible).toBe(false)
    store.cleanup()
  })

  it('shows a synthetic failure when startup makes no progress for too long', async () => {
    vi.spyOn(apiClient.boot, 'status').mockResolvedValue({
      phase: 'building_search_index',
      message: 'Building search index',
      ready: false,
      error: null,
    })

    const store = useBootStore()
    await store.initialize()

    await vi.advanceTimersByTimeAsync(20000)

    expect(store.failed).toBe(true)
    expect(store.status.error).toContain('taking longer than expected')
    expect(store.status.error).toContain('building the search index')
    expect(store.isVisible).toBe(true)
    store.cleanup()
  })

  it('clears the synthetic failure when a later ready status arrives', async () => {
    const statusSpy = vi.spyOn(apiClient.boot, 'status')
    statusSpy.mockResolvedValue({
      phase: 'building_chunk_index',
      message: 'Building chunk index',
      ready: false,
      error: null,
    })

    const store = useBootStore()
    await store.initialize()
    await vi.advanceTimersByTimeAsync(45000)

    expect(store.failed).toBe(true)
    expect(store.status.error).toContain('taking longer than expected')

    statusSpy.mockResolvedValue({
      phase: 'ready',
      message: 'Grafyn is ready',
      ready: true,
      error: null,
    })

    await vi.advanceTimersByTimeAsync(2000)

    expect(store.ready).toBe(true)
    expect(store.status.error).toBe(null)
    expect(store.isVisible).toBe(false)
    store.cleanup()
  })

  it('cleanup stops polling and watchdog timers', async () => {
    const statusSpy = vi.spyOn(apiClient.boot, 'status').mockResolvedValue({
      phase: 'building_graph',
      message: 'Building graph from your notes',
      ready: false,
      error: null,
    })

    const store = useBootStore()
    await store.initialize()
    store.cleanup()

    await vi.advanceTimersByTimeAsync(60000)

    expect(statusSpy).toHaveBeenCalledTimes(1)
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })
})

function desktopRuntimeStatus(available) {
  const health = available
    ? { status: 'ready', code: null, message: null }
    : {
        status: 'unavailable',
        code: 'canonical_runtime_unavailable',
        message: 'The canonical local runtime is unavailable.',
      }
  return normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'desktop',
    capabilities: Object.fromEntries(CAPABILITY_NAMES.map(name => [name, available])),
    vault: { kind: 'user_selected', available },
    secureSecrets: health,
    nativeImageShare: available
      ? {
          status: 'unavailable',
          code: 'desktop_save_as',
          message: 'Desktop exports use the governed Save As boundary.',
        }
      : health,
    diagnostics: [],
  })
}
