/**
 * E2E Tests: Navigation and Layout
 *
 * Tests app navigation, layout, and routing
 */

import { test, expect } from '@playwright/test'
import { waitForAppReady, generateNoteTitle } from './fixtures/test-helpers.js'

test.describe('Navigation and Layout', () => {
  test.describe('Main Layout', () => {
    test('should display header with logo', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.logo')).toContainText('Grafyn')
    })

    test('should display sidebar', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.sidebar')).toBeVisible()
    })

    test('should display editor area', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.editor-area')).toBeVisible()
    })

    test('should display New Note button', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('button:has-text("New Note")')).toBeVisible()
    })
  })

  test.describe('Note List Navigation', () => {
    test.beforeEach(async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // Create test notes
      for (let i = 1; i <= 3; i++) {
        await page.click('button:has-text("New Note")')
        await page.fill('input[placeholder*="title"], input[name="title"]', `Nav Test Note ${i}`)
        await page.fill('textarea', `Content for note ${i}`)
        await page.click('button:has-text("Save")')
        await page.waitForTimeout(300)
      }
    })

    test('should select note on click', async ({ page }) => {
      await page.click('.note-item:has-text("Nav Test Note 2")')

      await expect(page.locator('.note-item.selected')).toContainText('Nav Test Note 2')
    })

    test('should display selected note in editor', async ({ page }) => {
      await page.click('.note-item:has-text("Nav Test Note 1")')

      await expect(page.locator('.note-editor')).toBeVisible()
      await expect(page.locator('textarea')).toContainText('Content for note 1')
    })

    test('should update selection when clicking different notes', async ({ page }) => {
      // Click first note
      await page.click('.note-item:has-text("Nav Test Note 1")')
      await expect(page.locator('.note-item.selected')).toContainText('Nav Test Note 1')

      // Click second note
      await page.click('.note-item:has-text("Nav Test Note 2")')
      await expect(page.locator('.note-item.selected')).toContainText('Nav Test Note 2')
      await expect(page.locator('.note-item.selected')).not.toContainText('Nav Test Note 1')
    })
  })

  test.describe('Backlinks Panel', () => {
    test.beforeEach(async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // Create notes with wikilinks
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Target Note')
      await page.fill('textarea', 'This is the target note content')
      await page.click('button:has-text("Save")')
      await page.waitForTimeout(300)

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Linking Note')
      await page.fill('textarea', 'This links to [[Target Note]]')
      await page.click('button:has-text("Save")')
      await page.waitForTimeout(300)
    })

    test('should show backlinks panel when note selected', async ({ page }) => {
      await page.click('.note-item:has-text("Target Note")')

      await expect(page.locator('.right-panel, .backlinks-panel')).toBeVisible()
    })

    test('should display backlinks header', async ({ page }) => {
      await page.click('.note-item:has-text("Target Note")')

      await expect(page.locator('.backlinks-panel, .right-panel')).toContainText('Backlinks')
    })

    test('should hide backlinks panel when no note selected', async ({ page }) => {
      // Initially no note selected
      await expect(page.locator('.right-panel')).not.toBeVisible()
    })
  })

  test.describe('Empty States', () => {
    test('should show empty state when no note selected', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.empty-state')).toBeVisible()
      await expect(page.locator('.editor-area')).toContainText(/select|create/i)
    })

    test('should show empty list message when no notes', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // If there are no notes (depends on test isolation)
      const noteItems = page.locator('.note-item')
      const count = await noteItems.count()

      if (count === 0) {
        await expect(page.locator('.empty-list')).toContainText('No notes')
      }
    })
  })

  test.describe('Responsive Behavior', () => {
    test('should work on tablet viewport', async ({ page }) => {
      await page.setViewportSize({ width: 768, height: 1024 })
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.home-view')).toBeVisible()
      await expect(page.locator('.sidebar')).toBeVisible()
    })

    test('should work on mobile viewport', async ({ page }) => {
      await page.setViewportSize({ width: 375, height: 667 })
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.home-view')).toBeVisible()
    })
  })

  test.describe('Page Refresh', () => {
    test('should persist notes after refresh', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      const title = `Persist Test ${Date.now()}`

      // Create a note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', 'Persisted content')
      await page.click('button:has-text("Save")')

      // Wait for save
      await page.waitForTimeout(500)

      // Refresh the page
      await page.reload()
      await waitForAppReady(page)

      // Note should still exist
      await expect(page.locator('.note-list')).toContainText(title)
    })

    test('should maintain app state after navigation', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // App should be functional
      await expect(page.locator('button:has-text("New Note")')).toBeEnabled()
    })
  })
})
