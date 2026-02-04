# Grafyn Frontend Architecture

> **Part:** Frontend | **Type:** Vue 3 SPA | **Scan Level:** Exhaustive

## Overview

The frontend is a Vue 3 Single Page Application providing:
- Note listing and navigation
- Markdown editing with preview
- Semantic search with typeahead
- Backlink visualization
- Authentication (GitHub OAuth)
- Multi-LLM Canvas for comparing AI model responses
- Real-time streaming with SSE
- D3.js graph visualization
- Clean dark theme design system

## Entry Point

**File:** `frontend/src/main.js`

```javascript
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './style.css'

const app = createApp(App)

app.use(createPinia())
app.use(router)

app.mount('#app')
```

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         App.vue (Root)                                │
│                    ┌────────────────────┐                             │
│                    │   <router-view />  │                             │
│                    └────────────────────┘                             │
└──────────────────────────────────────────────────────────────────────┘
                              │
         ┌────────────────────┼────────────────────┐
         ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│   HomeView      │  │   LoginView     │  │ OAuthCallback   │
│                 │  │                 │  │     View        │
│  ┌───────────┐  │  │  Login form     │  │  OAuth handler  │
│  │ SearchBar │  │  │  GitHub OAuth   │  │                 │
│  ├───────────┤  │  │                 │  │                 │
│  │ NoteList  │  │  └─────────────────┘  └─────────────────┘
│  ├───────────┤  │
│  │NoteEditor │  │          ┌─────────────────────────────┐
│  ├───────────┤  │          │         Pinia Stores        │
│  │Backlinks  │  │          │  ┌─────────┐ ┌───────────┐  │
│  │  Panel    │  │◄─────────│  │ auth.js │ │ notes.js  │  │
│  └───────────┘  │          │  └─────────┘ └───────────┘  │
└─────────────────┘          └─────────────────────────────┘
```

## Views (5 Total)

### 1. HomeView.vue
**File:** `src/views/HomeView.vue`

**Purpose:** Main application view with note management.

**Layout:**
- **Header:** Logo, SearchBar, action buttons
- **Sidebar (280px):** NoteList
- **Editor Area (flex):** NoteEditor or empty state
- **Right Panel (300px):** BacklinksPanel (when note selected)

---

### 2. CanvasView.vue
**File:** `src/views/CanvasView.vue`

**Purpose:** Multi-LLM canvas session management.

**Layout:**
- **Sidebar (280px):** Session list with create/delete actions
- **Main Area:** CanvasContainer for active session
- **Empty State:** "Create New Canvas" when no session selected

---

### 3. LoginView.vue
**File:** `src/views/LoginView.vue`

**Purpose:** Authentication page with GitHub OAuth.

---

### 4. OAuthCallbackView.vue
**File:** `src/views/OAuthCallbackView.vue`

**Purpose:** Handles OAuth callback from GitHub, exchanges code for token.

---

### 5. NotFoundView.vue
**File:** `src/views/NotFoundView.vue`

**Purpose:** 404 page for unmatched routes.

---

## Pinia Stores (3 Stores)

### auth.js
**File:** `src/stores/auth.js`

**State:**
```javascript
{
  user: null,              // Current user data
  token: null,             // OAuth access token
  isAuthenticated: false,  // Auth status
}
```

**Actions:**
- `login()` - Initiate OAuth flow
- `handleCallback(code)` - Exchange code for token
- `logout()` - Clear auth state
- `checkAuth()` - Verify token validity

---

### notes.js
**File:** `src/stores/notes.js`

**State:**
```javascript
{
  notes: [],               // All notes list
  selectedNoteId: null,    // Currently selected
  selectedNote: null,      // Full note object
  loading: false,          // Loading state
}
```

**Actions:**
- `loadNotes()` - Fetch all notes
- `selectNote(id)` - Load and display a note
- `createNote(data)` - Create new note
- `updateNote(id, data)` - Update existing note
- `deleteNote(id)` - Delete note

---

### canvas.js
**File:** `src/stores/canvas.js`

**State:**
```javascript
{
  sessions: [],                    // All canvas sessions
  currentSession: null,            // Active session
  availableModels: [],              // Available models from OpenRouter
  loading: false,                 // Loading state
  error: null,                    // Error message
  streamingModels: new Set(),     // Currently streaming models
}
```

**Getters:**
- `promptTiles` - Current session's prompt tiles
- `debates` - Current session's debates
- `hasSession` - Whether a session is active
- `isStreaming` - Whether any model is streaming
- `modelsByProvider` - Models grouped by provider
- `tileEdges` - Parent-child tile edges
- `debateEdges` - Debate connection edges

**Actions:**
- `loadSessions()` - Fetch all sessions
- `loadSession(id)` - Load specific session
- `createSession(data)` - Create new session
- `updateSession(id, data)` - Update session metadata
- `deleteSession(id)` - Delete session
- `loadModels()` - Fetch available models from OpenRouter
- `sendPrompt(...)` - Send prompt to multiple models (SSE streaming)
- `updateTilePosition(tileId, position)` - Update tile position
- `updateLLMNodePosition(tileId, modelId, position)` - Update LLM node position
- `autoArrange(positions)` - Batch update positions
- `deleteTile(tileId)` - Delete tile
- `updateViewport(viewport)` - Save viewport state
- `startDebate(tileIds, models, mode, maxRounds)` - Start debate (SSE)
- `continueDebate(debateId, prompt)` - Continue debate
- `saveAsNote()` - Export canvas to markdown note
- `branchFromResponse(...)` - Branch from specific model response

---

## Components (14 Total)

### Core Components (5)

### 1. SearchBar.vue
**File:** `src/components/SearchBar.vue`

**Purpose:** Semantic search with debounced typeahead dropdown.

**Features:**
- 300ms debounce on input
- Dropdown shows top 5 results
- Score visualization bar
- Click/Enter to select
- Escape to clear

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `select` | `note_id` | User selected a search result |

---

### 2. NoteList.vue
**File:** `src/components/NoteList.vue`

**Purpose:** Sidebar listing of all notes with status and tags.

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `notes` | Array | List of NoteListItem objects |
| `selected` | String | Currently selected note ID |

---

### 3. NoteEditor.vue
**File:** `src/components/NoteEditor.vue`

**Purpose:** Markdown editor with preview mode.

**Features:**
- Edit/Preview toggle
- Title editing
- Markdown content textarea
- Wikilink rendering in preview
- Save button (enabled when dirty)
- Delete button

---

### 4. BacklinksPanel.vue
**File:** `src/components/BacklinksPanel.vue`

**Purpose:** Right panel showing notes that link to current note.

**Props:**
| Prop | Type | Required | Description |
|------|------|----------|-------------|
| `noteId` | String | ✅ | Current note to find backlinks for |

---

### 5. GraphView.vue
**File:** `src/components/GraphView.vue`

**Purpose:** Graph visualization of note connections.

---

### Canvas Components (9)

### 6. CanvasContainer.vue
**File:** `src/components/canvas/CanvasContainer.vue`

**Purpose:** Main canvas component with D3.js zoom/pan and node rendering.

**Features:**
- D3.js zoom/pan with constrained limits
- SVG edge rendering (bezier curves)
- Prompt nodes, LLM nodes, Debate nodes
- Minimap navigation
- Auto-arrange layout algorithm
- Floating action bar (New Prompt, Debate)
- Toolbar with zoom controls and Arrange/Save buttons
- SSE event handling for streaming

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `session-loaded` | `session` | Session loaded and ready |

---

### 7. PromptNode.vue
**File:** `src/components/canvas/PromptNode.vue`

**Purpose:** Compact prompt tile node on canvas.

**Features:**
- Draggable positioning
- Shows prompt text (truncated)
- Delete button
- Selection highlighting

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `tile` | Object | PromptTile data |
| `selected` | Boolean | Whether selected |

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `drag` | `{tileId, position}` | Node dragged |
| `delete` | `tileId` | Delete requested |

---

### 8. LLMNode.vue
**File:** `src/components/canvas/LLMNode.vue`

**Purpose:** Individual LLM response node with streaming support.

**Features:**
- Draggable positioning
- Model name and color header
- Streaming content display
- Branch button (for continuing conversation)
- Selection highlighting
- Copy to clipboard

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `tileId` | String | Parent tile ID |
| `modelId` | String | Model identifier |
| `response` | Object | ModelResponse data |
| `isStreaming` | Boolean | Currently streaming |
| `selected` | Boolean | Whether selected |

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `drag` | `{tileId, modelId, position}` | Node dragged |
| `branch` | `{tileId, modelId, prompt, contextMode}` | Branch requested |
| `select` | `{tileId, modelId}` | Node selected |
| `delete` | `{tileId, modelId}` | Delete requested |

---

### 9. DebateNode.vue
**File:** `src/components/canvas/DebateNode.vue`

**Purpose:** Debate tile node with expandable rounds.

**Features:**
- Draggable positioning
- Expand/collapse rounds
- Show participating models
- Round-by-round content display
- Continue debate button
- Status indicator (active/paused/completed)

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `debate` | Object | DebateRound data |
| `isExpanded` | Boolean | Whether expanded |

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `drag` | `{debateId, position}` | Node dragged |
| `delete` | `debateId` | Delete requested |
| `expand` | `debateId` | Expand requested |
| `collapse` | `debateId` | Collapse requested |
| `continue` | `{debateId, prompt}` | Continue debate |

---

### 10. PromptDialog.vue
**File:** `src/components/canvas/PromptDialog.vue`

**Purpose:** Dialog for creating new prompts with model selection.

**Features:**
- Model selector with provider grouping
- Prompt textarea
- System prompt input
- Temperature slider
- Max tokens input
- Context mode selector (Full History, Compact, Semantic)
- Branch context display (when branching)

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `submit` | `{prompt, models, systemPrompt, temperature, maxTokens, contextMode}` | Submit prompt |
| `cancel` | - | Dialog cancelled |

---

### 11. ModelSelector.vue
**File:** `src/components/canvas/ModelSelector.vue`

**Purpose:** Multi-select dropdown for AI models.

**Features:**
- Models grouped by provider
- Search/filter models
- Checkbox selection
- Show model pricing (optional)

---

### 12. PromptTile.vue
**File:** `src/components/canvas/PromptTile.vue`

**Purpose:** Legacy tile component (compatibility).

---

### 13. ModelResponseCard.vue
**File:** `src/components/canvas/ModelResponseCard.vue`

**Purpose:** Display model response with controls.

---

### 14. DebateTile.vue
**File:** `src/components/canvas/DebateTile.vue`

**Purpose:** Legacy debate tile component (compatibility).

---

### 15. DebateControls.vue
**File:** `src/components/canvas/DebateControls.vue`

**Purpose:** Controls for managing debates.

---

## Vue Router

**File:** `src/router/index.js`

```javascript
const routes = [
  { path: '/', component: HomeView },
  { path: '/canvas', component: CanvasView },
  { path: '/canvas/:id', component: CanvasView },
  { path: '/login', component: LoginView },
  { path: '/oauth/callback', component: OAuthCallbackView },
  { path: '/:pathMatch(.*)*', component: NotFoundView },
]
```

---

## API Client

**File:** `src/api/client.js`

**Base URL:** `/api` (proxied to backend in development)

**HTTP Client:** Axios with interceptors for auth token and error handling.

```javascript
// Notes API
export const notes = {
    list: () => api.get('/notes'),
    get: (id) => api.get(`/notes/${encodeURIComponent(id)}`),
    create: (data) => api.post('/notes', data),
    update: (id, data) => api.put(`/notes/${encodeURIComponent(id)}`, data),
    delete: (id) => api.delete(`/notes/${encodeURIComponent(id)}`),
    reindex: () => api.post('/notes/reindex'),
}

// Search API
export const search = {
    query: (q, { limit = 10, semantic = true } = {}) =>
        api.get('/search', { params: { q, limit, semantic } }),
    similar: (noteId, limit = 5) =>
        api.get(`/search/similar/${encodeURIComponent(noteId)}`, { params: { limit } }),
}

// Graph API
export const graph = {
    backlinks: (id) => api.get(`/graph/backlinks/${encodeURIComponent(id)}`),
    outgoing: (id) => api.get(`/graph/outgoing/${encodeURIComponent(id)}`),
    neighbors: (id, depth = 1) =>
        api.get(`/graph/neighbors/${encodeURIComponent(id)}`, { params: { depth } }),
    unlinkedMentions: (id) => api.get(`/graph/unlinked-mentions/${encodeURIComponent(id)}`),
    rebuild: () => api.post('/graph/rebuild'),
    full: () => api.get('/graph/full'),
}

// Auth API
export const oauth = {
    getAuthorizationUrl: (provider) => api.get(`/oauth/authorize/${provider}`),
    exchangeCode: (provider, code) => api.post(`/oauth/callback/${provider}`, { code }),
    getUser: () => api.get('/oauth/user'),
    logout: () => api.post('/oauth/logout'),
}

// Canvas API
export const canvas = {
    list: () => api.get('/canvas'),
    get: (id) => api.get(`/canvas/${encodeURIComponent(id)}`),
    create: (data) => api.post('/canvas', data),
    update: (id, data) => api.put(`/canvas/${encodeURIComponent(id)}`, data),
    delete: (id) => api.delete(`/canvas/${encodeURIComponent(id)}`),
    getModels: () => api.get('/canvas/models/available'),
    updateTilePosition: (sessionId, tileId, position) =>
        api.put(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/position`, position),
    updateLLMNodePosition: (sessionId, tileId, modelId, position) =>
        api.put(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}/responses/${encodeURIComponent(modelId)}/position`, position),
    autoArrange: (sessionId, positions) =>
        api.post(`/canvas/${encodeURIComponent(sessionId)}/arrange`, { positions }),
    deleteTile: (sessionId, tileId) =>
        api.delete(`/canvas/${encodeURIComponent(sessionId)}/tiles/${encodeURIComponent(tileId)}`),
    updateViewport: (sessionId, viewport) =>
        api.put(`/canvas/${encodeURIComponent(sessionId)}/viewport`, viewport),
    updateDebateStatus: (sessionId, debateId, status) =>
        api.put(`/canvas/${encodeURIComponent(sessionId)}/debate/${encodeURIComponent(debateId)}/status`, null, { params: { status } }),
    exportToNote: (sessionId) =>
        api.post(`/canvas/${encodeURIComponent(sessionId)}/export-note`),
    getNodeEdges: (sessionId) =>
        api.get(`/canvas/${encodeURIComponent(sessionId)}/node-edges`),
    getNodeGroups: (sessionId) =>
        api.get(`/canvas/${encodeURIComponent(sessionId)}/node-groups`),
}
```

---

## Design System

**File:** `src/style.css`

### Color Palette

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-primary` | `#0f0f10` | Main background |
| `--bg-secondary` | `#1a1a1d` | Sidebar, panels |
| `--bg-tertiary` | `#242428` | Cards, inputs |
| `--bg-hover` | `#2a2a2f` | Hover states |
| `--text-primary` | `#e8e8ed` | Main text |
| `--text-secondary` | `#a0a0a8` | Secondary text |
| `--text-muted` | `#6b6b73` | Muted text |
| `--accent-primary` | `#7c5cff` | Primary accent (purple) |
| `--accent-secondary` | `#5c8fff` | Secondary accent (blue) |
| `--accent-success` | `#34d399` | Success (green) |
| `--accent-warning` | `#fbbf24` | Warning (yellow) |
| `--accent-danger` | `#f87171` | Danger (red) |

### Typography

- **Font Family:** Inter (via Google Fonts)
- **Mono Font:** Fira Code

### Components

| Class | Usage |
|-------|-------|
| `.btn-primary` | Primary actions |
| `.btn-secondary` | Secondary actions |
| `.btn-ghost` | Subtle actions |
| `.tag` | Tag pills |
| `.status` | Status badges |
| `.wikilink` | Rendered wikilinks |
| `.card` | Card containers |
| `.card-hover` | Hoverable cards |

---

## Build Configuration

**File:** `frontend/vite.config.js`

```javascript
export default defineConfig({
    plugins: [vue()],
    server: {
        port: 5173,
        proxy: {
            '/api': {
                target: 'http://localhost:8080',
                changeOrigin: true,
            },
            '/sse': {
                target: 'http://localhost:8080',
                changeOrigin: true,
            },
            '/auth': {
                target: 'http://localhost:8080',
                changeOrigin: true,
            },
        },
    },
})
```

---

## Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| vue | ^3.4.0 | UI framework |
| pinia | Latest | State management |
| vue-router | ^4.2.0 | SPA routing |
| marked | ^11.0.0 | Markdown rendering |
| axios | Latest | HTTP client |
| d3 | ^7.0.0 | Graph visualization |
| @vitejs/plugin-vue | ^4.5.0 | Vue SFC support |
| vite | ^5.0.0 | Build tool |

---

## File Structure

```
frontend/
├── index.html              # HTML entry point
├── package.json            # Dependencies
├── vite.config.js          # Vite configuration
└── src/
    ├── main.js             # Vue app bootstrap
    ├── App.vue             # Root component
    ├── style.css           # Design system
    ├── router/
    │   └── index.js        # Route definitions
    ├── stores/
    │   ├── auth.js         # Auth state
    │   ├── notes.js        # Notes state
    │   └── canvas.js       # Canvas state
    ├── views/
    │   ├── HomeView.vue    # Main app
    │   ├── CanvasView.vue  # Canvas sessions
    │   ├── LoginView.vue   # Login page
    │   ├── OAuthCallbackView.vue  # OAuth handler
    │   └── NotFoundView.vue  # 404 page
    ├── api/
    │   └── client.js       # Backend API client
    └── components/
        ├── SearchBar.vue
        ├── NoteList.vue
        ├── NoteEditor.vue
        ├── BacklinksPanel.vue
        ├── GraphView.vue
        └── canvas/
            ├── index.js
            ├── CanvasContainer.vue
            ├── PromptNode.vue
            ├── LLMNode.vue
            ├── DebateNode.vue
            ├── PromptDialog.vue
            ├── ModelSelector.vue
            ├── PromptTile.vue
            ├── ModelResponseCard.vue
            ├── DebateTile.vue
            └── DebateControls.vue
```
