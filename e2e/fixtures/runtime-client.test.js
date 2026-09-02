import assert from 'node:assert/strict'
import http from 'node:http'
import test from 'node:test'

import { createRuntimeClient, RuntimeInvokeError } from './runtime-client.js'

const TOKEN = 'b'.repeat(64)

async function fixture(handler) {
  const server = http.createServer(handler)
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen({ host: '127.0.0.1', port: 0 }, resolve)
  })
  const address = server.address()
  return {
    url: `http://127.0.0.1:${address.port}`,
    close: () => new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  }
}

test('sends the strict runtime contract and returns only the invocation result', async (t) => {
  const originalTimeout = AbortSignal.timeout
  let requestedTimeout
  AbortSignal.timeout = (milliseconds) => {
    requestedTimeout = milliseconds
    return new AbortController().signal
  }
  t.after(() => { AbortSignal.timeout = originalTimeout })
  const server = await fixture((request, response) => {
    assert.equal(request.method, 'POST')
    assert.equal(request.url, '/invoke')
    assert.equal(request.headers.authorization, `Bearer ${TOKEN}`)
    assert.equal(request.headers.origin, 'http://127.0.0.1:5173')
    assert.equal(request.headers['x-grafyn-e2e-profile'], 'android')
    assert.equal(request.headers['x-grafyn-e2e-device'], 'device-b')
    let body = ''
    request.setEncoding('utf8')
    request.on('data', (chunk) => { body += chunk })
    request.on('end', () => {
      assert.deepEqual(JSON.parse(body), { command: 'list_notes', args: {} })
      response.writeHead(200, { 'Content-Type': 'application/json' })
      response.end(JSON.stringify({
        result: [{ id: 'note-1' }],
        events: [{ event: 'notes-changed', payload: { id: 'note-1' } }],
      }))
    })
  })
  t.after(() => server.close())

  const client = createRuntimeClient({
    baseUrl: server.url,
    token: TOKEN,
    profile: 'android',
    device: 'device-b',
  })
  assert.deepEqual(await client.invoke('list_notes', {}), [{ id: 'note-1' }])
  assert.equal(requestedTimeout, 180_000)
})

test('surfaces bounded command failures without exposing transport internals', async (t) => {
  const server = await fixture((_request, response) => {
    response.writeHead(400, { 'Content-Type': 'application/json' })
    response.end(JSON.stringify({
      error: { code: 'invoke_failed', message: 'encrypted sync bundle could not be imported' },
    }))
  })
  t.after(() => server.close())

  const client = createRuntimeClient({
    baseUrl: server.url,
    token: TOKEN,
    profile: 'desktop',
    device: 'device-a',
  })
  await assert.rejects(
    client.invoke('import_sync_envelopes', { bundle: { schemaVersion: 1, envelopes: [] } }),
    (error) => error instanceof RuntimeInvokeError
      && error.code === 'invoke_failed'
      && error.message === 'encrypted sync bundle could not be imported',
  )
})

test('rejects unsafe configuration and malformed success envelopes', async (t) => {
  assert.throws(
    () => createRuntimeClient({
      baseUrl: 'https://example.com',
      token: TOKEN,
      profile: 'android',
      device: 'device-a',
    }),
    /loopback/,
  )

  const server = await fixture((_request, response) => {
    response.writeHead(200, { 'Content-Type': 'application/json' })
    response.end(JSON.stringify({ result: true, events: [], extra: true }))
  })
  t.after(() => server.close())
  const client = createRuntimeClient({
    baseUrl: server.url,
    token: TOKEN,
    profile: 'android',
    device: 'device-a',
  })
  await assert.rejects(client.invoke('get_boot_status', {}), /malformed/)
})
