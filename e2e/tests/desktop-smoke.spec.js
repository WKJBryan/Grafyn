import { expect, test } from '@playwright/test'
import { collectBrowserErrors, waitForInvokeResponse } from '../fixtures/browser-events.js'
import { runtimeClientFor } from '../fixtures/runtime-client.js'

const NOTE_TITLE = 'Desktop E2E durable note'
const NOTE_BODY = 'Desktop search persistence marker 7f3a2e.'

async function waitForBoot(page) {
  await expect(page.locator('.startup-splash')).toHaveCount(0, { timeout: 30_000 })
}

async function fullNotes(client) {
  const metadata = await client.invoke('list_notes', {})
  return Promise.all(metadata.map(({ id }) => client.invoke('get_note', { id })))
}

async function settleInvokeResponses(page, responses) {
  const resolved = await Promise.all(responses)
  await Promise.all(resolved.map(response => response.finished()))
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(
    () => requestAnimationFrame(resolve),
  )))
}

test('desktop workspace keeps its wide graph, note, settings, Canvas, and import surfaces', async ({ page }) => {
  const browserErrors = collectBrowserErrors(page)
  const desktop = runtimeClientFor('device-a', 'desktop')
  const status = await desktop.invoke('get_runtime_status', {})
  expect(status).toMatchObject({
    schemaVersion: 1,
    runtime: 'desktop',
    capabilities: {
      notesRead: true,
      notesWrite: true,
      recall: true,
      twinReview: true,
      twinChat: true,
      linearCanvas: true,
      imageGeneration: true,
      spatialCanvas: true,
      nativeVaultPicker: true,
      importByPath: true,
      mcp: true,
      desktopUpdater: true,
    },
  })
  const mcpStatus = await desktop.invoke('get_mcp_status', {})
  expect(mcpStatus).toMatchObject({
    available: expect.any(Boolean),
    config_snippet: expect.stringContaining('mcpServers'),
  })
  expect(mcpStatus.binary_path === null || typeof mcpStatus.binary_path === 'string').toBe(true)

  const initialPanelResponses = [
    waitForInvokeResponse(page, 'list_link_suggestion_queue'),
    waitForInvokeResponse(page, 'get_link_discovery_status'),
    waitForInvokeResponse(page, 'get_vault_optimizer_inbox'),
  ]
  await page.goto('/')
  await waitForBoot(page)
  await settleInvokeResponses(page, initialPanelResponses)
  await expect(page.getByRole('heading', { name: 'Grafyn', exact: true })).toBeVisible()
  await expect(page.locator('.sidebar-left')).toBeVisible()
  await expect(page.locator('.full-graph-container')).toBeVisible()
  await expect(page.locator('.sidebar-right')).toBeVisible()

  let durableNote = (await fullNotes(desktop)).find(
    (note) => note.title === NOTE_TITLE && note.content === NOTE_BODY,
  )
  if (!durableNote) {
    await page.getByRole('button', { name: '+ New Note' }).click()
    await page.getByText('Skip - create without topic').click()
    await page.getByRole('button', { name: 'Create Note' }).click()
    await page.locator('.title-input').fill(NOTE_TITLE)
    await page.locator('.editor-textarea').fill(NOTE_BODY)
    await page.locator('.note-editor').getByRole('button', { name: 'Save', exact: true }).click()
  }

  await expect.poll(async () => {
    durableNote = (await fullNotes(desktop)).find(
      (note) => note.title === NOTE_TITLE && note.content === NOTE_BODY,
    )
    return Boolean(durableNote)
  }).toBe(true)
  const directSearch = await desktop.invoke('search_notes', { query: '7f3a2e', limit: 20 })
  expect(directSearch.some((result) => result.note?.id === durableNote.id)).toBe(true)

  await page.getByPlaceholder('Search notes...').fill('7f3a2e')
  const searchResult = page.locator('.search-result-item').filter({ hasText: NOTE_TITLE }).first()
  await expect(searchResult).toBeVisible()
  await searchResult.click()
  await expect(page.locator('.title-input')).toHaveValue(NOTE_TITLE)
  const backlinksPanel = page.locator('.backlinks-panel')
  await expect(backlinksPanel).toBeVisible()
  await expect(backlinksPanel.getByText('Loading...', { exact: true })).toHaveCount(0)
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(
    () => requestAnimationFrame(resolve),
  )))
  await expect(page.locator('.full-graph-container')).toBeVisible()

  await page.getByTitle('Settings').first().click()
  await expect(page.getByRole('heading', { name: /Settings/ })).toBeVisible()
  await expect(page.getByText('MCP Integration')).toBeVisible()
  await expect(page.locator('.mcp-status-badge')).toHaveText(
    mcpStatus.available ? 'Binary found' : 'Binary not found',
  )
  await expect(page.locator('.snippet-code')).toContainText('mcpServers')
  expect(await page.locator('.snippet-code').textContent()).toBe(mcpStatus.config_snippet)
  await expect(page.locator('.sync-status-card')).toBeVisible()
  await page.locator('.base-modal .close-btn').click()

  await page.goto('/canvas')
  await waitForBoot(page)
  await expect(page.locator('.canvas-view')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Multi-LLM Canvas' })).toBeVisible()

  await page.goto('/import')
  await waitForBoot(page)
  await expect(page.getByRole('heading', { name: 'Import Content' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Choose Export File' })).toBeDisabled()
  await expect(page.getByText('Capture', { exact: true })).toHaveCount(0)
  expect(browserErrors).toEqual([])
})
