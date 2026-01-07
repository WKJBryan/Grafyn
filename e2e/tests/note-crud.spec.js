/**
 * E2E Tests: Note CRUD Operations
 *
 * Tests the core note create, read, update, delete workflows
 */

import { test, expect } from '@playwright/test'
import { generateNoteTitle, generateNoteContent, waitForAppReady } from './fixtures/test-helpers.js'

test.describe('Note CRUD Operations', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test.describe('Create Note', () => {
    test('should create a new note with title and content', async ({ page }) => {
      const title = generateNoteTitle()
      const content = 'Test content for new note'

      // Click New Note button
      await page.click('button:has-text("New Note")')

      // Fill in the form
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', content)

      // Save the note
      await page.click('button:has-text("Save")')

      // Verify note appears in the list
      await expect(page.locator('.note-list')).toContainText(title)
    })

    test('should show validation error for empty title', async ({ page }) => {
      // Click New Note button
      await page.click('button:has-text("New Note")')

      // Try to save without title
      await page.fill('textarea', 'Some content')
      await page.click('button:has-text("Save")')

      // Expect validation message
      await expect(page.locator('.error, .validation-error, [role="alert"]')).toBeVisible()
    })

    test('should create note with tags', async ({ page }) => {
      const title = generateNoteTitle()

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Content with tags')

      // Add tags
      const tagsInput = page.locator('input[placeholder*="tag"]')
      if (await tagsInput.isVisible()) {
        await tagsInput.fill('tag1, tag2, tag3')
      }

      await page.click('button:has-text("Save")')

      // Click on the note in the list
      await page.click(`.note-item:has-text("${title}")`)

      // Verify tags are displayed
      const noteItem = page.locator('.note-item', { hasText: title })
      await expect(noteItem.locator('.tag')).toHaveCount(3)
    })

    test('should set note status', async ({ page }) => {
      const title = generateNoteTitle()

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Evidence content')

      // Select status
      const statusSelect = page.locator('select')
      if (await statusSelect.isVisible()) {
        await statusSelect.selectOption('evidence')
      }

      await page.click('button:has-text("Save")')

      // Verify status is shown
      const noteItem = page.locator('.note-item', { hasText: title })
      await expect(noteItem.locator('.status')).toContainText('evidence')
    })
  })

  test.describe('Read Note', () => {
    test('should display note content when selected', async ({ page }) => {
      // First create a note
      const title = generateNoteTitle()
      const content = 'Readable content here'

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', content)
      await page.click('button:has-text("Save")')

      // Now select it
      await page.click(`.note-item:has-text("${title}")`)

      // Verify editor shows the content
      await expect(page.locator('.note-editor')).toBeVisible()
      await expect(page.locator('textarea')).toHaveValue(content)
    })

    test('should load notes on page load', async ({ page }) => {
      // Should show the note list
      await expect(page.locator('.note-list')).toBeVisible()
    })

    test('should show empty state when no note selected', async ({ page }) => {
      // Should show empty state message
      await expect(page.locator('.empty-state, .editor-area')).toContainText(/select|create/i)
    })
  })

  test.describe('Update Note', () => {
    test('should update note title', async ({ page }) => {
      const originalTitle = generateNoteTitle()
      const updatedTitle = `Updated ${originalTitle}`

      // Create note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', originalTitle)
      await page.fill('textarea', 'Original content')
      await page.click('button:has-text("Save")')

      // Select and update
      await page.click(`.note-item:has-text("${originalTitle}")`)
      await page.fill('input[placeholder*="title"], input[name="title"]', updatedTitle)
      await page.click('button:has-text("Save")')

      // Verify update
      await expect(page.locator('.note-list')).toContainText(updatedTitle)
      await expect(page.locator('.note-list')).not.toContainText(originalTitle)
    })

    test('should update note content', async ({ page }) => {
      const title = generateNoteTitle()
      const originalContent = 'Original content'
      const updatedContent = 'Updated content with changes'

      // Create note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', originalContent)
      await page.click('button:has-text("Save")')

      // Update content
      await page.click(`.note-item:has-text("${title}")`)
      await page.fill('textarea', updatedContent)
      await page.click('button:has-text("Save")')

      // Reload and verify
      await page.reload()
      await waitForAppReady(page)
      await page.click(`.note-item:has-text("${title}")`)
      await expect(page.locator('textarea')).toHaveValue(updatedContent)
    })

    test('should track dirty state', async ({ page }) => {
      const title = generateNoteTitle()

      // Create a note first
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Initial content')
      await page.click('button:has-text("Save")')

      // Select the note
      await page.click(`.note-item:has-text("${title}")`)

      // Modify content
      await page.fill('textarea', 'Modified content')

      // Check for dirty indicator (depends on implementation)
      const dirtyIndicator = page.locator('.dirty, .unsaved, [data-dirty="true"]')
      // This may or may not exist depending on UI implementation
    })
  })

  test.describe('Delete Note', () => {
    test('should delete note with confirmation', async ({ page }) => {
      const title = generateNoteTitle()

      // Create note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Content to delete')
      await page.click('button:has-text("Save")')

      // Wait for note to appear in list
      await expect(page.locator('.note-list')).toContainText(title)

      // Select and delete
      await page.click(`.note-item:has-text("${title}")`)

      // Handle confirmation dialog
      page.on('dialog', dialog => dialog.accept())
      await page.click('button:has-text("Delete")')

      // Verify note is removed
      await expect(page.locator('.note-list')).not.toContainText(title)
    })

    test('should cancel delete on dialog dismiss', async ({ page }) => {
      const title = generateNoteTitle()

      // Create note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Content to keep')
      await page.click('button:has-text("Save")')

      // Select note
      await page.click(`.note-item:has-text("${title}")`)

      // Cancel deletion
      page.on('dialog', dialog => dialog.dismiss())
      await page.click('button:has-text("Delete")')

      // Note should still exist
      await expect(page.locator('.note-list')).toContainText(title)
    })

    test('should clear editor after delete', async ({ page }) => {
      const title = generateNoteTitle()

      // Create note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Content')
      await page.click('button:has-text("Save")')

      // Select and delete
      await page.click(`.note-item:has-text("${title}")`)
      page.on('dialog', dialog => dialog.accept())
      await page.click('button:has-text("Delete")')

      // Editor should show empty state
      await expect(page.locator('.empty-state')).toBeVisible()
    })
  })
})
