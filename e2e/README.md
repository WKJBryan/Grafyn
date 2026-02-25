# Grafyn E2E Tests

End-to-end tests for the Grafyn knowledge management platform using Playwright.

## Architecture

Grafyn uses a **graph-first UI** with overlay editor panels:

- **Main content area**: Knowledge graph visualization (D3.js)
- **Left sidebar** (`.sidebar-left`): `TreeNav` — hierarchical note navigation by topic
- **Right sidebar** (`.sidebar-right`): Backlinks, contradictions, interactive graph
- **Editor**: Opens as an **overlay panel** (`.editor-panel-overlay`) on top of the graph
- **Note creation**: Goes through `TopicSelector` modal → editor overlay
- **Delete confirmation**: Uses custom `ConfirmDialog` component (not `window.confirm()`)
- **Search**: `SearchBar` with dropdown results, keyboard navigation

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
    ├── note-crud.spec.js      # Note CRUD operations + TopicSelector
    ├── search.spec.js         # Search functionality
    ├── navigation.spec.js     # Navigation and layout
    ├── markdown.spec.js       # Markdown editor/preview
    ├── auth.spec.js           # Authentication flow
    ├── api.spec.js            # API integration tests
    ├── canvas.spec.js         # Canvas session management
    ├── settings.spec.js       # Settings modal
    └── feedback.spec.js       # Feedback modal
```

## Test Coverage

### Note CRUD Operations (note-crud.spec.js)
- Create note (TopicSelector → editor overlay → save)
- Create note with tags and status
- TopicSelector: show/cancel/skip/apply topic
- Read note via TreeNav selection
- Graph view as main content
- Empty state banner
- Update note title and content
- Save button disabled state
- Delete via ConfirmDialog (confirm and cancel)
- Editor overlay closes after delete

### Search Functionality (search.spec.js)
- Search input visibility and focus
- Clear button functionality
- Search results display with score bars
- Open editor overlay on result click
- Keyboard navigation (Enter, Escape)
- Debounce behavior
- Edge cases (empty, no results, special chars)

### Navigation & Layout (navigation.spec.js)
- Three-panel layout (sidebar-left, main-content, sidebar-right)
- Header buttons (New Note, Canvas, Settings)
- TreeNav note selection and active state
- Editor overlay open/close
- Backlinks panel in right sidebar
- Empty state banner
- Canvas navigation (to/from)
- Responsive behavior (tablet, mobile)
- Page refresh persistence

### Markdown Editor (markdown.spec.js)
- Edit mode in editor overlay
- Preview mode toggle (Edit/Preview tabs)
- Markdown rendering (headings, lists, links, bold, italic)
- Wikilink rendering and navigation
- Code blocks (inline and fenced)
- Unicode character support
- Special markdown characters

### Authentication (auth.spec.js)
- Login page display with "Continue with GitHub/Google" buttons
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

### Canvas (canvas.spec.js)
- Canvas view layout (sidebar, main area)
- Empty state ("Multi-LLM Canvas")
- Create session dialog (open, fill, create, cancel)
- Session list and selection
- Delete session via hover button
- Navigation back to notes
- **Note**: LLM interaction tests require `OPENROUTER_API_KEY`

### Settings (settings.spec.js)
- Open/close settings modal
- Theme options (System, Light, Dark)
- Feedback section visibility
- Cancel button

### Feedback (feedback.spec.js)
- Feedback type selection (Bug/Feature/General)
- Form validation (disabled submit when incomplete)
- Character count display
- Submit with mocked API
- Close via X button and Cancel

## Key Selectors Reference

| Element | Selector |
|---------|----------|
| App container | `.home-view` |
| Left sidebar | `.sidebar-left` |
| Right sidebar | `.sidebar-right` |
| Main content | `.main-content` |
| Graph container | `.full-graph-container` |
| TreeNav | `.tree-nav` |
| Nav item | `.tree-nav .nav-item` |
| Active nav item | `.tree-nav .nav-item.active` |
| Editor overlay | `.editor-panel-overlay` |
| Title input | `.editor-panel-overlay .title-input` |
| Editor textarea | `.editor-panel-overlay .editor-textarea` |
| Preview area | `.editor-panel-overlay .editor-preview` |
| Editor tabs | `.editor-panel-overlay .tab-btn` |
| Status dropdown | `.editor-panel-overlay .status-select` |
| Tags input | `.editor-panel-overlay .tags-input` |
| Close button | `.editor-panel-overlay .close-btn` |
| Search input | `input[placeholder*="Search"]` |
| Search results | `.search-results` |
| Result item | `.search-result-item` |
| TopicSelector | `.topic-selector-overlay` |
| ConfirmDialog | `.confirm-dialog` |
| Empty state | `.empty-state-banner` |
| Settings modal | `.settings-modal` |
| Feedback modal | `.feedback-modal` |
| Canvas view | `.canvas-view` |

## Helper Functions

```javascript
import {
  waitForAppReady,       // Wait for .home-view + networkidle
  generateNoteTitle,     // Unique timestamped title
  generateNoteContent,   // Sample markdown content
  createNote,            // Full UI flow: TopicSelector → editor → save
  createNoteViaAPI,      // Direct API creation (faster for test setup)
  selectNote,            // Click note in TreeNav → wait for editor overlay
  closeEditorOverlay,    // Click close button on editor overlay
  completeTopicSelector, // Navigate TopicSelector modal
  deleteNote,            // Click Delete → ConfirmDialog → confirm
  confirmDelete,         // Confirm ConfirmDialog
  cancelDelete,          // Cancel ConfirmDialog
  searchNote,            // Fill search input → wait for results
  clearAllNotes,         // Delete all notes via API
  mockApiResponses,      // Route API calls to mock responses
} from './fixtures/test-helpers.js'
```

## Configuration

### Web Servers
The test configuration automatically starts:
- Frontend dev server on port 5173 (`cd ../frontend && npm run dev`)
- Backend API server on port 8080 (`cd ../backend && uvicorn ...`)

Backend runs with `ENVIRONMENT=test`, `VAULT_PATH=/tmp/test_vault`, `DATA_PATH=/tmp/test_data`.

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

## CI/CD Integration

Tests run in `.github/workflows/test.yml` after backend and frontend unit tests pass.

CI configuration:
- Single worker for stability
- 2 retries on failure
- HTML and list reporters
- Screenshots/videos on failure
- Trace on first retry

## Test Counts

- **note-crud.spec.js**: 17 tests (including TopicSelector)
- **search.spec.js**: 12 tests
- **navigation.spec.js**: 16 tests
- **markdown.spec.js**: 14 tests
- **auth.spec.js**: 15 tests
- **api.spec.js**: 16 tests
- **canvas.spec.js**: 9 tests
- **settings.spec.js**: 5 tests
- **feedback.spec.js**: 7 tests

**Total: ~111 E2E tests**

## Troubleshooting

### Tests timing out
- Increase timeout in `playwright.config.js`
- Check that both frontend and backend servers are running
- Verify health endpoint at http://localhost:8080/health

### Flaky tests
- Use `createNoteViaAPI()` instead of UI creation for `beforeEach` setup
- Add explicit waits for network responses
- Use `waitForLoadState('networkidle')` after navigation

### Browser issues
- Run `npm run install-browsers` to reinstall
- Update Playwright: `npm update @playwright/test`
