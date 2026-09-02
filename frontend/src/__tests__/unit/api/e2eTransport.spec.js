import { describe, expect, it, vi } from 'vitest'
import {
  E2eTransportError,
  createE2eTransport,
  resolveE2eTransportConfiguration,
  resolveE2eRuntimePlatform,
} from '@/api/e2eTransport'

const RUNTIME_URL = 'http://127.0.0.1:43127'
const TOKEN = 'a'.repeat(64)

function jsonResponse(body, { status = 200, headers = {} } = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      ...headers,
    },
  })
}

describe('E2E runtime configuration', () => {
  it('stays disabled outside Vite development even when credentials are supplied', () => {
    expect(resolveE2eTransportConfiguration({
      enabled: false,
      runtimeUrl: RUNTIME_URL,
      token: TOKEN,
    })).toBeNull()
  })

  it('stays disabled when neither development credential is configured', () => {
    expect(resolveE2eTransportConfiguration({ enabled: true })).toBeNull()
  })

  it('accepts only an exact IPv4 loopback HTTP origin and lowercase 32-byte token', () => {
    expect(resolveE2eTransportConfiguration({
      enabled: true,
      runtimeUrl: RUNTIME_URL,
      token: TOKEN,
    })).toEqual({ baseUrl: RUNTIME_URL, token: TOKEN })
  })

  it.each([
    ['http://localhost:43127', TOKEN],
    ['https://127.0.0.1:43127', TOKEN],
    ['http://127.0.0.1:43127/', TOKEN],
    ['http://127.0.0.1:43127/invoke', TOKEN],
    ['http://127.0.0.1:43127?debug=1', TOKEN],
    [RUNTIME_URL, 'A'.repeat(64)],
    [RUNTIME_URL, 'a'.repeat(63)],
    [RUNTIME_URL, undefined],
    [undefined, TOKEN],
  ])('rejects an incomplete or unsafe development bridge (%s)', (runtimeUrl, token) => {
    expect(() => resolveE2eTransportConfiguration({
      enabled: true,
      runtimeUrl,
      token,
    })).toThrow('Invalid Grafyn E2E runtime configuration')
  })

  it.each([
    ['?grafynE2EProfile=android', 'android'],
    ['', 'desktop'],
    ['?grafynE2EProfile=Android', 'desktop'],
    ['?grafynE2EProfile=android&extra=1', 'desktop'],
    ['?extra=1&grafynE2EProfile=android', 'desktop'],
    ['?grafynE2EProfile=android/', 'desktop'],
  ])('selects Android only for the exact marker %s', (search, expected) => {
    expect(resolveE2eRuntimePlatform(search)).toBe(expected)
  })
})

describe('loopback E2E transport', () => {
  it('posts the exact authenticated invoke envelope and dispatches ordered events first', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({
      result: { id: 'tile-1' },
      events: [
        { event: 'canvas-stream', payload: { type: 'chunk', value: 'A' } },
        { event: 'canvas-stream', payload: { type: 'complete' } },
      ],
    }))
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl,
    })
    const order = []
    await transport.listen('canvas-stream', event => {
      order.push(event.payload.type)
    })

    const result = await transport.invoke('send_prompt', { sessionId: 'session-1' })
      .then(value => {
        order.push('resolved')
        return value
      })

    expect(result).toEqual({ id: 'tile-1' })
    expect(order).toEqual(['chunk', 'complete', 'resolved'])
    expect(fetchImpl).toHaveBeenCalledOnce()
    const [url, request] = fetchImpl.mock.calls[0]
    expect(url).toBe(`${RUNTIME_URL}/invoke`)
    expect(request).toMatchObject({
      method: 'POST',
      body: '{"command":"send_prompt","args":{"sessionId":"session-1"}}',
      credentials: 'omit',
      redirect: 'error',
    })
    expect(request.headers).toEqual({
      Accept: 'application/json',
      Authorization: `Bearer ${TOKEN}`,
      'Content-Type': 'application/json',
      'X-Grafyn-E2E-Device': 'device-a',
      'X-Grafyn-E2E-Profile': 'desktop',
    })
  })

  it('returns an idempotent unlisten that removes only its local listener', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({
      result: null,
      events: [{ event: 'boot-status', payload: { phase: 'ready' } }],
    }))
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl,
    })
    const retained = vi.fn()
    const removed = vi.fn()
    await transport.listen('boot-status', retained)
    const unlisten = await transport.listen('boot-status', removed)

    unlisten()
    unlisten()
    await transport.invoke('get_boot_status')

    expect(retained).toHaveBeenCalledWith({
      event: 'boot-status',
      payload: { phase: 'ready' },
    })
    expect(removed).not.toHaveBeenCalled()
  })

  it.each([
    ['bad-command', {}],
    ['list_notes', []],
    ['list_notes', { value: undefined }],
    ['list_notes', { value: Number.NaN }],
    ['list_notes', { value: new Date('2026-09-01T00:00:00Z') }],
    ['list_notes', Object.assign(Object.create({ inherited: true }), { value: 1 })],
  ])('rejects non-canonical invoke input before network I/O (%s)', async (command, args) => {
    const fetchImpl = vi.fn()
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl,
    })

    await expect(transport.invoke(command, args)).rejects.toMatchObject({
      code: 'E2E_RUNTIME_INVALID_REQUEST',
    })
    expect(fetchImpl).not.toHaveBeenCalled()
  })

  it('rejects cyclic and oversized invoke input before network I/O', async () => {
    const fetchImpl = vi.fn()
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl,
      maxRequestBytes: 128,
    })
    const cyclic = {}
    cyclic.self = cyclic

    await expect(transport.invoke('list_notes', cyclic)).rejects.toMatchObject({
      code: 'E2E_RUNTIME_INVALID_REQUEST',
    })
    await expect(transport.invoke('list_notes', { query: 'x'.repeat(256) }))
      .rejects.toMatchObject({ code: 'E2E_RUNTIME_REQUEST_TOO_LARGE' })
    expect(fetchImpl).not.toHaveBeenCalled()
  })

  it('surfaces only bounded structured command failures', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(jsonResponse({
      error: { code: 'note_not_found', message: 'The requested note was not found.' },
    }, { status: 400 }))
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl,
    })

    await expect(transport.invoke('get_note', { id: 'missing' })).rejects.toEqual(
      expect.objectContaining({
        name: 'E2eTransportError',
        code: 'note_not_found',
        message: 'The requested note was not found.',
      }),
    )
  })

  it.each([
    [jsonResponse({ result: null, events: [], extra: true }), 'E2E_RUNTIME_INVALID_RESPONSE'],
    [new Response('not-json', { headers: { 'content-type': 'text/plain' } }), 'E2E_RUNTIME_INVALID_RESPONSE'],
    [jsonResponse({ result: null, events: [{ event: 'bad event', payload: null }] }), 'E2E_RUNTIME_INVALID_RESPONSE'],
    [jsonResponse({ result: null, events: [] }, { headers: { 'content-length': '9999' } }), 'E2E_RUNTIME_RESPONSE_TOO_LARGE'],
  ])('rejects malformed or oversized runtime replies without exposing their body', async (response, code) => {
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl: vi.fn().mockResolvedValue(response),
      maxResponseBytes: 128,
    })

    await expect(transport.invoke('list_notes', {})).rejects.toMatchObject({ code })
  })

  it('turns network detail into a fixed error that cannot leak the bearer token', async () => {
    const transport = createE2eTransport({
      baseUrl: RUNTIME_URL,
      token: TOKEN,
      fetchImpl: vi.fn().mockRejectedValue(new Error(`network failed with ${TOKEN}`)),
    })

    let failure
    try {
      await transport.invoke('list_notes', {})
    } catch (error) {
      failure = error
    }

    expect(failure).toBeInstanceOf(E2eTransportError)
    expect(failure).toMatchObject({ code: 'E2E_RUNTIME_UNREACHABLE' })
    expect(failure.message).not.toContain(TOKEN)
  })

  it('allows a debug runtime request to finish after 30 seconds while remaining bounded', async () => {
    vi.useFakeTimers()
    try {
      const fetchImpl = vi.fn((_, { signal }) => new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), {
          once: true,
        })
        setTimeout(() => resolve(jsonResponse({ result: null, events: [] })), 31_000)
      }))
      const transport = createE2eTransport({
        baseUrl: RUNTIME_URL,
        token: TOKEN,
        fetchImpl,
      })

      const pending = transport.invoke('list_notes')
      await vi.advanceTimersByTimeAsync(31_000)

      await expect(pending).resolves.toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })
})
