/**
 * E2E Tests: Markdown Editor
 *
 * Tests markdown editing and preview functionality.
 * Updated for graph-first UI: scoped to .editor-panel-overlay, uses TopicSelector flow.
 */

import { test, expect } from '@playwright/test'
import {
  waitForAppReady,
  createNote,
  createNoteViaAPI,
  selectNote,
} from './fixtures/test-helpers.js'

const BASE_URL = 'http://localhost:8080'

test.describe('Markdown Editor', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test.describe('Edit Mode', () => {
    test('should show textarea in edit mode after creating note', async ({ page }) => {
      await createNote(page, { title: 'Edit Test', content: '' })

      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toBeVisible()
    })

    test('should allow typing markdown content', async ({ page }) => {
      const content = '# Heading\n\nParagraph text'
      await createNote(page, { title: 'Type Test', content })

      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toHaveValue(content)
    })

    test('should preserve markdown formatting after save and reload', async ({ page, request }) => {
      const title = `Format Test ${Date.now()}`
      const content = '## Subheading\n\n- List item 1\n- List item 2\n\n**Bold text**'

      await createNoteViaAPI(request, BASE_URL, { title, content })
      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, title)
      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toHaveValue(content)
    })
  })

  test.describe('Preview Mode', () => {
    test('should toggle between edit and preview tabs', async ({ page }) => {
      await createNote(page, { title: 'Preview Toggle', content: '# Hello World' })

      // Click Preview tab inside editor overlay
      const previewTab = page.locator('.editor-panel-overlay .tab-btn:has-text("Preview")')
      await expect(previewTab).toBeVisible()
      await previewTab.click()

      await expect(page.locator('.editor-panel-overlay .editor-preview')).toBeVisible()
    })

    test('should render markdown headings', async ({ page }) => {
      await createNote(page, { title: 'Heading Test', content: '# Heading 1\n## Heading 2\n### Heading 3' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('h1')).toContainText('Heading 1')
      await expect(preview.locator('h2')).toContainText('Heading 2')
      await expect(preview.locator('h3')).toContainText('Heading 3')
    })

    test('should render markdown lists', async ({ page }) => {
      await createNote(page, { title: 'List Test', content: '- Item 1\n- Item 2\n- Item 3' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('ul li')).toHaveCount(3)
    })

    test('should render markdown links', async ({ page }) => {
      await createNote(page, { title: 'Link Test', content: '[Example Link](https://example.com)' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      const link = preview.locator('a[href="https://example.com"]')
      await expect(link).toContainText('Example Link')
    })

    test('should render bold and italic', async ({ page }) => {
      await createNote(page, { title: 'Formatting Test', content: '**Bold** and *Italic* text' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('strong')).toContainText('Bold')
      await expect(preview.locator('em')).toContainText('Italic')
    })
  })

  test.describe('Wikilinks', () => {
    test('should render wikilinks in preview', async ({ page }) => {
      await createNote(page, { title: 'Wikilink Test', content: 'Check out [[Another Note]]' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('.wikilink, a:has-text("Another Note")')).toBeVisible()
    })

    test('should render wikilinks with display text', async ({ page }) => {
      await createNote(page, { title: 'Display Wikilink Test', content: 'See [[Note Title|custom display]]' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('.wikilink, a:has-text("custom display")')).toBeVisible()
    })

    test('should navigate on wikilink click', async ({ page, request }) => {
      // Create target note
      await createNoteViaAPI(request, BASE_URL, { title: 'Link Target', content: 'This is the target' })

      // Create note with wikilink
      await createNoteViaAPI(request, BASE_URL, { title: 'Link Source', content: 'Go to [[Link Target]]' })

      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, 'Link Source')

      // Switch to preview
      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      const wikilink = preview.locator('.wikilink, a:has-text("Link Target")')

      if (await wikilink.isVisible()) {
        await wikilink.click()

        // Should navigate to target note — editor should now show target content
        await expect(page.locator('.editor-panel-overlay .editor-textarea')).toContainText('This is the target')
      }
    })
  })

  test.describe('Code Blocks', () => {
    test('should render inline code', async ({ page }) => {
      await createNote(page, { title: 'Code Test', content: 'Use `const x = 1` for declaration' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('code')).toContainText('const x = 1')
    })

    test('should render fenced code blocks', async ({ page }) => {
      await createNote(page, {
        title: 'Code Block Test',
        content: '```javascript\nfunction hello() {\n  return "world";\n}\n```',
      })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('pre code')).toContainText('function hello')
    })
  })

  test.describe('Special Characters', () => {
    test('should handle unicode characters', async ({ page, request }) => {
      const title = 'Unicode Test'
      const content = '日本語テキスト and 中文文本'

      await createNoteViaAPI(request, BASE_URL, { title, content })
      await page.reload()
      await waitForAppReady(page)

      await selectNote(page, title)
      await expect(page.locator('.editor-panel-overlay .editor-textarea')).toHaveValue(content)
    })

    test('should handle special markdown characters', async ({ page }) => {
      await createNote(page, { title: 'Special Chars', content: '> Blockquote\n\n---\n\n* * *' })

      await page.click('.editor-panel-overlay .tab-btn:has-text("Preview")')

      const preview = page.locator('.editor-panel-overlay .editor-preview')
      await expect(preview.locator('blockquote')).toBeVisible()
      await expect(preview.locator('hr')).toBeVisible()
    })
  })
})
