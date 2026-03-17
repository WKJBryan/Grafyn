import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useBootStore } from '@/stores/boot'
import * as apiClient from '@/api/client'

const listenMock = vi.fn()
const unlistenMock = vi.fn()

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

describe('Boot Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    listenMock.mockResolvedValue(unlistenMock)
  })

  it('loads the initial boot status and reacts to boot-status events', async () => {
    vi.spyOn(apiClient.boot, 'status').mockResolvedValue({
      phase: 'building_indices',
      message: 'Building graph and search index',
      ready: false,
      error: null,
    })

    const store = useBootStore()
    await store.initialize()

    expect(store.phase).toBe('building_indices')
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
  })
})
