const TOKEN_PATTERN = /^[0-9a-f]{64}$/
const LOOPBACK_URL_PATTERN = /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})$/
const COMMAND_PATTERN = /^[a-z][a-z0-9_]{0,127}$/
const EVENT_PATTERN = /^[A-Za-z][-A-Za-z0-9_:/.]{0,127}$/
const ERROR_CODE_PATTERN = /^[a-z][a-z0-9_]{0,79}$/
const ANDROID_PROFILE_QUERY = '?grafynE2EProfile=android'

const DEFAULT_MAX_REQUEST_BYTES = 1024 * 1024
const DEFAULT_MAX_RESPONSE_BYTES = 48 * 1024 * 1024
const DEFAULT_MAX_EVENTS = 4096
// The dev-only Rust runtime may synchronously rebuild governed indexes before replying.
const DEFAULT_TIMEOUT_MS = 90_000
const MAX_JSON_DEPTH = 64

function hasExactKeys(value, expected) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false
  const keys = Object.keys(value)
  return keys.length === expected.length && keys.every(key => expected.includes(key))
}

function invalidRequest(message = 'Grafyn E2E invoke arguments must be strict JSON.') {
  return new E2eTransportError('E2E_RUNTIME_INVALID_REQUEST', message)
}

function assertStrictJson(value, ancestors = new Set(), depth = 0) {
  if (depth > MAX_JSON_DEPTH) throw invalidRequest()
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw invalidRequest()
    return
  }
  if (typeof value !== 'object') throw invalidRequest()
  if (ancestors.has(value)) throw invalidRequest()

  ancestors.add(value)
  try {
    if (Array.isArray(value)) {
      if (Object.getPrototypeOf(value) !== Array.prototype
        || Reflect.ownKeys(value).some(key => key !== 'length' && !/^\d+$/.test(String(key)))) {
        throw invalidRequest()
      }
      for (let index = 0; index < value.length; index += 1) {
        if (!Object.prototype.hasOwnProperty.call(value, index)) throw invalidRequest()
        assertStrictJson(value[index], ancestors, depth + 1)
      }
      return
    }

    const prototype = Object.getPrototypeOf(value)
    if (prototype !== Object.prototype && prototype !== null) throw invalidRequest()
    const ownKeys = Reflect.ownKeys(value)
    if (ownKeys.some(key => typeof key !== 'string')) throw invalidRequest()
    for (const key of ownKeys) {
      const descriptor = Object.getOwnPropertyDescriptor(value, key)
      if (!descriptor?.enumerable
        || !Object.prototype.hasOwnProperty.call(descriptor, 'value')) {
        throw invalidRequest()
      }
      if (key === '__proto__' || key === 'prototype' || key === 'constructor') {
        throw invalidRequest()
      }
      assertStrictJson(descriptor.value, ancestors, depth + 1)
    }
  } finally {
    ancestors.delete(value)
  }
}

function serializeInvoke(command, args, maxRequestBytes) {
  if (typeof command !== 'string' || !COMMAND_PATTERN.test(command)) {
    throw invalidRequest('Grafyn E2E command name is invalid.')
  }
  if (args === null || typeof args !== 'object' || Array.isArray(args)) {
    throw invalidRequest()
  }
  assertStrictJson(args)

  const body = JSON.stringify({ command, args })
  if (new TextEncoder().encode(body).byteLength > maxRequestBytes) {
    throw new E2eTransportError(
      'E2E_RUNTIME_REQUEST_TOO_LARGE',
      'Grafyn E2E invoke request exceeds the local bridge limit.',
    )
  }
  return body
}

function validateBound(name, value, maximum) {
  if (!Number.isSafeInteger(value) || value < 1 || value > maximum) {
    throw new Error(`Invalid ${name}`)
  }
}

function validateEventName(value) {
  return typeof value === 'string' && EVENT_PATTERN.test(value)
}

function parseDeclaredLength(headers) {
  const value = headers?.get?.('content-length')
  if (value === null || value === undefined) return null
  if (!/^(0|[1-9][0-9]*)$/.test(value)) return Number.POSITIVE_INFINITY
  const parsed = Number(value)
  return Number.isSafeInteger(parsed) ? parsed : Number.POSITIVE_INFINITY
}

async function readBoundedJson(response, maxResponseBytes) {
  const contentType = response?.headers?.get?.('content-type')
  if (typeof contentType !== 'string'
    || !/^application\/json(?:\s*;|$)/i.test(contentType)) {
    throw new E2eTransportError(
      'E2E_RUNTIME_INVALID_RESPONSE',
      'Grafyn E2E runtime returned an invalid response.',
    )
  }

  const declaredLength = parseDeclaredLength(response.headers)
  if (declaredLength !== null && declaredLength > maxResponseBytes) {
    throw new E2eTransportError(
      'E2E_RUNTIME_RESPONSE_TOO_LARGE',
      'Grafyn E2E runtime response exceeds the local bridge limit.',
    )
  }

  let bytes
  if (response.body?.getReader) {
    const reader = response.body.getReader()
    const chunks = []
    let total = 0
    let streamComplete = false
    while (!streamComplete) {
      const { done, value } = await reader.read()
      streamComplete = done
      if (streamComplete) continue
      total += value.byteLength
      if (total > maxResponseBytes) {
        await reader.cancel()
        throw new E2eTransportError(
          'E2E_RUNTIME_RESPONSE_TOO_LARGE',
          'Grafyn E2E runtime response exceeds the local bridge limit.',
        )
      }
      chunks.push(value)
    }
    bytes = new Uint8Array(total)
    let offset = 0
    for (const chunk of chunks) {
      bytes.set(chunk, offset)
      offset += chunk.byteLength
    }
  } else {
    const text = await response.text()
    bytes = new TextEncoder().encode(text)
    if (bytes.byteLength > maxResponseBytes) {
      throw new E2eTransportError(
        'E2E_RUNTIME_RESPONSE_TOO_LARGE',
        'Grafyn E2E runtime response exceeds the local bridge limit.',
      )
    }
  }

  try {
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  } catch {
    throw new E2eTransportError(
      'E2E_RUNTIME_INVALID_RESPONSE',
      'Grafyn E2E runtime returned an invalid response.',
    )
  }
}

function commandFailure(body) {
  if (!hasExactKeys(body, ['error'])
    || !hasExactKeys(body.error, ['code', 'message'])
    || typeof body.error.code !== 'string'
    || !ERROR_CODE_PATTERN.test(body.error.code)
    || typeof body.error.message !== 'string'
    || body.error.message.length < 1
    || body.error.message.length > 512
    || [...body.error.message].some(character => {
      const point = character.codePointAt(0)
      return point <= 31 || point === 127
    })) {
    throw new E2eTransportError(
      'E2E_RUNTIME_INVALID_RESPONSE',
      'Grafyn E2E runtime returned an invalid response.',
    )
  }
  return new E2eTransportError(body.error.code, body.error.message)
}

function validateSuccess(body, maxEvents) {
  if (!hasExactKeys(body, ['result', 'events'])
    || !Array.isArray(body.events)
    || body.events.length > maxEvents) {
    throw new E2eTransportError(
      'E2E_RUNTIME_INVALID_RESPONSE',
      'Grafyn E2E runtime returned an invalid response.',
    )
  }
  for (const event of body.events) {
    if (!hasExactKeys(event, ['event', 'payload']) || !validateEventName(event.event)) {
      throw new E2eTransportError(
        'E2E_RUNTIME_INVALID_RESPONSE',
        'Grafyn E2E runtime returned an invalid response.',
      )
    }
  }
  return body
}

export class E2eTransportError extends Error {
  constructor(code, message) {
    super(message)
    this.name = 'E2eTransportError'
    this.code = code
  }
}

export function resolveE2eRuntimePlatform(search = '') {
  return search === ANDROID_PROFILE_QUERY ? 'android' : 'desktop'
}

export function resolveE2eTransportConfiguration({
  enabled = false,
  runtimeUrl,
  token,
} = {}) {
  if (!enabled) return null

  const hasUrl = typeof runtimeUrl === 'string' && runtimeUrl.length > 0
  const hasToken = typeof token === 'string' && token.length > 0
  if (!hasUrl && !hasToken) return null

  const match = hasUrl ? LOOPBACK_URL_PATTERN.exec(runtimeUrl) : null
  const port = match ? Number(match[1]) : 0
  if (!match || port > 65_535 || !hasToken || !TOKEN_PATTERN.test(token)) {
    throw new Error('Invalid Grafyn E2E runtime configuration')
  }

  return Object.freeze({ baseUrl: runtimeUrl, token })
}

export function createE2eTransport({
  baseUrl,
  token,
  fetchImpl = globalThis.fetch,
  maxRequestBytes = DEFAULT_MAX_REQUEST_BYTES,
  maxResponseBytes = DEFAULT_MAX_RESPONSE_BYTES,
  maxEvents = DEFAULT_MAX_EVENTS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  profileResolver = () => resolveE2eRuntimePlatform(globalThis.location?.search ?? ''),
} = {}) {
  const configuration = resolveE2eTransportConfiguration({
    enabled: true,
    runtimeUrl: baseUrl,
    token,
  })
  if (!configuration || typeof fetchImpl !== 'function') {
    throw new Error('Invalid Grafyn E2E runtime configuration')
  }
  validateBound('Grafyn E2E request limit', maxRequestBytes, DEFAULT_MAX_REQUEST_BYTES)
  validateBound('Grafyn E2E response limit', maxResponseBytes, DEFAULT_MAX_RESPONSE_BYTES)
  validateBound('Grafyn E2E event limit', maxEvents, DEFAULT_MAX_EVENTS)
  validateBound('Grafyn E2E timeout', timeoutMs, 300_000)

  const listeners = new Map()

  async function listen(eventName, handler) {
    if (!validateEventName(eventName) || typeof handler !== 'function') {
      throw invalidRequest('Grafyn E2E event listener is invalid.')
    }
    let handlers = listeners.get(eventName)
    if (!handlers) {
      handlers = new Set()
      listeners.set(eventName, handlers)
    }
    handlers.add(handler)
    let active = true
    return () => {
      if (!active) return
      active = false
      handlers.delete(handler)
      if (handlers.size === 0) listeners.delete(eventName)
    }
  }

  function dispatch(events) {
    for (const event of events) {
      const handlers = listeners.get(event.event)
      if (!handlers) continue
      for (const handler of [...handlers]) {
        try {
          handler(Object.freeze({ event: event.event, payload: event.payload }))
        } catch {
          console.error('Grafyn E2E event listener failed.')
        }
      }
    }
  }

  async function invoke(command, args = {}) {
    const body = serializeInvoke(command, args, maxRequestBytes)
    const profile = profileResolver()
    if (profile !== 'android' && profile !== 'desktop') {
      throw invalidRequest('Grafyn E2E runtime profile is invalid.')
    }

    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), timeoutMs)
    try {
      const response = await fetchImpl(`${configuration.baseUrl}/invoke`, {
        method: 'POST',
        headers: {
          Accept: 'application/json',
          Authorization: `Bearer ${configuration.token}`,
          'Content-Type': 'application/json',
          'X-Grafyn-E2E-Device': 'device-a',
          'X-Grafyn-E2E-Profile': profile,
        },
        body,
        credentials: 'omit',
        redirect: 'error',
        signal: controller.signal,
      })
      const responseBody = await readBoundedJson(response, maxResponseBytes)
      if (response.status === 400) throw commandFailure(responseBody)
      if (response.status !== 200) {
        throw new E2eTransportError(
          'E2E_RUNTIME_HTTP_ERROR',
          `Grafyn E2E runtime rejected the request (${response.status}).`,
        )
      }

      const success = validateSuccess(responseBody, maxEvents)
      dispatch(success.events)
      return success.result
    } catch (error) {
      if (error instanceof E2eTransportError) throw error
      if (controller.signal.aborted) {
        throw new E2eTransportError(
          'E2E_RUNTIME_TIMEOUT',
          'Grafyn E2E runtime did not respond in time.',
        )
      }
      throw new E2eTransportError(
        'E2E_RUNTIME_UNREACHABLE',
        'Grafyn E2E runtime is unavailable.',
      )
    } finally {
      clearTimeout(timeout)
    }
  }

  return Object.freeze({ invoke, listen })
}
