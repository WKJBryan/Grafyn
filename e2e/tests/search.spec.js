/**
 * E2E Tests: Search Functionality
 *
 * Tests semantic search and navigation.
 * Updated for graph-first UI: uses API-based note creation and editor overlay selectors.
 */

import { test, expect } from '@playwright/test'
import { waitForAppReady, createNoteViaAPI, clearAllNotes } from './fixtures/test-helpers.js'

const BASE_URL = 'http://localhost:8080'

test.describe('Search Functionality', () => {
  test.beforeEach(async ({ page, request }) => {
    await page.goto('/')
    await waitForAppReady(page)

    // Create test notes via API for speed
    const testNotes = [
      { title: 'Machine Learning Basics', content: 'Neural networks and deep learning concepts' },
      { title: 'Python Programming', content: 'Python is a great language for machine learning' },
      { title: 'Web Development', content: 'Building websites with HTML, CSS, and JavaScript' },
    ]

    for (const note of testNotes) {
      await createNoteViaAPI(request, BASE_URL, note)
    }

    // Reload to pick up API-created notes
    await page.reload()
    await waitForAppReady(page)
  })

  test.describe('Search Input', () => {
    test('should show search input', async ({ page }) => {
      await expect(page.locator('input[placeholder*="Search"]')).toBeVisible()
    })

    test('should focus search input on click', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.click()
      await expect(searchInput).toBeFocused()
    })

    test('should show clear button when typing', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('test query')

      await expect(page.locator('.clear-btn')).toBeVisible()
    })

    test('should clear input when clear button clicked', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('test query')

      await page.click('.clear-btn')

      await expect(searchInput).toHaveValue('')
    })
  })

  test.describe('Search Results', () => {
    test('should show search results dropdown', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('machine learning')

      await page.waitForTimeout(400)

      await expect(page.locator('.search-results')).toBeVisible()
    })

    test('should display matching notes in results', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('machine learning')

      await page.waitForTimeout(400)

      const results = page.locator('.search-results .search-result-item')
      await expect(results.first()).toBeVisible()
    })

    test('should show score bars for results', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('Python')

      await page.waitForTimeout(400)

      await expect(page.locator('.score-bar')).toBeVisible()
    })

    test('should open editor overlay on result click', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('Web Development')

      await page.waitForTimeout(400)

      // Click on result
      await page.click('.search-result-item:first-child')

      // Editor overlay should open with the note content
      await expect(page.locator('.editor-panel-overlay')).toBeVisible()
    })

    test('should clear results after selection', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('Python')

      await page.waitForTimeout(400)
      await page.click('.search-result-item:first-child')

      await expect(page.locator('.search-results')).not.toBeVisible()
      await expect(searchInput).toHaveValue('')
    })
  })

  test.describe('Search Keyboard Navigation', () => {
    test('should select first result on Enter', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('machine')

      await page.waitForTimeout(400)
      await searchInput.press('Enter')

      // Should open editor overlay
      await expect(page.locator('.editor-panel-overlay')).toBeVisible()
    })

    test('should close results on Escape', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('Python')

      await page.waitForTimeout(400)
      await expect(page.locator('.search-results')).toBeVisible()

      await searchInput.press('Escape')

      await expect(page.locator('.search-results')).not.toBeVisible()
      await expect(searchInput).toHaveValue('')
    })
  })

  test.describe('Search Edge Cases', () => {
    test('should handle empty search gracefully', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('')

      await page.waitForTimeout(400)

      await expect(page.locator('.search-results')).not.toBeVisible()
    })

    test('should handle no results', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('xyznonexistent123')

      await page.waitForTimeout(400)

      const results = page.locator('.search-result-item')
      await expect(results).toHaveCount(0)
    })

    test('should debounce rapid typing', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')

      await searchInput.type('machine learning', { delay: 50 })

      await page.waitForTimeout(400)

      await expect(page.locator('.search-results')).toBeVisible()
    })

    test('should handle special characters', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('[[wikilink]]')

      await page.waitForTimeout(400)

      // Should not crash
      await expect(page.locator('.home-view')).toBeVisible()
    })
  })
})
