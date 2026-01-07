# Seedream Component Inventory - Frontend

> **Part:** Frontend | **Components:** 5 | **Views:** 4 | **Stores:** 2 | **Scan Level:** Exhaustive

## Summary

| Type | Count | Purpose |
|------|-------|---------|
| Components | 5 | Reusable UI elements |
| Views | 4 | Page-level components |
| Stores | 2 | Pinia state management |

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
| `select` | `note_id` | Emitted when result selected |

---

### 2. NoteList.vue

**File:** `src/components/NoteList.vue`

**Purpose:** Sidebar component displaying all notes with metadata.

**Props:**
| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `notes` | Array | `[]` | List of NoteListItem |
| `selected` | String | `null` | Selected note ID |

**Features:**
- Shows note title, status badge, link count
- Up to 3 tags displayed
- Selected note highlighted with accent border
- Hover states

---

### 3. NoteEditor.vue

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

### 4. BacklinksPanel.vue

**File:** `src/components/BacklinksPanel.vue`

**Purpose:** Right panel showing notes that link to current note.

**Props:**
| Prop | Type | Required | Description |
|------|------|----------|-------------|
| `noteId` | String | ✅ | Current note ID |

**Features:**
- Loads backlinks via `/api/graph/backlinks/{id}`
- Shows source title and context snippet
- Click to navigate to linking note
- Loading state

---

### 5. GraphView.vue

**File:** `src/components/GraphView.vue`

**Purpose:** Graph visualization of note connections.

**Features:**
- Local neighborhood visualization
- Interactive node exploration
- Configurable depth traversal

---

## Views (4 Total)

### 1. HomeView.vue

**File:** `src/views/HomeView.vue`

**Purpose:** Main application view with note management.

**Layout:**
```
┌──────────────────────────────────────────────────────────────┐
│  Header: Logo | SearchBar | Reindex | New Note               │
├──────────────┬───────────────────────┬───────────────────────┤
│   Sidebar    │      Editor Area      │    Right Panel        │
│   (280px)    │       (flex)          │      (300px)          │
│              │                       │                       │
│  NoteList    │   NoteEditor          │   BacklinksPanel      │
│              │   or Empty State      │   (when note          │
│              │                       │    selected)          │
│              │                       │                       │
└──────────────┴───────────────────────┴───────────────────────┘
```

**Uses Stores:** `notes.js`

---

### 2. LoginView.vue

**File:** `src/views/LoginView.vue`

**Purpose:** Login page with GitHub OAuth button.

**Features:**
- Clean login form design
- GitHub OAuth initiation
- Error handling display

**Uses Stores:** `auth.js`

---

### 3. OAuthCallbackView.vue

**File:** `src/views/OAuthCallbackView.vue`

**Purpose:** Handles OAuth callback from GitHub.

**Flow:**
1. Receives `code` from URL query params
2. Exchanges code for access token
3. Stores token in auth store
4. Redirects to home view

**Uses Stores:** `auth.js`

---

### 4. NotFoundView.vue

**File:** `src/views/NotFoundView.vue`

**Purpose:** 404 error page for unmatched routes.

**Features:**
- Friendly error message
- Link back to home

---

## Pinia Stores (2 Total)

### 1. auth.js

**File:** `src/stores/auth.js`

**Purpose:** Authentication state management.

**State:**
```javascript
{
  user: null,              // User data from OAuth
  token: null,             // Access token
  isAuthenticated: false,  // Auth status
  loading: false,          // Loading state
}
```

**Actions:**
| Action | Description |
|--------|-------------|
| `login()` | Initiate GitHub OAuth flow |
| `handleCallback(code)` | Exchange code for token |
| `logout()` | Clear authentication |
| `checkAuth()` | Verify token validity |

**Getters:**
| Getter | Returns |
|--------|---------|
| `isLoggedIn` | Boolean auth status |
| `currentUser` | User object or null |

---

### 2. notes.js

**File:** `src/stores/notes.js`

**Purpose:** Notes state management.

**State:**
```javascript
{
  notes: [],               // All notes list
  selectedNoteId: null,    // Current note ID
  selectedNote: null,      // Full note object
  loading: false,          // Loading state
  indexing: false,         // Reindex in progress
}
```

**Actions:**
| Action | Description |
|--------|-------------|
| `loadNotes()` | Fetch all notes from API |
| `selectNote(id)` | Load and display a note |
| `createNote(data)` | Create new note |
| `updateNote(id, data)` | Update note content |
| `deleteNote(id)` | Delete note |
| `reindex()` | Trigger full reindex |

**Getters:**
| Getter | Returns |
|--------|---------|
| `noteCount` | Number of notes |
| `hasSelectedNote` | Boolean |

---

## Component Dependencies

```
App.vue
└── router-view
    ├── HomeView.vue
    │   ├── SearchBar.vue
    │   │   └── api/client.js (search.query)
    │   ├── NoteList.vue
    │   │   └── stores/notes.js
    │   ├── NoteEditor.vue
    │   │   ├── marked (markdown rendering)
    │   │   └── stores/notes.js
    │   └── BacklinksPanel.vue
    │       └── api/client.js (graph.backlinks)
    ├── LoginView.vue
    │   └── stores/auth.js
    ├── OAuthCallbackView.vue
    │   └── stores/auth.js
    └── NotFoundView.vue
```

---

## Styling Conventions

All components use scoped CSS with design system tokens:

```css
<style scoped>
.component {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  padding: var(--spacing-md);
  transition: all var(--transition-fast);
}

.component:hover {
  background: var(--bg-hover);
}
</style>
```

### Status Colors

| Status | Background | Text |
|--------|------------|------|
| `canonical` | `rgba(52, 211, 153, 0.15)` | Green |
| `draft` | `rgba(251, 191, 36, 0.15)` | Yellow |
| `evidence` | `rgba(124, 92, 255, 0.15)` | Purple |
