/**
 * E2E Test Helpers and Fixtures
 */

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
    content: 'Welcome to Seedream!\n\nThis is your first note.',
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
 * Wait for the app to be fully loaded
 */
export async function waitForAppReady(page) {
  await page.waitForSelector('.home-view', { state: 'visible' })
  await page.waitForLoadState('networkidle')
}

/**
 * Create a note via the UI
 */
export async function createNote(page, { title, content, status = 'draft', tags = [] }) {
  // Click New Note button
  await page.click('button:has-text("New Note")')

  // Wait for editor
  await page.waitForSelector('.note-editor')

  // Fill in title
  await page.fill('input[placeholder*="title"]', title)

  // Fill in content
  await page.fill('textarea', content)

  // Select status if not draft
  if (status !== 'draft') {
    await page.selectOption('select', status)
  }

  // Add tags
  if (tags.length > 0) {
    await page.fill('input[placeholder*="tag"]', tags.join(', '))
  }

  // Save
  await page.click('button:has-text("Save")')

  // Wait for save to complete
  await page.waitForResponse(resp =>
    resp.url().includes('/api/notes') && resp.status() === 200
  )
}

/**
 * Select a note from the list
 */
export async function selectNote(page, title) {
  await page.click(`.note-item:has-text("${title}")`)
  await page.waitForSelector('.note-editor')
}

/**
 * Delete the currently selected note
 */
export async function deleteNote(page) {
  // Click delete button
  await page.click('button:has-text("Delete")')

  // Accept confirmation dialog
  page.on('dialog', dialog => dialog.accept())

  // Wait for delete to complete
  await page.waitForResponse(resp =>
    resp.url().includes('/api/notes') && resp.request().method() === 'DELETE'
  )
}

/**
 * Search for a note
 */
export async function searchNote(page, query) {
  // Click on search input
  await page.fill('input[placeholder*="Search"]', query)

  // Wait for search results
  await page.waitForSelector('.search-results', { state: 'visible' })
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
