import http from 'node:http'
import process from 'node:process'
import { deflateSync } from 'node:zlib'
import { pathToFileURL } from 'node:url'

const HOST = '127.0.0.1'
const DEFAULT_PORT = 18891
const MAX_BODY_BYTES = 1024 * 1024
const MAX_REQUEST_LOG = 64
const DEFAULT_API_KEY = 'grafyn-e2e-key'

function crc32(bytes) {
  let crc = 0xffffffff
  for (const byte of bytes) {
    crc ^= byte
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1))
    }
  }
  return (crc ^ 0xffffffff) >>> 0
}

function pngChunk(type, data) {
  const typeBytes = Buffer.from(type, 'ascii')
  const payload = Buffer.concat([typeBytes, data])
  const output = Buffer.allocUnsafe(data.length + 12)
  output.writeUInt32BE(data.length, 0)
  typeBytes.copy(output, 4)
  data.copy(output, 8)
  output.writeUInt32BE(crc32(payload), output.length - 4)
  return output
}

function deterministicPng() {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])
  const header = Buffer.alloc(13)
  header.writeUInt32BE(2, 0)
  header.writeUInt32BE(2, 4)
  header.set([8, 6, 0, 0, 0], 8)
  const pixels = Buffer.from([
    0, 28, 79, 214, 255, 91, 192, 235, 255,
    0, 250, 204, 21, 255, 28, 79, 214, 255,
  ])
  return Buffer.concat([
    signature,
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(pixels)),
    pngChunk('IEND', Buffer.alloc(0)),
  ])
}

const IMAGE_BASE64 = deterministicPng().toString('base64')

const imageCatalog = {
  data: [
    {
      id: 'grafyn/e2e-image',
      name: 'Grafyn E2E Image',
      description: 'Deterministic local raster fixture',
      created: 1800000000,
      architecture: { input_modalities: ['text'], output_modalities: ['image'] },
      supported_parameters: {
        resolution: { type: 'enum', values: ['1024x1024'] },
        aspect_ratio: { type: 'enum', values: ['1:1'] },
        n: { type: 'range', min: 1, max: 1 },
        output_format: { type: 'enum', values: ['png'] },
      },
      supports_streaming: false,
      endpoints: '/api/v1/images/models/grafyn/e2e-image/endpoints',
    },
  ],
}

const imageEndpoints = {
  id: 'grafyn/e2e-image',
  endpoints: [
    {
      provider_name: 'Grafyn E2E',
      provider_slug: 'grafyn-e2e',
      provider_tag: 'grafyn-e2e',
      supported_parameters: {
        resolution: { type: 'enum', values: ['1024x1024'] },
        aspect_ratio: { type: 'enum', values: ['1:1'] },
        n: { type: 'range', min: 1, max: 1 },
        output_format: { type: 'enum', values: ['png'] },
      },
      allowed_passthrough_parameters: [],
      supports_streaming: false,
      pricing: [{ billable: 'output_image', unit: 'image', cost_usd: 0.0042 }],
    },
  ],
}

function sendJson(response, status, body) {
  response.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
  })
  response.end(JSON.stringify(body))
}

function bearer(request) {
  const value = request.headers.authorization
  return typeof value === 'string' && value.startsWith('Bearer ') ? value.slice(7) : null
}

function readJson(request) {
  return new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    let exceeded = false
    request.on('data', (chunk) => {
      size += chunk.length
      if (size > MAX_BODY_BYTES) {
        exceeded = true
        return
      }
      chunks.push(chunk)
    })
    request.on('end', () => {
      if (exceeded) {
        reject(Object.assign(new Error('request body too large'), { status: 413 }))
        return
      }
      try {
        resolve(chunks.length === 0 ? {} : JSON.parse(Buffer.concat(chunks).toString('utf8')))
      } catch {
        reject(Object.assign(new Error('request body is not valid JSON'), { status: 400 }))
      }
    })
    request.on('error', reject)
  })
}

function historyAwareText(messages) {
  const safeMessages = Array.isArray(messages) ? messages : []
  const lastUser = [...safeMessages].reverse().find((message) => message?.role === 'user')
  const conversation = safeMessages.filter((message) => (
    message?.role === 'user' || message?.role === 'assistant'
  ))
  const currentUserIndex = conversation.findLastIndex((message) => message.role === 'user')
  let pendingUser = false
  let priorTurns = 0
  for (const message of conversation.slice(0, Math.max(0, currentUserIndex))) {
    if (message.role === 'user') {
      pendingUser = true
    } else if (pendingUser) {
      priorTurns += 2
      pendingUser = false
    }
  }
  const question = typeof lastUser?.content === 'string' ? lastUser.content.slice(0, 160) : 'your memory'
  const combinedContext = safeMessages
    .map((message) => typeof message?.content === 'string' ? message.content : '')
    .join('\n')
  const candidateMemory = combinedContext.includes(
    'owner prefers answers that include concrete implementation details such as files, commands, tests, or code.',
  ) ? 'included' : 'excluded'
  return `History-aware response; prior turns: ${priorTurns}; candidate memory: ${candidateMemory}. I connected “${question}” to the reviewed evidence in your twin.`
}

async function sendStream(response, text) {
  response.writeHead(200, {
    'Content-Type': 'text/event-stream; charset=utf-8',
    'Cache-Control': 'no-store',
    Connection: 'keep-alive',
  })
  const split = Math.max(1, Math.floor(text.length / 2))
  for (const content of [text.slice(0, split), text.slice(split)]) {
    response.write(`data: ${JSON.stringify({ choices: [{ delta: { content } }] })}\n\n`)
    await new Promise((resolve) => setImmediate(resolve))
  }
  response.write(`data: ${JSON.stringify({ choices: [], usage: { cost: 0.0001 } })}\n\n`)
  response.end('data: [DONE]\n\n')
}

export async function startOpenRouterStub({
  host = HOST,
  port = DEFAULT_PORT,
  token,
  apiKey = DEFAULT_API_KEY,
} = {}) {
  if (host !== HOST) throw new Error('OpenRouter stub must bind to 127.0.0.1')
  if (!/^[a-f0-9]{64}$/.test(token ?? '')) throw new Error('stub token must be 64 lowercase hex characters')
  if (!apiKey) throw new Error('stub API key must not be empty')

  let online = true
  const requests = []
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', `http://${HOST}`)
    requests.push({ method: request.method, path: url.pathname })
    if (requests.length > MAX_REQUEST_LOG) requests.shift()

    if (request.method === 'GET' && url.pathname === '/health') {
      sendJson(response, 200, { ok: true, online })
      return
    }

    if (url.pathname === '/__control' || url.pathname === '/__requests') {
      if (bearer(request) !== token) {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      if (request.method === 'GET' && url.pathname === '/__requests') {
        sendJson(response, 200, { requests: [...requests] })
        return
      }
      if (request.method !== 'POST' || url.pathname !== '/__control') {
        sendJson(response, 405, { error: 'method not allowed' })
        return
      }
      try {
        const body = await readJson(request)
        if (typeof body.online !== 'boolean' || Object.keys(body).some((key) => key !== 'online')) {
          sendJson(response, 400, { error: 'expected exactly one boolean online field' })
          return
        }
        online = body.online
        sendJson(response, 200, { ok: true, online })
      } catch (error) {
        sendJson(response, error.status ?? 400, { error: error.message })
      }
      return
    }

    if (!url.pathname.startsWith('/api/v1/')) {
      sendJson(response, 404, { error: 'not found' })
      return
    }
    if (bearer(request) !== apiKey) {
      sendJson(response, 401, { error: 'invalid API key' })
      return
    }
    if (!online) {
      sendJson(response, 503, { error: 'deterministic offline boundary' })
      return
    }

    if (request.method === 'GET' && url.pathname === '/api/v1/models') {
      sendJson(response, 200, {
        data: [
          {
            id: 'grafyn/e2e-text',
            name: 'Grafyn E2E Text',
            description: 'Deterministic local text fixture',
            context_length: 8192,
            pricing: { prompt: '0.000001', completion: '0.000002' },
          },
        ],
      })
      return
    }
    if (request.method === 'GET' && url.pathname === '/api/v1/images/models') {
      sendJson(response, 200, imageCatalog)
      return
    }
    if (
      request.method === 'GET' &&
      url.pathname === '/api/v1/images/models/grafyn/e2e-image/endpoints'
    ) {
      sendJson(response, 200, imageEndpoints)
      return
    }

    if (request.method !== 'POST') {
      sendJson(response, 405, { error: 'method not allowed' })
      return
    }
    try {
      const body = await readJson(request)
      if (url.pathname === '/api/v1/images') {
        if (
          body.model !== 'grafyn/e2e-image' ||
          body.resolution !== '1024x1024' ||
          body.aspect_ratio !== '1:1' ||
          body.stream !== false ||
          body.provider?.only?.[0] !== 'grafyn-e2e' ||
          body.provider?.allow_fallbacks !== false
        ) {
          sendJson(response, 400, { error: 'image request does not match the E2E capability' })
          return
        }
        sendJson(response, 200, {
          created: 1800000001,
          data: [{ b64_json: IMAGE_BASE64, media_type: 'image/png' }],
          usage: { cost: 0.0042 },
        })
        return
      }
      if (url.pathname === '/api/v1/chat/completions') {
        const text = historyAwareText(body.messages)
        if (body.stream === true) {
          await sendStream(response, text)
        } else {
          sendJson(response, 200, { choices: [{ message: { role: 'assistant', content: text } }] })
        }
        return
      }
      sendJson(response, 404, { error: 'not found' })
    } catch (error) {
      sendJson(response, error.status ?? 400, { error: error.message })
    }
  })

  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen({ host, port, exclusive: true }, resolve)
  })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('stub did not bind a TCP address')
  return {
    url: `http://${HOST}:${address.port}`,
    close: () => new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  }
}

const isMain = process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url
if (isMain) {
  const token = process.env.GRAFYN_E2E_RUNTIME_TOKEN
  const port = Number.parseInt(process.env.GRAFYN_E2E_OPENROUTER_PORT ?? `${DEFAULT_PORT}`, 10)
  const apiKey = process.env.GRAFYN_E2E_OPENROUTER_KEY ?? DEFAULT_API_KEY
  const stub = await startOpenRouterStub({ token, port, apiKey })
  process.stdout.write(`Grafyn OpenRouter stub listening at ${stub.url}\n`)
  const stop = async () => {
    await stub.close()
    process.exit(0)
  }
  process.once('SIGINT', stop)
  process.once('SIGTERM', stop)
}
