import assert from 'node:assert/strict'
import test from 'node:test'

import { startOpenRouterStub } from './openrouter-stub.js'

const TOKEN = 'a'.repeat(64)
const API_KEY = 'grafyn-e2e-key'

async function jsonRequest(url, { method = 'GET', token, apiKey, body } = {}) {
  const headers = {}
  if (token) headers.Authorization = `Bearer ${token}`
  if (apiKey) headers.Authorization = `Bearer ${apiKey}`
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  return fetch(url, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
}

test('serves deterministic text and image responses without accepting missing credentials', async (t) => {
  const stub = await startOpenRouterStub({ port: 0, token: TOKEN, apiKey: API_KEY })
  t.after(() => stub.close())

  assert.equal((await fetch(`${stub.url}/health`)).status, 200)
  assert.equal((await fetch(`${stub.url}/api/v1/models`)).status, 401)

  const models = await jsonRequest(`${stub.url}/api/v1/models`, { apiKey: API_KEY })
  assert.equal(models.status, 200)
  assert.equal((await models.json()).data[0].id, 'grafyn/e2e-text')

  const image = await jsonRequest(`${stub.url}/api/v1/images`, {
    method: 'POST',
    apiKey: API_KEY,
    body: {
      model: 'grafyn/e2e-image',
      prompt: 'a blue memory garden',
      resolution: '1024x1024',
      aspect_ratio: '1:1',
      stream: false,
      provider: { only: ['grafyn-e2e'], allow_fallbacks: false },
    },
  })
  assert.equal(image.status, 200)
  const imageBody = await image.json()
  assert.equal(imageBody.data.length, 1)
  assert.equal(imageBody.data[0].media_type, 'image/png')
  assert.ok(Buffer.from(imageBody.data[0].b64_json, 'base64').length > 40)

  const stream = await jsonRequest(`${stub.url}/api/v1/chat/completions`, {
    method: 'POST',
    apiKey: API_KEY,
    body: {
      model: 'grafyn/e2e-text',
      stream: true,
      messages: [
        { role: 'user', content: 'first memory' },
        { role: 'assistant', content: 'first answer' },
        { role: 'user', content: 'what do you remember?' },
      ],
    },
  })
  assert.equal(stream.status, 200)
  const streamBody = await stream.text()
  assert.match(stream.headers.get('content-type'), /^text\/event-stream/)
  assert.match(streamBody, /History-aware/)
  assert.match(streamBody, /prior turns: 2/)
  assert.match(streamBody, /candidate memory: excluded/i)
  assert.match(streamBody, /data: \[DONE\]/)

  const reviewed = await jsonRequest(`${stub.url}/api/v1/chat/completions`, {
    method: 'POST',
    apiKey: API_KEY,
    body: {
      model: 'grafyn/e2e-text',
      stream: false,
      messages: [{
        role: 'system',
        content: '- [Preference; Endorsed] owner prefers answers that include concrete implementation details such as files, commands, tests, or code.',
      }],
    },
  })
  assert.equal(reviewed.status, 200)
  assert.match((await reviewed.json()).choices[0].message.content, /candidate memory: included/i)

  const unrelated = await jsonRequest(`${stub.url}/api/v1/chat/completions`, {
    method: 'POST',
    apiKey: API_KEY,
    body: {
      model: 'grafyn/e2e-text',
      stream: false,
      messages: [{
        role: 'system',
        content: '- [Preference; Endorsed] owner prefers fast answers without implementation evidence.',
      }],
    },
  })
  assert.equal(unrelated.status, 200)
  assert.match((await unrelated.json()).choices[0].message.content, /candidate memory: excluded/i)
})

test('control is token-authenticated and makes every paid boundary fail offline', async (t) => {
  const stub = await startOpenRouterStub({ port: 0, token: TOKEN, apiKey: API_KEY })
  t.after(() => stub.close())

  const unauthenticated = await jsonRequest(`${stub.url}/__control`, {
    method: 'POST',
    body: { online: false },
  })
  assert.equal(unauthenticated.status, 401)

  const control = await jsonRequest(`${stub.url}/__control`, {
    method: 'POST',
    token: TOKEN,
    body: { online: false },
  })
  assert.equal(control.status, 200)

  const unavailable = await jsonRequest(`${stub.url}/api/v1/models`, { apiKey: API_KEY })
  assert.equal(unavailable.status, 503)

  const health = await fetch(`${stub.url}/health`)
  assert.deepEqual(await health.json(), { ok: true, online: false })
})

test('rejects oversized and malformed control bodies without recording secrets', async (t) => {
  const stub = await startOpenRouterStub({ port: 0, token: TOKEN, apiKey: API_KEY })
  t.after(() => stub.close())

  const oversized = await fetch(`${stub.url}/__control`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${TOKEN}`,
      'Content-Type': 'application/json',
    },
    body: 'x'.repeat(1024 * 1024 + 1),
  })
  assert.equal(oversized.status, 413)

  const requests = await jsonRequest(`${stub.url}/__requests`, { token: TOKEN })
  assert.equal(requests.status, 200)
  const serialized = JSON.stringify(await requests.json())
  assert.doesNotMatch(serialized, new RegExp(TOKEN))
  assert.doesNotMatch(serialized, new RegExp(API_KEY))
})
