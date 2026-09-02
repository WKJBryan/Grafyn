import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import test from 'node:test'

import { collectBrowserErrors, waitForInvokeResponse } from './browser-events.js'

test('collects every browser console error and uncaught page error from registration onward', () => {
  const page = new EventEmitter()
  const errors = collectBrowserErrors(page)

  page.emit('console', { type: () => 'log', text: () => 'ordinary log' })
  page.emit('console', { type: () => 'error', text: () => 'panel failed' })
  page.emit('pageerror', new Error('uncaught render failure'))

  assert.deepEqual(errors, [
    'console: panel failed',
    'pageerror: uncaught render failure',
  ])
})

test('waits only for the named command on the invoke endpoint', async () => {
  let predicate
  const page = {
    waitForResponse: (candidate) => {
      predicate = candidate
      return Promise.resolve('matched response')
    },
  }

  assert.equal(await waitForInvokeResponse(page, 'regenerate_response'), 'matched response')
  const responseFor = ({ method = 'POST', url = 'http://127.0.0.1:18890/invoke', body }) => ({
    url: () => url,
    request: () => ({
      method: () => method,
      postDataJSON: () => body,
    }),
  })

  assert.equal(predicate(responseFor({ body: { command: 'send_prompt' } })), false)
  assert.equal(predicate(responseFor({ method: 'GET', body: { command: 'regenerate_response' } })), false)
  assert.equal(predicate(responseFor({
    url: 'http://127.0.0.1:18890/not-invoke',
    body: { command: 'regenerate_response' },
  })), false)
  assert.equal(predicate(responseFor({ body: { command: 'regenerate_response' } })), true)
})
