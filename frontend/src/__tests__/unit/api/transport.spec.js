import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import {
  createTransport,
  getRuntimeStatus,
  resetTransport,
  setRuntimeProfile,
  setRuntimeStatus,
  setTransport,
} from '@/api/transport'
import { notes } from '@/api/client'
import { CapabilityUnavailableError, assertCapability } from '@/platform/capabilities'
import { useBootStore } from '@/stores/boot'
import { useCanvasStore } from '@/stores/canvas'

describe('frontend transport seam', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    resetTransport()
  })

  it('routes API namespaces through an injected invoke operation', async () => {
    const invoke = vi.fn().mockResolvedValue([{ id: 'note-1' }])
    setTransport(createTransport({ invoke }))

    await expect(notes.list()).resolves.toEqual([{ id: 'note-1' }])
    expect(invoke).toHaveBeenCalledWith('list_notes', {})
  })

  it('exposes exactly the four approved runtime bridge names', async () => {
    const invoke = vi.fn()
    const listen = vi.fn()
    const openExternal = vi.fn().mockResolvedValue(undefined)
    const showMainWindow = vi.fn().mockResolvedValue(undefined)
    const transport = createTransport({ invoke, listen, openExternal, showMainWindow })

    expect(Object.keys(transport).sort()).toEqual([
      'invoke',
      'listen',
      'openExternal',
      'showMainWindow',
    ])

    await transport.openExternal({ type: 'url', url: 'https://grafyn.app/docs' })
    await transport.openExternal({ type: 'file-dialog', options: { multiple: false } })
    await transport.showMainWindow()

    expect(openExternal).toHaveBeenNthCalledWith(1, {
      type: 'url',
      url: 'https://grafyn.app/docs',
    })
    expect(openExternal).toHaveBeenNthCalledWith(2, {
      type: 'file-dialog',
      options: { multiple: false },
    })
    expect(showMainWindow).toHaveBeenCalledOnce()
  })

  it('uses the active runtime when asserting a capability', () => {
    setTransport(createTransport())
    setRuntimeProfile({ name: 'android-compact' })

    expect(() => assertCapability('mcp')).toThrow(CapabilityUnavailableError)
  })

  it('holds injected runtime health only for the active bootstrap lifetime', () => {
    const status = Object.freeze({ schemaVersion: 1, runtime: 'android' })
    setRuntimeStatus(status)

    expect(getRuntimeStatus()).toBe(status)

    resetTransport()
    expect(getRuntimeStatus()).toBeNull()
  })

  it('routes boot event listening through an injected transport', async () => {
    const listen = vi.fn().mockResolvedValue(vi.fn())
    const invoke = vi.fn().mockResolvedValue({ ready: true, phase: 'ready' })
    setTransport(createTransport({ invoke, listen }))

    const boot = useBootStore()
    await boot.initialize()

    expect(listen).toHaveBeenCalledWith('boot-status', expect.any(Function))
    expect(invoke).toHaveBeenCalledWith('get_boot_status', {})
  })

  it('routes Canvas stream listening through an injected transport', async () => {
    let streamHandler
    const listen = vi.fn().mockImplementation(async (_name, handler) => {
      streamHandler = handler
      return vi.fn()
    })
    const invoke = vi.fn().mockImplementation(async (command) => {
      if (command === 'send_prompt') {
        streamHandler({
          payload: {
            session_id: 'session-1',
            type: 'complete',
            tile_id: 'tile-1',
            model_id: 'model-1',
          },
        })
        return 'tile-1'
      }
      return null
    })
    setTransport(createTransport({ invoke, listen }))

    const canvas = useCanvasStore()
    canvas.currentSession = { id: 'session-1', prompt_tiles: [], debates: [] }
    await canvas.sendPrompt('Hello', ['model-1'])

    expect(listen).toHaveBeenCalledWith('canvas-stream', expect.any(Function))
  })
})
