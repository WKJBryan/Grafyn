/**
 * E2E Tests: Navigation and Layout
 *
 * Tests app navigation, layout, and routing.
 * Updated for graph-first UI with sidebar-left, sidebar-right, TreeNav, and editor overlay.
 */

import { test, expect } from '@playwright/test'
import {
  waitForAppReady,
  createNoteViaAPI,
  selectNote,
  clearAllNotes,
  closeEditorOverlay,
} from './fixtures/test-helpers.js'

const BASE_URL = 'http://localhost:8080'

test.describe('Navigation and Layout', () => {
  test.describe('Main Layout', () => {
    test('should display header with logo', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.logo')).toBeVisible()
    })

    test('should display left sidebar with TreeNav', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.sidebar-left')).toBeVisible()
    })

    test('should display main content with graph container', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.main-content')).toBeVisible()
      await expect(page.locator('.full-graph-container')).toBeVisible()
    })

    test('should display right sidebar', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.sidebar-right')).toBeVisible()
    })

    test('should display New Note button', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('button:has-text("New Note")')).toBeVisible()
    })

    test('should display Canvas button', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('button:has-text("Canvas")')).toBeVisible()
    })

    test('should display Settings button', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // Settings button uses gear emoji
      await expect(page.locator('.action-buttons .btn-ghost')).toBeVisible()
    })
  })

  test.describe('TreeNav Navigation', () => {
    test.beforeEach(async ({ page, request }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // Create test notes via API for speed
      for (let i = 1; i <= 3; i++) {
        await createNoteViaAPI(request, BASE_URL, {
          title: `Nav Test Note ${i}`,
          content: `Content for note ${i}`,
        })
      }
      await page.reload()
      await waitForAppReady(page)
    })

    test('should select note on click in TreeNav', async ({ page }) => {
      await page.click('.tree-nav .nav-item:has-text("Nav Test Note 2")')

      await expect(page.locator('.tree-nav .nav-item.active')).toContainText('Nav Test Note 2')
    })

    test('should display selected note in editor overlay', async ({ page }) => {
      await selectNote(page, 'Nav Test Note 1')

      await expect(page.locator('.editor-panel-overlay')).toBeVisible()
      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toContainText('Content for note 1')
    })

    test('should update active state when clicking different notes', async ({ page }) => {
      // Click first note
      await selectNote(page, 'Nav Test Note 1')
      await expect(page.locator('.tree-nav .nav-item.active')).toContainText('Nav Test Note 1')

      // Close overlay, click second note
      await closeEditorOverlay(page)
      await selectNote(page, 'Nav Test Note 2')
      await expect(page.locator('.tree-nav .nav-item.active')).toContainText('Nav Test Note 2')
    })

    test('should close editor overlay with close button', async ({ page }) => {
      await selectNote(page, 'Nav Test Note 1')
      await expect(page.locator('.editor-panel-overlay')).toBeVisible()

      await closeEditorOverlay(page)
      await expect(page.locator('.editor-panel-overlay')).not.toBeVisible()
    })
  })

  test.describe('Backlinks Panel', () => {
    test.beforeEach(async ({ page, request }) => {
      await page.goto('/')
      await waitForAppReady(page)

      // Create notes with wikilinks via API
      await createNoteViaAPI(request, BASE_URL, {
        title: 'Target Note',
        content: 'This is the target note content',
      })
      await createNoteViaAPI(request, BASE_URL, {
        title: 'Linking Note',
        content: 'This links to [[Target Note]]',
      })

      await page.reload()
      await waitForAppReady(page)
    })

    test('should show backlinks section in right sidebar', async ({ page }) => {
      await expect(page.locator('.sidebar-right')).toBeVisible()
      await expect(page.locator('.sidebar-right')).toContainText('Backlinks')
    })

    test('should display backlinks heading', async ({ page }) => {
      const backlinksSection = page.locator('.sidebar-right .section-title', { hasText: 'Backlinks' })
      await expect(backlinksSection).toBeVisible()
    })
  })

  test.describe('Empty States', () => {
    test('should show empty state banner when no notes exist', async ({ page, request }) => {
      await clearAllNotes(request, BASE_URL)
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.empty-state-banner')).toBeVisible()
      await expect(page.locator('.empty-state-banner')).toContainText('Welcome to Grafyn')
    })

    test('should show Create Your First Note button in empty state', async ({ page, request }) => {
      await clearAllNotes(request, BASE_URL)
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.empty-state-banner button:has-text("Create Your First Note")')).toBeVisible()
    })
  })

  test.describe('Canvas Navigation', () => {
    test('should navigate to canvas view', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await page.click('button:has-text("Canvas")')

      await expect(page).toHaveURL(/\/canvas/)
      await expect(page.locator('.canvas-view')).toBeVisible()
    })

    test('should navigate back to notes from canvas', async ({ page }) => {
      await page.goto('/canvas')
      await page.waitForSelector('.canvas-view', { state: 'visible' })

      await page.click('.back-link')

      await expect(page).toHaveURL('/')
      await expect(page.locator('.home-view')).toBeVisible()
    })
  })

  test.describe('Responsive Behavior', () => {
    test('should work on tablet viewport', async ({ page }) => {
      await page.setViewportSize({ width: 768, height: 1024 })
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.home-view')).toBeVisible()
      await expect(page.locator('.sidebar-left')).toBeVisible()
    })

    test('should work on mobile viewport', async ({ page }) => {
      await page.setViewportSize({ width: 375, height: 667 })
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('.home-view')).toBeVisible()
    })
  })

  test.describe('Page Refresh', () => {
    test('should persist notes after refresh', async ({ page, request }) => {
      const title = `Persist Test ${Date.now()}`

      await createNoteViaAPI(request, BASE_URL, { title, content: 'Persisted content' })
      await page.goto('/')
      await waitForAppReady(page)

      // Note should be in TreeNav
      await expect(page.locator('.tree-nav')).toContainText(title)

      // Refresh and check again
      await page.reload()
      await waitForAppReady(page)

      await expect(page.locator('.tree-nav')).toContainText(title)
    })

    test('should maintain app state after navigation', async ({ page }) => {
      await page.goto('/')
      await waitForAppReady(page)

      await expect(page.locator('button:has-text("New Note")')).toBeEnabled()
    })
  })
})
