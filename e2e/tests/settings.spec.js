/**
 * E2E Tests: Settings Modal
 *
 * Tests settings modal open/close, theme options, and section visibility.
 */

import { test, expect } from '@playwright/test'
import { waitForAppReady } from './fixtures/test-helpers.js'

test.describe('Settings Modal', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test('should open settings modal via gear button', async ({ page }) => {
    // Click the settings gear button in header
    await page.click('.action-buttons .btn-ghost')

    await expect(page.locator('.settings-modal')).toBeVisible()
    await expect(page.locator('.settings-modal .modal-header h2')).toContainText('Settings')
  })

  test('should close settings modal via X button', async ({ page }) => {
    await page.click('.action-buttons .btn-ghost')
    await expect(page.locator('.settings-modal')).toBeVisible()

    // Click close button
    await page.click('.settings-modal .close-btn')

    await expect(page.locator('.settings-modal')).not.toBeVisible()
  })

  test('should show theme options', async ({ page }) => {
    await page.click('.action-buttons .btn-ghost')
    await expect(page.locator('.settings-modal')).toBeVisible()

    await expect(page.locator('.theme-options')).toBeVisible()
    await expect(page.locator('.theme-option')).toHaveCount(3) // System, Light, Dark
  })

  test('should show feedback section with Send Feedback button', async ({ page }) => {
    await page.click('.action-buttons .btn-ghost')
    await expect(page.locator('.settings-modal')).toBeVisible()

    await expect(page.locator('.settings-modal .action-btn:has-text("Send Feedback")')).toBeVisible()
  })

  test('should close settings modal via Cancel button', async ({ page }) => {
    await page.click('.action-buttons .btn-ghost')
    await expect(page.locator('.settings-modal')).toBeVisible()

    await page.click('.settings-modal .cancel-btn')

    await expect(page.locator('.settings-modal')).not.toBeVisible()
  })
})
