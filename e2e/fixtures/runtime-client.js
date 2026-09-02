const ORIGIN = 'http://127.0.0.1:5173'
const MAX_RESPONSE_BYTES = 48 * 1024 * 1024
// Duplicate/reordered sync imports run full governed repair in the debug runtime.
const REQUEST_TIMEOUT_MS = 180_000
const TOKEN_PATTERN = /^[a-f0-9]{64}$/
const COMMAND_PATTERN = /^[a-z][a-z0-9_]{0,95}$/
const PROFILES = new Set(['desktop', 'android'])
const DEVICES = new Set(['device-a', 'device-b'])

export class RuntimeInvokeError extends Error {
  constructor(code, message) {
    super(message)
    this.name = 'RuntimeInvokeError'
    this.code = code
  }
}

function exactKeys(value, keys) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const actual = Object.keys(value)
  return actual.length === keys.length && actual.every((key) => keys.includes(key))
}

function validateBaseUrl(value) {
  let parsed
  try {
    parsed = new URL(value)
  } catch {
    throw new Error('runtime URL must be an exact loopback HTTP origin')
  }
  if (
    parsed.protocol !== 'http:'
    || parsed.hostname !== '127.0.0.1'
    || !parsed.port
    || parsed.username
    || parsed.password
    || parsed.pathname !== '/'
    || parsed.search
    || parsed.hash
  ) {
    throw new Error('runtime URL must be an exact loopback HTTP origin')
  }
  return parsed.origin
}

async function boundedJson(response) {
  const declared = Number(response.headers.get('content-length'))
  if (Number.isFinite(declared) && declared > MAX_RESPONSE_BYTES) {
    throw new Error('runtime response exceeded its bound')
  }
  if (!response.body) throw new Error('runtime response was malformed')
  const reader = response.body.getReader()
  const chunks = []
  let total = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    total += value.byteLength
    if (total > MAX_RESPONSE_BYTES) {
      await reader.cancel()
      throw new Error('runtime response exceeded its bound')
    }
    chunks.push(Buffer.from(value))
  }
  try {
    return JSON.parse(Buffer.concat(chunks, total).toString('utf8'))
  } catch {
    throw new Error('runtime response was malformed')
  }
}

function validateEvents(events) {
  return Array.isArray(events)
    && events.length <= 1024
    && events.every((event) => exactKeys(event, ['event', 'payload'])
      && typeof event.event === 'string'
      && event.event.length > 0
      && event.event.length <= 128)
}

export function createRuntimeClient({ baseUrl, token, profile, device }) {
  const origin = validateBaseUrl(baseUrl)
  if (!TOKEN_PATTERN.test(token ?? '')) throw new Error('runtime token is invalid')
  if (!PROFILES.has(profile)) throw new Error('runtime profile is invalid')
  if (!DEVICES.has(device)) throw new Error('runtime device is invalid')

  return Object.freeze({
    async invoke(command, args = {}) {
      if (!COMMAND_PATTERN.test(command ?? '')) throw new Error('runtime command is invalid')
      if (!args || typeof args !== 'object' || Array.isArray(args)) {
        throw new Error('runtime arguments must be an object')
      }

      let response
      try {
        response = await fetch(`${origin}/invoke`, {
          method: 'POST',
          headers: {
            Accept: 'application/json',
            Authorization: `Bearer ${token}`,
            'Content-Type': 'application/json',
            Origin: ORIGIN,
            'X-Grafyn-E2E-Profile': profile,
            'X-Grafyn-E2E-Device': device,
          },
          body: JSON.stringify({ command, args }),
          credentials: 'omit',
          redirect: 'error',
          signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
        })
      } catch {
        throw new Error('runtime request failed')
      }

      const envelope = await boundedJson(response)
      if (response.status === 200) {
        if (!exactKeys(envelope, ['result', 'events']) || !validateEvents(envelope.events)) {
          throw new Error('runtime success response was malformed')
        }
        return envelope.result
      }
      if (
        response.status === 400
        && exactKeys(envelope, ['error'])
        && exactKeys(envelope.error, ['code', 'message'])
        && typeof envelope.error.code === 'string'
        && envelope.error.code.length > 0
        && envelope.error.code.length <= 80
        && typeof envelope.error.message === 'string'
        && envelope.error.message.length > 0
        && envelope.error.message.length <= 512
      ) {
        throw new RuntimeInvokeError(envelope.error.code, envelope.error.message)
      }
      throw new Error('runtime request was rejected')
    },
  })
}

export function runtimeClientFor(device, profile = 'android') {
  return createRuntimeClient({
    baseUrl: process.env.GRAFYN_E2E_RUNTIME_URL ?? 'http://127.0.0.1:18890',
    token: process.env.GRAFYN_E2E_RUNTIME_TOKEN,
    profile,
    device,
  })
}
