/**
 * E2E Tests: Search Functionality
 *
 * Tests semantic search and navigation
 */

import { test, expect } from '@playwright/test'
import { generateNoteTitle, waitForAppReady } from './fixtures/test-helpers.js'

test.describe('Search Functionality', () => {
  // Create some notes before running search tests
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)

    // Create a few test notes for searching
    const testNotes = [
      { title: 'Machine Learning Basics', content: 'Neural networks and deep learning concepts' },
      { title: 'Python Programming', content: 'Python is a great language for machine learning' },
      { title: 'Web Development', content: 'Building websites with HTML, CSS, and JavaScript' },
    ]

    for (const note of testNotes) {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', note.title)
      await page.fill('textarea', note.content)
      await page.click('button:has-text("Save")')
      await page.waitForTimeout(500) // Small delay between creates
    }
  })

  test.describe('Search Input', () => {
    test('should show search input in header', async ({ page }) => {
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

      await expect(page.locator('.clear-btn, button:has-text("×")')).toBeVisible()
    })

    test('should clear input when clear button clicked', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('test query')

      await page.click('.clear-btn, button:has-text("×")')

      await expect(searchInput).toHaveValue('')
    })
  })

  test.describe('Search Results', () => {
    test('should show search results dropdown', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('machine learning')

      // Wait for debounce and results
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

    test('should navigate to note on result click', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('Web Development')

      await page.waitForTimeout(400)

      // Click on result
      await page.click('.search-result-item:first-child')

      // Verify editor shows the note
      await expect(page.locator('.note-editor')).toBeVisible()
      await expect(page.locator('textarea')).toContainText(/HTML|CSS|JavaScript/)
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

      // Should navigate to note
      await expect(page.locator('.note-editor')).toBeVisible()
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

      // Should not show results
      await expect(page.locator('.search-results')).not.toBeVisible()
    })

    test('should handle no results', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')
      await searchInput.fill('xyznonexistent123')

      await page.waitForTimeout(400)

      // Should not show results dropdown (or show empty message)
      const results = page.locator('.search-result-item')
      await expect(results).toHaveCount(0)
    })

    test('should debounce rapid typing', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search"]')

      // Type rapidly
      await searchInput.type('machine learning', { delay: 50 })

      // Should only make one search request after debounce
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
