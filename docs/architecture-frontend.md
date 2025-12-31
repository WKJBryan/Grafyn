# OrgAI Frontend Architecture

> **Part:** Frontend | **Type:** Vue 3 SPA | **Scan Level:** Exhaustive

## Overview

The frontend is a Vue 3 Single Page Application providing:
- Note listing and navigation
- Markdown editing with preview
- Semantic search with typeahead
- Backlink visualization
- Clean dark theme design system

## Entry Point

**File:** `frontend/src/main.js`

```javascript
import { createApp } from 'vue'
import App from './App.vue'
import './style.css'

createApp(App).mount('#app')
```

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                            App.vue (Root)                             │
│  ┌────────────┐  ┌─────────────────────┐  ┌────────────────────────┐ │
│  │  Header    │  │    Main Content     │  │     Right Panel        │ │
│  │            │  │                     │  │                        │ │
│  │ SearchBar  │  │ ┌─────────────────┐ │  │ ┌────────────────────┐ │ │
│  │            │  │ │    Sidebar      │ │  │ │  BacklinksPanel    │ │ │
│  │            │  │ │   (NoteList)    │ │  │ │                    │ │ │
│  │            │  │ └─────────────────┘ │  │ └────────────────────┘ │ │
│  │            │  │                     │  │                        │ │
│  │            │  │ ┌─────────────────┐ │  │                        │ │
│  │            │  │ │   NoteEditor    │ │  │                        │ │
│  │            │  │ │   or Empty      │ │  │                        │ │
│  │            │  │ │   State         │ │  │                        │ │
│  │            │  │ └─────────────────┘ │  │                        │ │
│  └────────────┘  └─────────────────────┘  └────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────┘
```

## Components (6 Total)

### 1. App.vue (Root Component)
**File:** `src/App.vue`

**Purpose:** Root layout with header, sidebar, editor, and backlinks panel.

**State:**
```javascript
const notes = ref([])           // All notes list
const selectedNoteId = ref(null) // Currently selected
const selectedNote = ref(null)   // Full note object
const indexing = ref(false)      // Reindex in progress
```

**Layout:**
- **Header:** Logo, SearchBar, action buttons
- **Sidebar (280px):** NoteList
- **Editor Area (flex):** NoteEditor or empty state
- **Right Panel (300px):** BacklinksPanel (when note selected)

---

### 2. SearchBar.vue
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

### 3. NoteList.vue
**File:** `src/components/NoteList.vue`

**Purpose:** Sidebar listing of all notes with status and tags.

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `notes` | Array | List of NoteListItem objects |
| `selected` | String | Currently selected note ID |

**Features:**
- Shows note title, status badge, link count
- Up to 3 tags displayed
- Selected note highlighted with accent border
- Hover states

---

### 4. NoteEditor.vue
**File:** `src/components/NoteEditor.vue`

**Purpose:** Markdown editor with preview mode.

**Features:**
- Edit/Preview toggle
- Title editing
- Markdown content textarea
- Wikilink rendering in preview (styled `<span>` elements)
- Save button (enabled when dirty)
- Delete button

**Wikilink Rendering:**
```javascript
html = html.replace(
  /\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g,
  (match, target, display) => {
    const text = display || target
    return `<span class="wikilink" data-target="${target}">${text}</span>`
  }
)
```

**Events:**
| Event | Payload | Description |
|-------|---------|-------------|
| `save` | `(id, data)` | Save note changes |
| `delete` | `id` | Delete note |

---

### 5. BacklinksPanel.vue
**File:** `src/components/BacklinksPanel.vue`

**Purpose:** Right panel showing notes that link to current note.

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `noteId` | String | Current note ID |

**Features:**
- Loads backlinks via `/api/graph/backlinks/{id}`
- Shows source title and context snippet
- Click to navigate to linking note
- Loading state

---

### 6. GraphView.vue (Placeholder)
**File:** `src/components/GraphView.vue`

**Purpose:** Placeholder for Phase 2 graph visualization.

**Status:** Not yet implemented - shows "Coming in Phase 2" message.

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

### Status Colors

| Status | Background | Text |
|--------|------------|------|
| `canonical` | `rgba(52, 211, 153, 0.15)` | Green |
| `draft` | `rgba(251, 191, 36, 0.15)` | Yellow |
| `evidence` | `rgba(124, 92, 255, 0.15)` | Purple |

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
            '/mcp': {
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
| vue-router | ^4.2.0 | SPA routing (ready for use) |
| axios | ^1.6.0 | HTTP client |
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
    ├── api/
    │   └── client.js       # Backend API client
    └── components/
        ├── SearchBar.vue   # Semantic search
        ├── NoteList.vue    # Sidebar listing
        ├── NoteEditor.vue  # Markdown editor
        ├── BacklinksPanel.vue  # Backlinks panel
        └── GraphView.vue   # Graph (placeholder)
```
