# Seedream Frontend Architecture

> **Part:** Frontend | **Type:** Vue 3 SPA | **Scan Level:** Exhaustive

## Overview

The frontend is a Vue 3 Single Page Application providing:
- Note listing and navigation
- Markdown editing with preview
- Semantic search with typeahead
- Backlink visualization
- Authentication (GitHub OAuth)
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

## Views (4 Total)

### 1. HomeView.vue
**File:** `src/views/HomeView.vue`

**Purpose:** Main application view with note management.

**Layout:**
- **Header:** Logo, SearchBar, action buttons
- **Sidebar (280px):** NoteList
- **Editor Area (flex):** NoteEditor or empty state
- **Right Panel (300px):** BacklinksPanel (when note selected)

---

### 2. LoginView.vue
**File:** `src/views/LoginView.vue`

**Purpose:** Authentication page with GitHub OAuth.

---

### 3. OAuthCallbackView.vue
**File:** `src/views/OAuthCallbackView.vue`

**Purpose:** Handles OAuth callback from GitHub, exchanges code for token.

---

### 4. NotFoundView.vue
**File:** `src/views/NotFoundView.vue`

**Purpose:** 404 page for unmatched routes.

---

## Pinia Stores (2 Stores)

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

## Components (5 Total)

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

## Vue Router

**File:** `src/router/index.js`

```javascript
const routes = [
  { path: '/', component: HomeView },
  { path: '/login', component: LoginView },
  { path: '/oauth/callback', component: OAuthCallbackView },
  { path: '/:pathMatch(.*)*', component: NotFoundView },
]
```

---

## API Client

**File:** `src/api/client.js`

**Base URL:** `/api` (proxied to backend in development)

```javascript
// Notes API
export const notes = {
    list: () => request('/notes'),
    get: (id) => request(`/notes/${encodeURIComponent(id)}`),
    create: (data) => request('/notes', { method: 'POST', body: data }),
    update: (id, data) => request(`/notes/${encodeURIComponent(id)}`, { method: 'PUT', body: data }),
    delete: (id) => request(`/notes/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    reindex: () => request('/notes/reindex', { method: 'POST' }),
}

// Search API
export const search = {
    query: (q, { limit = 10, semantic = true } = {}) => ...,
    similar: (noteId, limit = 5) => ...,
}

// Graph API
export const graph = {
    backlinks: (id) => ...,
    outgoing: (id) => ...,
    neighbors: (id, depth = 1) => ...,
    unlinkedMentions: (id) => ...,
    rebuild: () => ...,
}

// Auth API
export const auth = {
    getGithubUrl: () => ...,
    callback: (code) => ...,
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
    │   └── notes.js        # Notes state
    ├── views/
    │   ├── HomeView.vue    # Main app
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
        └── GraphView.vue
```
