# Seedream E2E Tests

End-to-end tests for the Seedream knowledge management platform using Playwright.

## Setup

```bash
cd e2e
npm install
npm run install-browsers
```

## Running Tests

```bash
# Run all tests
npm test

# Run with UI (visual mode)
npm run test:ui

# Run in headed mode (see browser)
npm run test:headed

# Run with debug mode
npm run test:debug

# Run specific browser
npm run test:chromium
npm run test:firefox
npm run test:webkit

# Run mobile tests
npm run test:mobile

# View test report
npm run report
```

## Test Structure

```
e2e/
├── playwright.config.js    # Playwright configuration
├── package.json           # Dependencies and scripts
├── README.md              # This file
└── tests/
    ├── fixtures/
    │   └── test-helpers.js    # Shared utilities and helpers
    ├── note-crud.spec.js      # Note CRUD operations
    ├── search.spec.js         # Search functionality
    ├── navigation.spec.js     # Navigation and layout
    ├── markdown.spec.js       # Markdown editor/preview
    ├── auth.spec.js           # Authentication flow
    └── api.spec.js            # API integration tests
```

## Test Coverage

### Note CRUD Operations (note-crud.spec.js)
- Create note with title and content
- Create note with tags
- Create note with status
- Validation for empty title
- Read note content
- Update note title
- Update note content
- Delete note with confirmation
- Cancel delete
- Clear editor after delete

### Search Functionality (search.spec.js)
- Search input visibility
- Clear button functionality
- Search results display
- Score bar visualization
- Navigate to note on click
- Keyboard navigation (Enter, Escape)
- Debounce behavior
- Edge cases (empty, no results, special chars)

### Navigation & Layout (navigation.spec.js)
- Main layout (header, sidebar, editor)
- Note list navigation
- Selection updates
- Backlinks panel visibility
- Empty states
- Responsive behavior (tablet, mobile)
- Page refresh persistence

### Markdown Editor (markdown.spec.js)
- Edit mode with textarea
- Preview mode toggle
- Markdown rendering (headings, lists, links, bold, italic)
- Wikilink rendering and navigation
- Code blocks (inline and fenced)
- Unicode character support
- Special markdown characters

### Authentication (auth.spec.js)
- Login page display
- OAuth provider buttons
- OAuth flow initiation
- Callback handling (success, error, missing params)
- Session management (localStorage token)
- 401 redirect to login
- Logout flow

### API Integration (api.spec.js)
- Notes CRUD endpoints
- Search endpoints
- Graph endpoints (backlinks, outgoing, neighbors)
- Error handling (404, 422)
- Rate limiting behavior

## Configuration

### Web Servers
The test configuration automatically starts:
- Frontend dev server on port 5173
- Backend API server on port 8080

### Browsers
Tests run on:
- Chromium (Desktop Chrome)
- Firefox (Desktop Firefox)
- WebKit (Desktop Safari)
- Mobile Chrome (Pixel 5)
- Mobile Safari (iPhone 12)

### Timeouts
- Global timeout: 30 seconds
- Expect timeout: 5 seconds
- Web server startup: 120 seconds

## Writing New Tests

```javascript
import { test, expect } from '@playwright/test'
import { waitForAppReady, generateNoteTitle } from './fixtures/test-helpers.js'

test.describe('My Feature', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForAppReady(page)
  })

  test('should do something', async ({ page }) => {
    // Test implementation
    await expect(page.locator('.element')).toBeVisible()
  })
})
```

## Helper Functions

```javascript
// Wait for app to be fully loaded
await waitForAppReady(page)

// Generate unique note title
const title = generateNoteTitle()

// Generate sample content
const content = generateNoteContent()

// Create a note via UI
await createNote(page, { title, content, status: 'draft', tags: [] })

// Select a note from the list
await selectNote(page, 'Note Title')

// Search for a note
await searchNote(page, 'query')
```

## CI/CD Integration

Tests are configured to run in CI with:
- Single worker for stability
- Retries on failure (2 retries)
- HTML and list reporters
- Screenshots/videos on failure
- Trace on first retry

## Troubleshooting

### Tests timing out
- Increase timeout in playwright.config.js
- Check that both frontend and backend servers are running
- Verify health endpoint at http://localhost:8080/health

### Flaky tests
- Add explicit waits for network requests
- Use `waitForLoadState('networkidle')`
- Add small delays between rapid operations

### Browser issues
- Run `npm run install-browsers` to reinstall
- Update Playwright: `npm update @playwright/test`

## Test Counts

- **note-crud.spec.js**: 13 tests
- **search.spec.js**: 14 tests
- **navigation.spec.js**: 16 tests
- **markdown.spec.js**: 15 tests
- **auth.spec.js**: 15 tests
- **api.spec.js**: 16 tests

**Total: 89 E2E tests**
