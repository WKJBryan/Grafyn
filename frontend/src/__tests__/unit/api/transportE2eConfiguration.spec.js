import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const tauri = vi.hoisted(() => ({
  invoke: vi.fn().mockResolvedValue(null),
  profile: Object.freeze({
    name: 'plain-web',
    isTauri: false,
    platform: null,
    nativePlugins: false,
  }),
}))

vi.mock('@/api/tauriTransport', () => ({
  getTauriRuntimeProfile: () => tauri.profile,
  tauriTransport: {
    invoke: tauri.invoke,
    listen: vi.fn(),
    openExternal: vi.fn(),
    showMainWindow: vi.fn(),
  },
}))

function runtimeResponse(result = null) {
  return new Response(JSON.stringify({ result, events: [] }), {
    headers: { 'content-type': 'application/json' },
  })
}

describe('development E2E transport activation', () => {
  beforeEach(() => {
    vi.resetModules()
    tauri.invoke.mockClear()
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_URL', '')
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_TOKEN', '')
    window.history.replaceState({}, '', '/')
  })

  afterEach(() => {
    window.history.replaceState({}, '', '/')
    vi.unstubAllEnvs()
    vi.unstubAllGlobals()
  })

  it('keeps the normal Tauri boundary when no bridge credentials exist', async () => {
    const transport = await import('@/api/transport')

    expect(transport.getRuntimeProfile()).toBe(tauri.profile)
    await transport.getTransport().invoke('list_notes', {})
    expect(tauri.invoke).toHaveBeenCalledWith('list_notes', {})
  })

  it('freezes an initially exact Android marker across Router query removal', async () => {
    const token = 'b'.repeat(64)
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_URL', 'http://127.0.0.1:43128')
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_TOKEN', token)
    window.history.replaceState({}, '', '/?grafynE2EProfile=android')
    const fetchImpl = vi.fn()
      .mockResolvedValueOnce(runtimeResponse({ schemaVersion: 1, runtime: 'android' }))
      .mockResolvedValueOnce(runtimeResponse([]))
    vi.stubGlobal('fetch', fetchImpl)

    const transport = await import('@/api/transport')
    expect(transport.getRuntimeProfile()).toEqual({
      name: 'android-compact',
      isTauri: true,
      platform: 'android',
      nativePlugins: false,
    })
    await transport.getTransport().invoke('get_runtime_status', {})

    window.history.pushState({}, '', '/twin')

    expect(transport.getRuntimeProfile()).toEqual({
      name: 'android-compact',
      isTauri: true,
      platform: 'android',
      nativePlugins: false,
    })
    await transport.getTransport().invoke('list_twin_proposals', { request: {} })
    for (const [, request] of fetchImpl.mock.calls) {
      expect(request.headers).toMatchObject({
        'X-Grafyn-E2E-Device': 'device-a',
        'X-Grafyn-E2E-Profile': 'android',
      })
    }
  })

  it.each([
    '?grafynE2EProfile=Android',
    '?grafynE2EProfile=android&extra=1',
    '?extra=1&grafynE2EProfile=android',
  ])('falls back to desktop for an invalid initial marker: %s', async (search) => {
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_URL', 'http://127.0.0.1:43129')
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_TOKEN', 'c'.repeat(64))
    window.history.replaceState({}, '', `/${search}`)
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(runtimeResponse()))

    const transport = await import('@/api/transport')

    expect(transport.getRuntimeProfile()).toMatchObject({
      name: 'desktop-wide',
      platform: 'windows',
      nativePlugins: false,
    })
    await transport.getTransport().invoke('get_runtime_status', {})
    expect(globalThis.fetch.mock.calls[0][1].headers)
      .toMatchObject({ 'X-Grafyn-E2E-Profile': 'desktop' })
  })

  it('fails closed when only one development credential is supplied', async () => {
    vi.stubEnv('VITE_GRAFYN_E2E_RUNTIME_URL', 'http://127.0.0.1:43128')

    await expect(import('@/api/transport'))
      .rejects.toThrow('Invalid Grafyn E2E runtime configuration')
  })
})
