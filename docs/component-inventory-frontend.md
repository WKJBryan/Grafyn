# Grafyn Component Inventory - Frontend

> **Part:** Frontend | **Components:** 14 | **Views:** 5 | **Stores:** 3 | **Scan Level:** Exhaustive

## Summary

| Type | Count | Purpose |
|------|-------|---------|
| Components | 14 | Reusable UI elements |
| Views | 5 | Page-level components |
| Stores | 3 | Pinia state management |

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
  /\[\[([^\]]+)(?:\|[^\]]+)?\]\]/g,
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

## Views (5 Total)

### 1. HomeView.vue

**File:** `src/views/HomeView.vue`

**Purpose:** Main application view with note management.

**Layout:**
```
┌──────────────────────────────────────────────────────────────┐
│  Header: Logo | SearchBar | Reindex | New Note               │
├──────────────┬───────────────────────┬───────────────────────┤
│  Sidebar    │      Editor Area      │    Right Panel        │
│  (280px)    │       (flex)          │      (300px)          │
│              │                       │                       │
│  NoteList    │   NoteEditor          │   BacklinksPanel      │
│              │   or Empty State      │   (when note          │
│              │                       │    selected)          │
└──────────────┴───────────────────────┴───────────────────────┘
```

**Uses Stores:** `notes.js`

---

### 2. CanvasView.vue

**File:** `src/views/CanvasView.vue`

**Purpose:** Multi-LLM canvas session management.

**Layout:**
- **Sidebar (280px):** Session list with create/delete actions
- **Main Area:** CanvasContainer for active session
- **Empty State:** "Create New Canvas" when no session selected

**Features:**
- Session list with tile/debate counts
- Create new session dialog
- Delete session confirmation
- Theme toggle button
- Navigation back to notes

**Uses Stores:** `canvas.js`

---

### 3. LoginView.vue

**File:** `src/views/LoginView.vue`

**Purpose:** Login page with GitHub OAuth button.

**Features:**
- Clean login form design
- GitHub OAuth initiation
- Error handling display

**Uses Stores:** `auth.js`

---

### 4. OAuthCallbackView.vue

**File:** `src/views/OAuthCallbackView.vue`

**Purpose:** Handles OAuth callback from GitHub.

**Flow:**
1. Receives `code` from URL query params
2. Exchanges code for token
3. Stores token in auth store
4. Redirects to home view

**Uses Stores:** `auth.js`

---

### 5. NotFoundView.vue

**File:** `src/views/NotFoundView.vue`

**Purpose:** 404 error page for unmatched routes.

**Features:**
- Friendly error message
- Link back to home

---

## Pinia Stores (3 Total)

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

### 3. canvas.js

**File:** `src/stores/canvas.js`

**Purpose:** Canvas session and streaming state management.

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
| Getter | Returns |
|--------|---------|
| `promptTiles` | Current session's prompt tiles |
| `debates` | Current session's debates |
| `hasSession` | Whether a session is active |
| `isStreaming` | Whether any model is streaming |
| `modelsByProvider` | Models grouped by provider |
| `tileEdges` | Parent-child tile edges |
| `debateEdges` | Debate connection edges |

**Actions:**
| Action | Description |
|--------|-------------|
| `loadSessions()` | Fetch all sessions |
| `loadSession(id)` | Load specific session |
| `createSession(data)` | Create new session |
| `updateSession(id, data)` | Update session metadata |
| `deleteSession(id)` | Delete session |
| `loadModels()` | Fetch available models from OpenRouter |
| `sendPrompt(...)` | Send prompt to multiple models (SSE streaming) |
| `updateTilePosition(tileId, position)` | Update tile position |
| `updateLLMNodePosition(tileId, modelId, position)` | Update LLM node position |
| `autoArrange(positions)` | Batch update positions |
| `deleteTile(tileId)` | Delete tile |
| `updateViewport(viewport)` | Save viewport state |
| `startDebate(tileIds, models, mode, maxRounds)` | Start debate (SSE) |
| `continueDebate(debateId, prompt)` | Continue debate |
| `saveAsNote()` | Export canvas to markdown note |
| `branchFromResponse(...)` | Branch from specific model response |

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
    │   ├── BacklinksPanel.vue
    │   │   └── api/client.js (graph.backlinks)
    │   └── GraphView.vue
    │       └── d3 (graph visualization)
    ├── CanvasView.vue
    │   ├── CanvasContainer.vue
    │   │   ├── PromptNode.vue
    │   │   ├── LLMNode.vue
    │   │   ├── DebateNode.vue
    │   │   ├── PromptDialog.vue
    │   │   ├── ModelSelector.vue
    │   │   ├── PromptTile.vue (legacy)
    │   │   ├── ModelResponseCard.vue
    │   │   ├── DebateTile.vue (legacy)
    │   │   └── DebateControls.vue
    │   │   └── stores/canvas.js
    │   │       └── api/client.js (canvas.*)
    │   └── stores/theme.js
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
