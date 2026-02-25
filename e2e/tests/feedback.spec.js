/**
 * E2E Tests: Feedback Modal
 *
 * Tests feedback form interaction, validation, and submission.
 */

import { test, expect } from '@playwright/test'
import { waitForAppReady } from './fixtures/test-helpers.js'

test.describe('Feedback Modal', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)

    // Open feedback modal via Settings → Send Feedback
    await page.click('.action-buttons .btn-ghost')
    await page.waitForSelector('.settings-modal', { state: 'visible' })
    await page.click('.settings-modal .action-btn:has-text("Send Feedback")')
    await page.waitForSelector('.feedback-modal-overlay', { state: 'visible' })
  })

  test('should display feedback modal with correct title', async ({ page }) => {
    await expect(page.locator('.feedback-modal')).toBeVisible()
    await expect(page.locator('.feedback-modal .modal-header h3')).toContainText('Send Feedback')
  })

  test('should display feedback type options', async ({ page }) => {
    await expect(page.locator('.type-option')).toHaveCount(3)
    await expect(page.locator('.type-option:has-text("Bug Report")')).toBeVisible()
    await expect(page.locator('.type-option:has-text("Feature Request")')).toBeVisible()
    await expect(page.locator('.type-option:has-text("General Feedback")')).toBeVisible()
  })

  test('should disable submit when form is incomplete', async ({ page }) => {
    // Submit button should be disabled initially
    await expect(page.locator('.feedback-modal .btn-primary:has-text("Submit Feedback")')).toBeDisabled()
  })

  test('should show character count for title and description', async ({ page }) => {
    // Select a type first
    await page.click('.type-option:has-text("Bug Report")')

    // Fill title
    await page.fill('.text-input', 'Test bug report title')
    await expect(page.locator('.char-count').first()).toBeVisible()

    // Fill description
    await page.fill('.textarea-input', 'This is a detailed description of the bug')
    await expect(page.locator('.char-count').nth(1)).toBeVisible()
  })

  test('should submit feedback with mocked API', async ({ page }) => {
    // Mock the feedback API
    await page.route('**/api/feedback', route => {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          issue_url: 'https://github.com/test/repo/issues/1',
        }),
      })
    })

    // Fill out the form
    await page.click('.type-option:has-text("Feature Request")')
    await page.fill('.text-input', 'Add dark mode toggle in sidebar')
    await page.fill('.textarea-input', 'It would be great to have a quick toggle for dark mode accessible from the sidebar instead of going through settings.')

    // Submit
    await page.click('.feedback-modal .btn-primary:has-text("Submit Feedback")')

    // Should show success message
    await expect(page.locator('.success-message')).toBeVisible()
  })

  test('should close feedback modal via X button', async ({ page }) => {
    await page.click('.feedback-modal .close-btn')

    await expect(page.locator('.feedback-modal-overlay')).not.toBeVisible()
  })

  test('should close feedback modal via Cancel button', async ({ page }) => {
    await page.click('.feedback-modal .btn-ghost:has-text("Cancel")')

    await expect(page.locator('.feedback-modal-overlay')).not.toBeVisible()
  })
})
