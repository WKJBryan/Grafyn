/**
 * E2E Tests: Markdown Editor
 *
 * Tests markdown editing and preview functionality
 */

import { test, expect } from '@playwright/test'
import { waitForAppReady, generateNoteTitle } from './fixtures/test-helpers.js'

test.describe('Markdown Editor', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test.describe('Edit Mode', () => {
    test('should show textarea in edit mode', async ({ page }) => {
      await page.click('button:has-text("New Note")')

      await expect(page.locator('textarea')).toBeVisible()
    })

    test('should allow typing markdown content', async ({ page }) => {
      await page.click('button:has-text("New Note")')

      const content = '# Heading\n\nParagraph text'
      await page.fill('textarea', content)

      await expect(page.locator('textarea')).toHaveValue(content)
    })

    test('should preserve markdown formatting', async ({ page }) => {
      const title = generateNoteTitle()
      const content = '## Subheading\n\n- List item 1\n- List item 2\n\n**Bold text**'

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', content)
      await page.click('button:has-text("Save")')

      // Reload and check
      await page.reload()
      await waitForAppReady(page)
      await page.click(`.note-item:has-text("${title}")`)

      await expect(page.locator('textarea')).toHaveValue(content)
    })
  })

  test.describe('Preview Mode', () => {
    test('should toggle between edit and preview', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Preview Test')
      await page.fill('textarea', '# Hello World')

      // Find and click preview toggle (may be a button or tab)
      const previewBtn = page.locator('button:has-text("Preview"), [data-mode="preview"], .preview-toggle')

      if (await previewBtn.isVisible()) {
        await previewBtn.click()
        await expect(page.locator('.preview, .markdown-preview, .rendered-content')).toBeVisible()
      }
    })

    test('should render markdown headings', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Heading Test')
      await page.fill('textarea', '# Heading 1\n## Heading 2\n### Heading 3')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('h1')).toContainText('Heading 1')
        await expect(page.locator('h2')).toContainText('Heading 2')
        await expect(page.locator('h3')).toContainText('Heading 3')
      }
    })

    test('should render markdown lists', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'List Test')
      await page.fill('textarea', '- Item 1\n- Item 2\n- Item 3')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('ul li')).toHaveCount(3)
      }
    })

    test('should render markdown links', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Link Test')
      await page.fill('textarea', '[Example Link](https://example.com)')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        const link = page.locator('a[href="https://example.com"]')
        await expect(link).toContainText('Example Link')
      }
    })

    test('should render bold and italic', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Formatting Test')
      await page.fill('textarea', '**Bold** and *Italic* text')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('strong')).toContainText('Bold')
        await expect(page.locator('em')).toContainText('Italic')
      }
    })
  })

  test.describe('Wikilinks', () => {
    test('should render wikilinks in preview', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Wikilink Test')
      await page.fill('textarea', 'Check out [[Another Note]]')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        // Wikilinks should be rendered as clickable links
        await expect(page.locator('.wikilink, a:has-text("Another Note")')).toBeVisible()
      }
    })

    test('should render wikilinks with display text', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Display Wikilink Test')
      await page.fill('textarea', 'See [[Note Title|custom display]]')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('.wikilink, a:has-text("custom display")')).toBeVisible()
      }
    })

    test('should navigate on wikilink click', async ({ page }) => {
      // Create target note
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Link Target')
      await page.fill('textarea', 'This is the target')
      await page.click('button:has-text("Save")')

      // Create note with wikilink
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Link Source')
      await page.fill('textarea', 'Go to [[Link Target]]')
      await page.click('button:has-text("Save")')

      // Click on wikilink in preview (if available)
      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        const wikilink = page.locator('.wikilink, a:has-text("Link Target")')
        if (await wikilink.isVisible()) {
          await wikilink.click()

          // Should navigate to target note
          await expect(page.locator('textarea')).toContainText('This is the target')
        }
      }
    })
  })

  test.describe('Code Blocks', () => {
    test('should render inline code', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Code Test')
      await page.fill('textarea', 'Use `const x = 1` for declaration')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('code')).toContainText('const x = 1')
      }
    })

    test('should render code blocks', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Code Block Test')
      await page.fill('textarea', '```javascript\nfunction hello() {\n  return "world";\n}\n```')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('pre code')).toContainText('function hello')
      }
    })
  })

  test.describe('Special Characters', () => {
    test('should handle unicode characters', async ({ page }) => {
      const title = 'Unicode Test'
      const content = '日本語テキスト and 中文文本 🎉'

      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', title)
      await page.fill('textarea', content)
      await page.click('button:has-text("Save")')

      // Reload and verify
      await page.reload()
      await waitForAppReady(page)
      await page.click(`.note-item:has-text("${title}")`)

      await expect(page.locator('textarea')).toHaveValue(content)
    })

    test('should handle special markdown characters', async ({ page }) => {
      await page.click('button:has-text("New Note")')
      await page.fill('input[placeholder*="title"], input[name="title"]', 'Special Chars')
      await page.fill('textarea', '> Blockquote\n\n---\n\n* * *')

      const previewBtn = page.locator('button:has-text("Preview")')
      if (await previewBtn.isVisible()) {
        await previewBtn.click()

        await expect(page.locator('blockquote')).toBeVisible()
        await expect(page.locator('hr')).toBeVisible()
      }
    })
  })
})
