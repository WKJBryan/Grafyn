# OrgAI Component Inventory - Frontend

> **Part:** Frontend | **Components:** 6 | **Scan Level:** Exhaustive

## Component Summary

| Component | Lines | Purpose | Status |
|-----------|-------|---------|--------|
| App.vue | 261 | Root layout and state management | ✅ Complete |
| SearchBar.vue | 218 | Semantic search with typeahead | ✅ Complete |
| NoteEditor.vue | 263 | Markdown editor with preview | ✅ Complete |
| NoteList.vue | 140 | Sidebar note listing | ✅ Complete |
| BacklinksPanel.vue | 136 | Backlinks display panel | ✅ Complete |
| GraphView.vue | 68 | Graph visualization | ⏳ Phase 2 |

---

## App.vue (Root Component)

**File:** `src/App.vue` | **Lines:** 261

### Template Structure
```html
<div class="app">
  <header class="header">
    <div class="header-left">
      <h1 class="logo">◈ OrgAI</h1>
    </div>
    <div class="header-center">
      <SearchBar @select="openNote" />
    </div>
    <div class="header-right">
      <button @click="reindex">⟳ Reindex</button>
      <button @click="createNote">+ New Note</button>
    </div>
  </header>

  <main class="main">
    <aside class="sidebar">
      <NoteList :notes="notes" :selected="selectedNoteId" @select="openNote" />
    </aside>

    <section class="editor-area">
      <NoteEditor v-if="selectedNote" :note="selectedNote" @save="saveNote" @delete="deleteNote" />
      <div v-else class="empty-state">📝 No note selected</div>
    </section>

    <aside class="right-panel" v-if="selectedNote">
      <BacklinksPanel :noteId="selectedNoteId" @select="openNote" />
    </aside>
  </main>
</div>
```

### State Management
```javascript
const notes = ref([])             // All notes from API
const selectedNoteId = ref(null)  // Current note ID
const selectedNote = ref(null)    // Full note object
const indexing = ref(false)       // Reindex loading state
```

### Methods
| Method | Description |
|--------|-------------|
| `loadNotes()` | Fetch all notes from API |
| `openNote(id)` | Load and display a note |
| `createNote()` | Prompt for title and create |
| `saveNote(id, data)` | Update note content |
| `deleteNote(id)` | Delete with confirmation |
| `reindex()` | Trigger full reindex |

---

## SearchBar.vue

**File:** `src/components/SearchBar.vue` | **Lines:** 218

### Features
- **Debounced Search:** 300ms delay before API call
- **Typeahead Dropdown:** Shows top 5 results
- **Score Visualization:** Gradient bar shows relevance
- **Keyboard Support:** Enter selects first, Escape clears

### Template
```html
<div class="search-bar">
  <input v-model="query" @input="onInput" @keydown.enter="onSubmit"
         placeholder="Search notes semantically..." />
  
  <div v-if="showResults && results.length" class="search-results">
    <div v-for="result in results" @click="selectResult(result)">
      <div class="result-title">{{ result.title }}</div>
      <div class="result-snippet">{{ result.snippet }}</div>
      <div class="result-score">
        <span class="score-bar" :style="{ width: (result.score * 100) + '%' }"></span>
      </div>
    </div>
  </div>
</div>
```

### Props & Events
| Event | Payload | Description |
|-------|---------|-------------|
| `select` | `note_id` | Emitted when result selected |

---

## NoteEditor.vue

**File:** `src/components/NoteEditor.vue` | **Lines:** 263

### Features
- **Edit/Preview Toggle:** Switch between raw markdown and rendered
- **Title Editing:** Inline title input
- **Dirty Tracking:** Save button enables when changed
- **Wikilink Rendering:** Converts `[[links]]` to styled spans
- **Markdown Support:** Full rendering via `marked`

### Props
| Prop | Type | Required | Description |
|------|------|----------|-------------|
| `note` | Object | ✅ | Full note object |

### Events
| Event | Payload | Description |
|-------|---------|-------------|
| `save` | `(id, { title, content })` | Save changes |
| `delete` | `id` | Delete note |

### Wikilink Rendering
```javascript
const renderedContent = computed(() => {
  let html = marked.parse(content.value || '')
  
  // Convert [[wikilinks]] to styled spans
  html = html.replace(
    /\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g,
    (match, target, display) => {
      const text = display || target
      return `<span class="wikilink" data-target="${target}">${text}</span>`
    }
  )
  
  return html
})
```

---

## NoteList.vue

**File:** `src/components/NoteList.vue` | **Lines:** 140

### Purpose
Sidebar component displaying all notes with metadata.

### Props
| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `notes` | Array | `[]` | List of NoteListItem |
| `selected` | String | `null` | Selected note ID |

### Template
```html
<div class="note-list">
  <div class="list-header">
    <h3>Notes</h3>
    <span class="note-count">{{ notes.length }}</span>
  </div>
  
  <div class="list-items">
    <div v-for="note in notes" :class="['note-item', { selected: note.id === selected }]"
         @click="$emit('select', note.id)">
      <div class="note-title">{{ note.title }}</div>
      <div class="note-meta">
        <span :class="['status', `status-${note.status}`]">{{ note.status }}</span>
        <span v-if="note.link_count" class="link-count">{{ note.link_count }} links</span>
      </div>
      <div v-if="note.tags.length" class="note-tags">
        <span v-for="tag in note.tags.slice(0, 3)" class="tag">{{ tag }}</span>
      </div>
    </div>
  </div>
</div>
```

---

## BacklinksPanel.vue

**File:** `src/components/BacklinksPanel.vue` | **Lines:** 136

### Purpose
Right panel showing notes that link to the current note.

### Props
| Prop | Type | Required | Description |
|------|------|----------|-------------|
| `noteId` | String | ✅ | Current note to find backlinks for |

### Lifecycle
```javascript
onMounted(loadBacklinks)
watch(() => props.noteId, loadBacklinks)
```

### Template
```html
<div class="backlinks-panel">
  <div class="panel-header">
    <h3>Backlinks</h3>
    <span class="count">{{ backlinks.length }}</span>
  </div>

  <div v-if="loading" class="loading">Loading...</div>
  
  <div v-else-if="backlinks.length === 0" class="empty">
    No notes link to this one yet
  </div>
  
  <div v-else class="backlinks-list">
    <div v-for="backlink in backlinks" @click="$emit('select', backlink.source_id)">
      <div class="backlink-title">{{ backlink.source_title }}</div>
      <div class="backlink-context">{{ backlink.context }}</div>
    </div>
  </div>
</div>
```

---

## GraphView.vue (Placeholder)

**File:** `src/components/GraphView.vue` | **Lines:** 68

### Status
⏳ **Not Implemented** - Placeholder for Phase 2

### Planned Features
- Local neighborhood visualization
- D3.js or vis-network based
- Interactive node exploration
- Configurable depth traversal

### Current Template
```html
<div class="graph-view">
  <div class="graph-header">
    <h3>Graph View</h3>
    <span class="text-muted">Coming in Phase 2</span>
  </div>
  <div class="graph-placeholder">
    <div class="placeholder-icon">🕸️</div>
    <p>Graph visualization will be available in Phase 2</p>
  </div>
</div>
```

---

## Component Dependencies

```
App.vue
├── SearchBar.vue
│   └── api/client.js (search.query)
├── NoteList.vue
│   └── (no external deps)
├── NoteEditor.vue
│   └── marked (markdown rendering)
└── BacklinksPanel.vue
    └── api/client.js (graph.backlinks)
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
