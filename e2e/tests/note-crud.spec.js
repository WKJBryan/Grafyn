/**
 * E2E Tests: Note CRUD Operations
 *
 * Tests the core note create, read, update, delete workflows.
 * Updated for graph-first UI with TopicSelector, TreeNav, editor overlay, and ConfirmDialog.
 */

import { test, expect } from '@playwright/test'
import {
  generateNoteTitle,
  waitForAppReady,
  createNote,
  createNoteViaAPI,
  selectNote,
  closeEditorOverlay,
  completeTopicSelector,
  clearAllNotes,
} from './fixtures/test-helpers.js'

test.describe('Note CRUD Operations', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test.describe('Create Note', () => {
    test('should create a new note with title and content', async ({ page }) => {
      const title = generateNoteTitle()
      const content = 'Test content for new note'

      await createNote(page, { title, content })

      // Verify note appears in the TreeNav
      await expect(page.locator('.tree-nav .nav-item', { hasText: title })).toBeVisible()
    })

    test('should show editor overlay after creating note', async ({ page }) => {
      const title = generateNoteTitle()

      await createNote(page, { title, content: 'Some content' })

      // Editor overlay should be visible with the title
      await expect(page.locator('.editor-panel-overlay')).toBeVisible()
      await expect(page.locator('.editor-panel-overlay .title-input')).toHaveValue(title)
    })

    test('should create note with tags', async ({ page }) => {
      const title = generateNoteTitle()

      await createNote(page, { title, content: 'Content with tags', tags: ['tag1', 'tag2', 'tag3'] })

      // Verify tags are in the editor footer
      await expect(page.locator('.editor-panel-overlay .tags-input')).toHaveValue('tag1, tag2, tag3')
    })

    test('should set note status', async ({ page }) => {
      const title = generateNoteTitle()

      await createNote(page, { title, content: 'Evidence content', status: 'evidence' })

      // Verify status dropdown value in editor
      await expect(page.locator('.editor-panel-overlay .status-select')).toHaveValue('evidence')
    })
  })

  test.describe('TopicSelector', () => {
    test('should show TopicSelector on New Note click', async ({ page }) => {
      await page.click('button:has-text("New Note")')

      await expect(page.locator('.topic-selector-overlay')).toBeVisible()
      await expect(page.locator('.topic-selector')).toContainText('Select Note Topic')
    })

    test('should enable Create Note when skip checkbox is checked', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.waitForSelector('.topic-selector-overlay', { state: 'visible' })

      // Initially the Create Note button may be disabled
      const createBtn = page.locator('.selector-footer .btn-primary')

      // Check skip checkbox
      const skipCheckbox = page.locator('.no-topic-option input[type="checkbox"]')
      await skipCheckbox.check()

      await expect(createBtn).toBeEnabled()
    })

    test('should apply selected topic as tag', async ({ page }) => {
      const title = generateNoteTitle()

      await page.click('button:has-text("New Note")')
      await page.waitForSelector('.topic-selector-overlay', { state: 'visible' })

      // Type a topic name
      await page.fill('.new-topic-input input', 'test-topic')

      // Click Create Note
      await page.click('.selector-footer .btn-primary')
      await page.waitForSelector('.editor-panel-overlay', { state: 'visible' })

      // Fill in the note
      await page.fill('.editor-panel-overlay .title-input', title)
      await page.fill('.editor-panel-overlay .editor-textarea', 'Content')
      await page.click('.editor-panel-overlay button:has-text("Save")')

      await page.waitForResponse(resp =>
        resp.url().includes('/api/notes') && resp.status() === 200
      )
    })

    test('should cancel TopicSelector without creating', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.waitForSelector('.topic-selector-overlay', { state: 'visible' })

      // Click Cancel
      await page.click('.selector-footer .btn-ghost')

      // TopicSelector should close, no editor overlay
      await expect(page.locator('.topic-selector-overlay')).not.toBeVisible()
      await expect(page.locator('.editor-panel-overlay')).not.toBeVisible()
    })
  })

  test.describe('Read Note', () => {
    test('should display note content when selected from TreeNav', async ({ page, request }) => {
      const title = generateNoteTitle()
      const content = 'Readable content here'
      const baseURL = 'http://localhost:8080'

      // Create via API for speed
      await createNoteViaAPI(request, baseURL, { title, content })
      await page.reload()
      await waitForAppReady(page)

      // Select from TreeNav
      await selectNote(page, title)

      // Verify editor overlay shows the content
      await expect(page.locator('.editor-panel-overlay')).toBeVisible()
      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toHaveValue(content)
    })

    test('should show graph view as main content area', async ({ page }) => {
      await expect(page.locator('.full-graph-container')).toBeVisible()
    })

    test('should show empty state banner when no notes exist', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      await clearAllNotes(request, baseURL)
      await page.reload()
      await waitForAppReady(page)

      await expect(page.locator('.empty-state-banner')).toBeVisible()
      await expect(page.locator('.empty-state-banner')).toContainText('Welcome to Grafyn')
    })
  })

  test.describe('Update Note', () => {
    test('should update note title', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const originalTitle = generateNoteTitle()
      const updatedTitle = `Updated ${originalTitle}`

      await createNoteViaAPI(request, baseURL, { title: originalTitle, content: 'Original content' })
      await page.reload()
      await waitForAppReady(page)

      // Select and update
      await selectNote(page, originalTitle)
      await page.fill('.editor-panel-overlay .title-input', updatedTitle)
      await page.click('.editor-panel-overlay button:has-text("Save")')

      await page.waitForResponse(resp =>
        resp.url().includes('/api/notes') && resp.status() === 200
      )

      // Verify update in TreeNav
      await expect(page.locator('.tree-nav')).toContainText(updatedTitle)
    })

    test('should update note content', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const title = generateNoteTitle()
      const updatedContent = 'Updated content with changes'

      await createNoteViaAPI(request, baseURL, { title, content: 'Original content' })
      await page.reload()
      await waitForAppReady(page)

      // Update content
      await selectNote(page, title)
      await page.fill('.editor-panel-overlay .editor-textarea', updatedContent)
      await page.click('.editor-panel-overlay button:has-text("Save")')

      await page.waitForResponse(resp =>
        resp.url().includes('/api/notes') && resp.status() === 200
      )

      // Close and reopen to verify persistence
      await closeEditorOverlay(page)
      await selectNote(page, title)
      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toHaveValue(updatedContent)
    })

    test('should show Save button disabled when no changes', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const title = generateNoteTitle()

      await createNoteViaAPI(request, baseURL, { title, content: 'Content' })
      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, title)

      // Save should be disabled (not dirty)
      await expect(page.locator('.editor-panel-overlay button:has-text("Save")')).toBeDisabled()
    })
  })

  test.describe('Delete Note', () => {
    test('should delete note via ConfirmDialog', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const title = generateNoteTitle()

      await createNoteViaAPI(request, baseURL, { title, content: 'Content to delete' })
      await page.reload()
      await waitForAppReady(page)

      // Verify note exists in tree
      await expect(page.locator('.tree-nav')).toContainText(title)

      // Select and delete
      await selectNote(page, title)
      await page.click('.editor-panel-overlay button:has-text("Delete")')

      // ConfirmDialog should appear
      await expect(page.locator('.confirm-dialog')).toBeVisible()
      await expect(page.locator('.confirm-dialog')).toContainText('Delete Note')

      // Confirm deletion
      await page.click('.confirm-dialog .btn-danger')

      await page.waitForResponse(resp =>
        resp.url().includes('/api/notes') && resp.request().method() === 'DELETE'
      )

      // Note should be removed from TreeNav
      await expect(page.locator('.tree-nav')).not.toContainText(title)
    })

    test('should cancel delete via ConfirmDialog', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const title = generateNoteTitle()

      await createNoteViaAPI(request, baseURL, { title, content: 'Content to keep' })
      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, title)
      await page.click('.editor-panel-overlay button:has-text("Delete")')

      // Cancel in ConfirmDialog
      await expect(page.locator('.confirm-dialog')).toBeVisible()
      await page.click('.confirm-dialog .btn-secondary')

      // Dialog should close, note should still exist
      await expect(page.locator('.confirm-dialog')).not.toBeVisible()
      await expect(page.locator('.tree-nav')).toContainText(title)
    })

    test('should close editor overlay after delete', async ({ page, request }) => {
      const baseURL = 'http://localhost:8080'
      const title = generateNoteTitle()

      await createNoteViaAPI(request, baseURL, { title, content: 'Content' })
      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, title)
      await page.click('.editor-panel-overlay button:has-text("Delete")')
      await page.click('.confirm-dialog .btn-danger')

      await page.waitForResponse(resp =>
        resp.url().includes('/api/notes') && resp.request().method() === 'DELETE'
      )

      // Editor overlay should close
      await expect(page.locator('.editor-panel-overlay')).not.toBeVisible()
    })
  })
})
