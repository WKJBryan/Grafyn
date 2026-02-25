/**
 * E2E Test Helpers and Fixtures
 *
 * Updated for graph-first UI with overlay editor panels, TopicSelector modal,
 * TreeNav navigation, and ConfirmDialog.
 */

const API_BASE = 'http://localhost:8080/api'

/**
 * Generate a unique note title for testing
 */
export function generateNoteTitle() {
  return `Test Note ${Date.now()}`
}

/**
 * Generate unique note content
 */
export function generateNoteContent() {
  return `
This is test content created at ${new Date().toISOString()}.

## Features
- Item 1
- Item 2
- Item 3

## Links
Check out [[Another Note]] for more info.
`
}

/**
 * Sample notes for testing
 */
export const sampleNotes = [
  {
    title: 'Getting Started',
    content: 'Welcome to Grafyn!\n\nThis is your first note.',
    status: 'canonical',
    tags: ['welcome', 'tutorial'],
  },
  {
    title: 'Quick Notes',
    content: 'Some quick thoughts...\n\n- Idea 1\n- Idea 2',
    status: 'draft',
    tags: ['ideas'],
  },
  {
    title: 'Research Notes',
    content: 'Research findings:\n\nSee [[Getting Started]] for context.',
    status: 'evidence',
    tags: ['research', 'evidence'],
  },
]

/**
 * Wait for the app to be fully loaded.
 * The home view uses a graph-first layout with TreeNav in the left sidebar.
 */
export async function waitForAppReady(page) {
  await page.waitForSelector('.home-view', { state: 'visible' })
  await page.waitForLoadState('networkidle')
}

/**
 * Complete the TopicSelector modal that appears when creating a new note.
 *
 * @param {import('@playwright/test').Page} page
 * @param {Object} options
 * @param {string} [options.noteType] - 'Note', 'Source', or 'Map of Content'
 * @param {string} [options.topic] - Topic name to select or create
 * @param {boolean} [options.skipTopic=true] - Check "Skip - create without topic"
 */
export async function completeTopicSelector(page, { noteType, topic, skipTopic = true } = {}) {
  await page.waitForSelector('.topic-selector-overlay', { state: 'visible' })

  // Select note type if specified (default is "Note" which is pre-selected)
  if (noteType) {
    await page.click(`.type-option:has-text("${noteType}")`)
  }

  if (topic) {
    // Type a new topic or select existing
    const topicInput = page.locator('.new-topic-input input')
    await topicInput.fill(topic)

    // Check if it matches an existing topic chip
    const existingChip = page.locator(`.topic-chip:has-text("${topic}")`)
    if (await existingChip.isVisible({ timeout: 500 }).catch(() => false)) {
      await existingChip.click()
    }
  } else if (skipTopic) {
    // Check the "Skip - create without topic" checkbox
    const skipCheckbox = page.locator('.no-topic-option input[type="checkbox"]')
    if (!(await skipCheckbox.isChecked())) {
      await skipCheckbox.check()
    }
  }

  // Click "Create Note"
  await page.click('.selector-footer .btn-primary')

  // Wait for editor overlay to open
  await page.waitForSelector('.editor-panel-overlay', { state: 'visible' })
}

/**
 * Create a note via the UI (TopicSelector → Editor Overlay → Save).
 *
 * @param {import('@playwright/test').Page} page
 * @param {Object} data
 * @param {string} data.title
 * @param {string} data.content
 * @param {string} [data.status='draft']
 * @param {string[]} [data.tags=[]]
 * @param {Object} [topicOptions] - Options for TopicSelector
 */
export async function createNote(page, { title, content, status = 'draft', tags = [] }, topicOptions) {
  // Click "+ New Note" button in header
  await page.click('button:has-text("New Note")')

  // Complete TopicSelector modal
  await completeTopicSelector(page, topicOptions)

  // Fill in editor overlay fields
  await page.fill('.editor-panel-overlay .title-input', title)
  await page.fill('.editor-panel-overlay .editor-textarea', content)

  // Select status if not draft
  if (status !== 'draft') {
    await page.selectOption('.editor-panel-overlay .status-select', status)
  }

  // Add tags
  if (tags.length > 0) {
    await page.fill('.editor-panel-overlay .tags-input', tags.join(', '))
  }

  // Save
  await page.click('.editor-panel-overlay button:has-text("Save")')

  // Wait for save response
  await page.waitForResponse(resp =>
    resp.url().includes('/api/notes') && resp.status() === 200
  )
}

/**
 * Create a note via API (faster, for test setup — skips UI).
 *
 * @param {import('@playwright/test').APIRequestContext} request
 * @param {string} baseURL
 * @param {Object} data
 * @param {string} data.title
 * @param {string} data.content
 * @param {string} [data.status='draft']
 * @param {string[]} [data.tags=[]]
 * @returns {Promise<Object>} Created note
 */
export async function createNoteViaAPI(request, baseURL, { title, content, status = 'draft', tags = [] }) {
  const response = await request.post(`${baseURL}/api/notes`, {
    data: { title, content, status, tags },
  })
  return response.json()
}

/**
 * Select a note from the TreeNav in the left sidebar.
 */
export async function selectNote(page, title) {
  await page.click(`.tree-nav .nav-item:has-text("${title}")`)
  await page.waitForSelector('.editor-panel-overlay', { state: 'visible' })
}

/**
 * Close the editor overlay panel.
 */
export async function closeEditorOverlay(page) {
  await page.click('.editor-panel-overlay .close-btn')
  await page.waitForSelector('.editor-panel-overlay', { state: 'hidden' })
}

/**
 * Delete the currently open note using the ConfirmDialog.
 */
export async function deleteNote(page) {
  // Click delete button in editor overlay
  await page.click('.editor-panel-overlay button:has-text("Delete")')

  // Wait for ConfirmDialog to appear
  await page.waitForSelector('.confirm-dialog', { state: 'visible' })

  // Click the confirm "Delete" button in the dialog
  await page.click('.confirm-dialog .btn-danger')

  // Wait for delete response
  await page.waitForResponse(resp =>
    resp.url().includes('/api/notes') && resp.request().method() === 'DELETE'
  )
}

/**
 * Confirm the ConfirmDialog by clicking the confirm button.
 */
export async function confirmDelete(page) {
  await page.waitForSelector('.confirm-dialog', { state: 'visible' })
  await page.click('.confirm-dialog .btn-danger')
}

/**
 * Cancel the ConfirmDialog.
 */
export async function cancelDelete(page) {
  await page.waitForSelector('.confirm-dialog', { state: 'visible' })
  await page.click('.confirm-dialog .btn-secondary')
  await page.waitForSelector('.confirm-dialog', { state: 'hidden' })
}

/**
 * Search for a note using the SearchBar.
 */
export async function searchNote(page, query) {
  await page.fill('input[placeholder*="Search"]', query)
  await page.waitForSelector('.search-results', { state: 'visible' })
}

/**
 * Select a note from the TreeNav by title.
 * Alias for selectNote with more descriptive name.
 */
export async function selectNoteFromTree(page, title) {
  return selectNote(page, title)
}

/**
 * Mock API responses for controlled testing
 */
export async function mockApiResponses(page, responses = {}) {
  await page.route('**/api/**', route => {
    const url = route.request().url()

    for (const [pattern, response] of Object.entries(responses)) {
      if (url.includes(pattern)) {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(response),
        })
      }
    }

    return route.continue()
  })
}

/**
 * Clear all notes via API
 */
export async function clearAllNotes(request, baseURL) {
  const response = await request.get(`${baseURL}/api/notes`)
  const notes = await response.json()

  for (const note of notes) {
    await request.delete(`${baseURL}/api/notes/${encodeURIComponent(note.id)}`)
  }
}
