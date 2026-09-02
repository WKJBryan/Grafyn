import assert from 'node:assert/strict'
import test from 'node:test'

test('Playwright config reuses one runtime token across worker reloads', async () => {
  const previous = process.env.GRAFYN_E2E_RUNTIME_TOKEN
  delete process.env.GRAFYN_E2E_RUNTIME_TOKEN
  try {
    await import(`../playwright.config.js?token-reload=first-${Date.now()}`)
    const first = process.env.GRAFYN_E2E_RUNTIME_TOKEN
    await import(`../playwright.config.js?token-reload=second-${Date.now()}`)
    assert.match(first, /^[a-f0-9]{64}$/)
    assert.equal(process.env.GRAFYN_E2E_RUNTIME_TOKEN, first)
  } finally {
    if (previous === undefined) delete process.env.GRAFYN_E2E_RUNTIME_TOKEN
    else process.env.GRAFYN_E2E_RUNTIME_TOKEN = previous
  }
})

test('Playwright config rejects an invalid inherited runtime token', async () => {
  const previous = process.env.GRAFYN_E2E_RUNTIME_TOKEN
  process.env.GRAFYN_E2E_RUNTIME_TOKEN = 'not-a-valid-token'
  try {
    await assert.rejects(
      import(`../playwright.config.js?invalid-token=${Date.now()}`),
      /runtime token must be 64 lowercase hexadecimal characters/,
    )
  } finally {
    if (previous === undefined) delete process.env.GRAFYN_E2E_RUNTIME_TOKEN
    else process.env.GRAFYN_E2E_RUNTIME_TOKEN = previous
  }
})
