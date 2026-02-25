/**
 * E2E Tests: Canvas View
 *
 * Tests canvas session management (no OpenRouter API key required).
 * Tests session CRUD, navigation, and UI layout.
 */

import { test, expect } from '@playwright/test'

test.describe('Canvas View', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/canvas')
    await page.waitForSelector('.canvas-view', { state: 'visible' })
  })

  test.describe('Layout', () => {
    test('should display canvas view with sidebar and main area', async ({ page }) => {
      await expect(page.locator('.canvas-view')).toBeVisible()
      await expect(page.locator('.canvas-sidebar')).toBeVisible()
      await expect(page.locator('.canvas-main')).toBeVisible()
    })

    test('should display sidebar header with Canvas Sessions title', async ({ page }) => {
      await expect(page.locator('.sidebar-header h2')).toContainText('Canvas Sessions')
    })

    test('should show empty state when no sessions exist', async ({ page }) => {
      // Main area shows "Multi-LLM Canvas" when no session is selected
      await expect(page.locator('.no-session')).toBeVisible()
      await expect(page.locator('.no-session-content h2')).toContainText('Multi-LLM Canvas')
    })

    test('should display Back to Notes link', async ({ page }) => {
      await expect(page.locator('.back-link')).toBeVisible()
      await expect(page.locator('.back-link')).toContainText('Back to Notes')
    })
  })

  test.describe('Session Management', () => {
    test('should open create session dialog on New button click', async ({ page }) => {
      await page.click('.sidebar-header .btn-primary')

      await expect(page.locator('.dialog-overlay')).toBeVisible()
      await expect(page.locator('.create-dialog')).toBeVisible()
      await expect(page.locator('.dialog-header h3')).toContainText('New Canvas Session')
    })

    test('should create a new session', async ({ page }) => {
      const sessionTitle = `Test Session ${Date.now()}`

      // Open dialog
      await page.click('.sidebar-header .btn-primary')
      await page.waitForSelector('.create-dialog', { state: 'visible' })

      // Fill title
      await page.fill('#sessionTitle', sessionTitle)

      // Click Create
      await page.click('.dialog-footer .btn-primary')

      // Session should appear in the list
      await expect(page.locator('.session-item', { hasText: sessionTitle })).toBeVisible()
    })

    test('should disable Create button when title is empty', async ({ page }) => {
      await page.click('.sidebar-header .btn-primary')
      await page.waitForSelector('.create-dialog', { state: 'visible' })

      // Clear the title input
      await page.fill('#sessionTitle', '')

      // Create button should be disabled
      await expect(page.locator('.dialog-footer .btn-primary')).toBeDisabled()
    })

    test('should cancel create dialog', async ({ page }) => {
      await page.click('.sidebar-header .btn-primary')
      await page.waitForSelector('.create-dialog', { state: 'visible' })

      // Click Cancel
      await page.click('.dialog-footer .btn-secondary')

      // Dialog should close
      await expect(page.locator('.dialog-overlay')).not.toBeVisible()
    })

    test('should close create dialog with X button', async ({ page }) => {
      await page.click('.sidebar-header .btn-primary')
      await page.waitForSelector('.create-dialog', { state: 'visible' })

      // Click close button
      await page.click('.dialog-header .close-btn')

      await expect(page.locator('.dialog-overlay')).not.toBeVisible()
    })

    test('should select session and update URL', async ({ page }) => {
      const sessionTitle = `Select Session ${Date.now()}`

      // Create a session first
      await page.click('.sidebar-header .btn-primary')
      await page.fill('#sessionTitle', sessionTitle)
      await page.click('.dialog-footer .btn-primary')

      // Wait for session to appear
      await expect(page.locator('.session-item', { hasText: sessionTitle })).toBeVisible()

      // Click the session
      await page.click(`.session-item:has-text("${sessionTitle}")`)

      // URL should update to include session ID
      await expect(page).toHaveURL(/\/canvas\//)
    })

    test('should delete session via hover delete button', async ({ page }) => {
      const sessionTitle = `Delete Session ${Date.now()}`

      // Create a session
      await page.click('.sidebar-header .btn-primary')
      await page.fill('#sessionTitle', sessionTitle)
      await page.click('.dialog-footer .btn-primary')

      await expect(page.locator('.session-item', { hasText: sessionTitle })).toBeVisible()

      // Hover over session to reveal delete button
      await page.hover(`.session-item:has-text("${sessionTitle}")`)
      await page.click(`.session-item:has-text("${sessionTitle}") .delete-btn`)

      // Session should be removed
      await expect(page.locator('.session-item', { hasText: sessionTitle })).not.toBeVisible()
    })
  })

  test.describe('Navigation', () => {
    test('should navigate back to notes via Back to Notes link', async ({ page }) => {
      await page.click('.back-link')

      await expect(page).toHaveURL('/')
      await expect(page.locator('.home-view')).toBeVisible()
    })
  })
})
