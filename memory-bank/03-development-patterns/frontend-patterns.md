# Frontend Development Patterns

> **Purpose:** Document common patterns and best practices for OrgAI frontend development
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document describes common patterns and conventions used in OrgAI frontend (Vue 3) to ensure consistency and maintainability.

## Component Pattern

### Pattern Description

Use Vue 3 Composition API with `<script setup>` syntax for components.

### Implementation

```vue
<!-- components/NoteEditor.vue -->
<template>
  <div class="note-editor">
    <input 
      v-model="title" 
      class="note-title"
      placeholder="Note title"
    />
    <textarea 
      v-model="content" 
      class="note-content"
      placeholder="Start writing..."
    />
    <div class="note-actions">
      <button 
        @click="save" 
        :disabled="!isDirty"
        class="btn-primary"
      >
        Save
      </button>
      <button @click="cancel" class="btn-secondary">
        Cancel
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'

const props = defineProps({
  noteId: {
    type: String,
    required: true
  },
  note: {
    type: Object,
    default: null
  }
})

const emit = defineEmits(['save', 'cancel'])

// State
const title = ref('')
const content = ref('')
const originalTitle = ref('')
const originalContent = ref('')

// Computed
const isDirty = computed(() => 
  title.value !== originalTitle.value || 
  content.value !== originalContent.value
)

// Methods
const save = () => {
  emit('save', props.noteId, {
    title: title.value,
    content: content.value
  })
}

const cancel = () => {
  emit('cancel')
}

// Watchers
watch(() => props.note, (newNote) => {
  if (newNote) {
    title.value = newNote.title
    content.value = newNote.content
    originalTitle.value = newNote.title
    originalContent.value = newNote.content
  }
}, { immediate: true })
</script>

<style scoped>
.note-editor {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.note-title {
  font-size: 1.5rem;
  padding: 0.5rem;
  border: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
  color: var(--text-primary);
}

.note-content {
  flex: 1;
  padding: 1rem;
  border: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
  color: var(--text-primary);
  resize: none;
  min-height: 300px;
}

.note-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
}
</style>
```

## Props and Emits Pattern

### Pattern Description

Define props with validation and emits for component communication.

### Implementation

```vue
<script setup>
import { defineProps, defineEmits } from 'vue'

// Props with validation
const props = defineProps({
  noteId: {
    type: String,
    required: true,
    validator: (value) => value.length > 0
  },
  note: {
    type: Object,
    default: null,
    // Custom validator
    validator: (value) => {
      if (value === null) return true
      return 'id' in value && 'title' in value
    }
  },
  limit: {
    type: Number,
    default: 10,
    validator: (value) => value >= 1 && value <= 100
  }
})

// Emits
const emit = defineEmits({
  // No validation
  'click': null,
  // With validation
  'save': (payload) => {
    if (!payload.id || !payload.title) {
      console.warn('Invalid save payload')
      return false
    }
    return true
  },
  'delete': (id) => typeof id === 'string'
})
</script>
```

## Computed Properties Pattern

### Pattern Description

Use computed properties for derived state.

### Implementation

```vue
<script setup>
import { ref, computed } from 'vue'

const notes = ref([])
const filter = ref('')
const selectedNoteId = ref(null)

// Simple computed
const noteCount = computed(() => notes.value.length)

// Filtered computed
const filteredNotes = computed(() => {
  if (!filter.value) return notes.value
  const query = filter.value.toLowerCase()
  return notes.value.filter(note => 
    note.title.toLowerCase().includes(query) ||
    note.tags.some(tag => tag.toLowerCase().includes(query))
  )
})

// Complex computed with memoization
const selectedNote = computed(() => {
  if (!selectedNoteId.value) return null
  return notes.value.find(note => note.id === selectedNoteId.value)
})

// Computed with getter and setter
const searchQuery = computed({
  get: () => filter.value,
  set: (value) => {
    filter.value = value
    // Debounce search
    clearTimeout(debounceTimer)
    debounceTimer = setTimeout(() => {
      performSearch()
    }, 300)
  }
})
</script>
```

## Watchers Pattern

### Pattern Description

Use watchers for side effects and async operations.

### Implementation

```vue
<script setup>
import { ref, watch, onMounted } from 'vue'

const notes = ref([])
const loading = ref(false)
const error = ref(null)

// Simple watcher
watch(selectedNoteId, (newId, oldId) => {
  console.log(`Note changed from ${oldId} to ${newId}`)
  loadNote(newId)
})

// Watcher with options
watch(notes, (newNotes, oldNotes) => {
  if (newNotes.length !== oldNotes.length) {
    console.log('Notes count changed')
  }
}, { deep: true })

// Watcher with immediate execution
watch(() => props.noteId, async (noteId) => {
  if (noteId) {
    loading.value = true
    error.value = null
    try {
      const note = await notesApi.get(noteId)
      currentNote.value = note
    } catch (e) {
      error.value = 'Failed to load note'
    } finally {
      loading.value = false
    }
  }
}, { immediate: true })

// Multiple watchers
watch([filter, sortBy], ([newFilter, newSortBy]) => {
  console.log(`Filter: ${newFilter}, Sort: ${newSortBy}`)
  applyFiltersAndSort()
})
</script>
```

## Lifecycle Hooks Pattern

### Pattern Description

Use lifecycle hooks for initialization and cleanup.

### Implementation

```vue
<script setup>
import { ref, onMounted, onUnmounted, onBeforeUnmount } from 'vue'

const data = ref(null)
const timer = ref(null)

onMounted(() => {
  console.log('Component mounted')
  loadData()
  startPolling()
})

onBeforeUnmount(() => {
  console.log('Component about to unmount')
  cleanup()
})

onUnmounted(() => {
  console.log('Component unmounted')
  // Final cleanup
})

const loadData = async () => {
  data.value = await fetchData()
}

const startPolling = () => {
  timer.value = setInterval(() => {
    refreshData()
  }, 5000)
}

const cleanup = () => {
  if (timer.value) {
    clearInterval(timer.value)
    timer.value = null
  }
}
</script>
```

## API Client Pattern

### Pattern Description

Centralize API calls in a dedicated module.

### Implementation

```javascript
// api/client.js
import axios from 'axios'

const API_BASE = '/api'

const request = async (url, options = {}) => {
  try {
    const response = await axios({
      url: `${API_BASE}${url}`,
      ...options
    })
    return response.data
  } catch (error) {
    console.error('API request failed:', error)
    throw error
  }
}

// Notes API
export const notes = {
  list: () => request('/notes'),
  
  get: (id) => request(`/notes/${encodeURIComponent(id)}`),
  
  create: (data) => request('/notes', {
    method: 'POST',
    data
  }),
  
  update: (id, data) => request(`/notes/${encodeURIComponent(id)}`, {
    method: 'PUT',
    data
  }),
  
  delete: (id) => request(`/notes/${encodeURIComponent(id)}`, {
    method: 'DELETE'
  }),
  
  reindex: () => request('/notes/reindex', {
    method: 'POST'
  })
}

// Search API
export const search = {
  query: (q, { limit = 10, semantic = true } = {}) => 
    request(`/search?q=${encodeURIComponent(q)}&limit=${limit}&semantic=${semantic}`),
  
  similar: (noteId, limit = 5) => 
    request(`/search/similar/${encodeURIComponent(noteId)}?limit=${limit}`)
}

// Graph API
export const graph = {
  backlinks: (id) => request(`/graph/backlinks/${encodeURIComponent(id)}`),
  
  outgoing: (id) => request(`/graph/outgoing/${encodeURIComponent(id)}`),
  
  neighbors: (id, depth = 1) => 
    request(`/graph/neighbors/${encodeURIComponent(id)}?depth=${depth}`),
  
  unlinkedMentions: (id) => 
    request(`/graph/unlinked-mentions/${encodeURIComponent(id)}`),
  
  rebuild: () => request('/graph/rebuild', { method: 'POST' })
}
```

## Debounce Pattern

### Pattern Description

Debounce user input to reduce API calls.

### Implementation

```vue
<script setup>
import { ref } from 'vue'

const searchQuery = ref('')
const results = ref([])
const loading = ref(false)

let debounceTimer = null

const handleSearch = (query) => {
  clearTimeout(debounceTimer)
  
  debounceTimer = setTimeout(async () => {
    if (query.trim()) {
      loading.value = true
      try {
        results.value = await searchApi.query(query)
      } finally {
        loading.value = false
      }
    } else {
      results.value = []
    }
  }, 300) // 300ms delay
}

// In template
<input 
  v-model="searchQuery" 
  @input="handleSearch"
  placeholder="Search notes..."
/>
```

## Loading States Pattern

### Pattern Description

Show loading indicators during async operations.

### Implementation

```vue
<template>
  <div class="note-list">
    <!-- Loading state -->
    <div v-if="loading" class="loading">
      <div class="spinner"></div>
      <p>Loading notes...</p>
    </div>
    
    <!-- Error state -->
    <div v-else-if="error" class="error">
      <p>{{ error }}</p>
      <button @click="retry" class="btn-primary">Retry</button>
    </div>
    
    <!-- Empty state -->
    <div v-else-if="notes.length === 0" class="empty">
      <p>No notes found</p>
      <button @click="createNote" class="btn-primary">Create Note</button>
    </div>
    
    <!-- Content -->
    <div v-else class="notes">
      <div 
        v-for="note in notes" 
        :key="note.id"
        class="note-item"
        :class="{ active: selectedNoteId === note.id }"
        @click="selectNote(note.id)"
      >
        <h3>{{ note.title }}</h3>
        <div class="note-meta">
          <span class="status" :class="note.status">{{ note.status }}</span>
          <span class="tags">{{ note.tags.join(', ') }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { notes } from '@/api/client'

const loading = ref(false)
const error = ref(null)
const notes = ref([])
const selectedNoteId = ref(null)

const loadNotes = async () => {
  loading.value = true
  error.value = null
  try {
    notes.value = await notes.list()
  } catch (e) {
    error.value = 'Failed to load notes'
  } finally {
    loading.value = false
  }
}

const retry = () => {
  loadNotes()
}

onMounted(() => {
  loadNotes()
})
</script>

<style scoped>
.loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  padding: 2rem;
}

.spinner {
  width: 40px;
  height: 40px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.error {
  padding: 1rem;
  background: var(--accent-danger);
  color: white;
  border-radius: 4px;
}

.empty {
  text-align: center;
  padding: 2rem;
  color: var(--text-muted);
}
</style>
```

## Error Handling Pattern

### Pattern Description

Handle errors gracefully with user feedback.

### Implementation

```vue
<script setup>
import { ref } from 'vue'

const error = ref(null)
const showError = ref(false)

const handleError = (err) => {
  console.error('Error:', err)
  
  if (err.response) {
    // Server responded with error
    error.value = err.response.data.detail || 'Server error'
  } else if (err.request) {
    // Request made but no response
    error.value = 'Network error. Please check your connection.'
  } else {
    // Error in request setup
    error.value = 'An unexpected error occurred'
  }
  
  showError.value = true
  
  // Auto-hide after 5 seconds
  setTimeout(() => {
    showError.value = false
  }, 5000)
}

const clearError = () => {
  error.value = null
  showError.value = false
}

// Usage
const saveNote = async () => {
  try {
    await notesApi.update(noteId.value, noteData)
    clearError()
  } catch (err) {
    handleError(err)
  }
}
</script>
```

## Conditional Rendering Pattern

### Pattern Description

Use v-if, v-else-if, v-else for conditional rendering.

### Implementation

```vue
<template>
  <div class="note-editor">
    <!-- Edit mode -->
    <div v-if="isEditing" class="edit-mode">
      <textarea v-model="content" />
      <button @click="save">Save</button>
    </div>
    
    <!-- View mode -->
    <div v-else-if="note" class="view-mode">
      <div v-html="renderedContent" />
      <button @click="edit">Edit</button>
    </div>
    
    <!-- Empty state -->
    <div v-else class="empty-state">
      <p>No note selected</p>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { marked } from 'marked'

const isEditing = ref(false)
const content = ref('')

const renderedContent = computed(() => {
  return marked(content.value)
})
</script>
```

## List Rendering Pattern

### Pattern Description

Use v-for with :key for efficient list rendering.

### Implementation

```vue
<template>
  <div class="note-list">
    <div 
      v-for="note in notes" 
      :key="note.id"
      class="note-item"
      :class="{ 
        active: selectedNoteId === note.id,
        draft: note.status === 'draft'
      }"
      @click="selectNote(note.id)"
    >
      <h3>{{ note.title }}</h3>
      <p class="snippet">{{ note.content.substring(0, 100) }}...</p>
      <div class="meta">
        <span class="status">{{ note.status }}</span>
        <span class="tags">{{ note.tags.join(', ') }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'

const notes = ref([])
const selectedNoteId = ref(null)

const selectNote = (id) => {
  selectedNoteId.value = id
}
</script>
```

## Common Patterns Summary

| Pattern | Use Case | Example |
|---------|-----------|----------|
| Component Pattern | Reusable UI components | `NoteEditor.vue` |
| Props/Emits | Component communication | `defineProps`, `defineEmits` |
| Computed | Derived state | `filteredNotes` |
| Watchers | Side effects | `watch(selectedNoteId)` |
| Lifecycle Hooks | Initialization/cleanup | `onMounted`, `onUnmounted` |
| API Client | Centralized API calls | `notesApi.get()` |
| Debounce | Reduce API calls | `setTimeout` with clear |
| Loading States | User feedback | `v-if="loading"` |
| Error Handling | Graceful failures | `try/catch` with UI feedback |
| Conditional Rendering | Show/hide content | `v-if`, `v-else` |
| List Rendering | Dynamic lists | `v-for` with `:key` |

## Anti-Patterns to Avoid

### ❌ Don't Use Options API

```vue
<!-- Bad -->
<script>
export default {
  data() {
    return {
      notes: []
    }
  },
  methods: {
    loadNotes() {
      // ...
    }
  }
}
</script>

<!-- Good -->
<script setup>
import { ref, onMounted } from 'vue'

const notes = ref([])

const loadNotes = () => {
  // ...
}

onMounted(() => {
  loadNotes()
})
</script>
```

### ❌ Don't Mutate Props

```vue
<script setup>
const props = defineProps({
  note: Object
})

// Bad
props.note.title = 'New Title'

// Good
const localNote = ref({ ...props.note })
localNote.value.title = 'New Title'
</script>
```

### ❌ Don't Use Index as Key

```vue
<!-- Bad -->
<div v-for="(note, index) in notes" :key="index">

<!-- Good -->
<div v-for="note in notes" :key="note.id">
```

### ❌ Don't Mix Concerns

```vue
<!-- Bad - API calls in component -->
<script setup>
const notes = ref([])

const loadNotes = async () => {
  const response = await fetch('/api/notes')
  notes.value = await response.json()
}
</script>

<!-- Good - use API client -->
<script setup>
import { notes } from '@/api/client'

const notes = ref([])

const loadNotes = async () => {
  notes.value = await notes.list()
}
</script>
```

## Resources

- [Vue 3 Documentation](https://vuejs.org/)
- [Composition API Guide](https://vuejs.org/guide/extras/composition-api-faq.html)
- [Vue 3 Style Guide](https://vuejs.org/style-guide/)
- [Vite Documentation](https://vitejs.dev/)

---

**See Also:**
- [Coding Standards](./coding-standards.md)
- [Backend Patterns](./backend-patterns.md)
- [Testing Patterns](./testing-patterns.md)
