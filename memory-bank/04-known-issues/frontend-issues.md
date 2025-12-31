# Frontend Known Issues and Solutions

> **Purpose:** Document common frontend issues and their solutions
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document records common issues encountered in OrgAI frontend (Vue 3) and their solutions or workarounds.

## Installation and Setup Issues

### Issue: npm Install Fails

**Symptom:**
```
npm ERR! code ERESOLVE
npm ERR! ERESOLVE unable to resolve dependency tree
```

**Cause:**
Node.js version incompatible or dependency conflicts.

**Solution:**
```bash
# Check Node.js version
node --version  # Should be 18+ for Vue 3

# Update Node.js using nvm
nvm install 20
nvm use 20

# Clear npm cache
npm cache clean --force

# Delete node_modules and package-lock.json
rm -rf node_modules package-lock.json

# Reinstall
npm install
```

**Prevention:**
- Use Node.js 18+ for Vue 3
- Keep npm up to date
- Use lockfile for consistent installs

---

### Issue: Vite Dev Server Won't Start

**Symptom:**
```
Error: Port 5173 is already in use
```

**Cause:**
Another process using port 5173.

**Solution:**
```bash
# Find process using port 5173
netstat -ano | findstr :5173  # Windows
lsof -i :5173  # Linux/Mac

# Kill process
taskkill /PID <PID> /F  # Windows
kill -9 <PID>  # Linux/Mac

# Or use different port
npm run dev -- --port 5174
```

**Prevention:**
- Stop dev server before restarting
- Use different ports for multiple instances
- Check for zombie processes

---

### Issue: Module Not Found Errors

**Symptom:**
```
Failed to resolve import "@/components/NoteEditor"
```

**Cause:**
Path alias not configured in Vite.

**Solution:**
```javascript
// vite.config.js
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import path from 'path'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src')
    }
  },
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

**Prevention:**
- Configure path aliases in vite.config.js
- Use consistent import paths
- Restart dev server after config changes

---

## Runtime Issues

### Issue: API Calls Fail with CORS Error

**Symptom:**
```
Access to fetch at 'http://localhost:8080/api/notes' from origin 
'http://localhost:5173' has been blocked by CORS policy
```

**Cause:**
Backend CORS not configured or proxy not working.

**Solution:**
```javascript
// Verify proxy is configured in vite.config.js
server: {
  proxy: {
    '/api': {
      target: 'http://localhost:8080',
      changeOrigin: true,
    },
  },
}

// Use proxy URL (not direct backend URL)
import { notes } from '@/api/client'

// Correct - uses proxy
const notes = await notes.list()

// Incorrect - direct backend URL
const notes = await fetch('http://localhost:8080/api/notes')
```

**Prevention:**
- Use Vite proxy for API calls
- Verify proxy configuration
- Check backend CORS settings

---

### Issue: Component Not Rendering

**Symptom:**
Component mounts but doesn't show content.

**Cause:**
- Props not passed correctly
- Data not loaded
- Conditional rendering issue

**Solution:**
```vue
<script setup>
import { ref, onMounted } from 'vue'

const props = defineProps({
  noteId: {
    type: String,
    required: true
  }
})

const note = ref(null)

// Load data in onMounted
onMounted(async () => {
  note.value = await notesApi.get(props.noteId)
})
</script>

<template>
  <!-- Check if data loaded -->
  <div v-if="note">
    <h1>{{ note.title }}</h1>
    <p>{{ note.content }}</p>
  </div>
  
  <!-- Show loading state -->
  <div v-else>
    <p>Loading...</p>
  </div>
</template>
```

**Prevention:**
- Use v-if for conditional rendering
- Load data in lifecycle hooks
- Check props are passed correctly

---

### Issue: Reactive Updates Not Working

**Symptom:**
State changes don't update UI.

**Cause:**
- Not using ref/reactive
- Mutating props directly
- Not triggering reactivity

**Solution:**
```vue
<script setup>
import { ref, reactive, computed } from 'vue'

// Use ref for primitives
const count = ref(0)

// Use reactive for objects
const note = reactive({
  title: '',
  content: ''
})

// Correct: Update ref value
count.value = count.value + 1

// Incorrect: Direct mutation
count = count.value + 1

// Correct: Update reactive object
note.title = 'New Title'

// Incorrect: Reassign reactive object
note = { title: 'New Title', content: 'Content' }

// Use computed for derived state
const doubleCount = computed(() => count.value * 2)
</script>
```

**Prevention:**
- Use ref for primitives, reactive for objects
- Always use .value for ref
- Never reassign reactive objects

---

### Issue: Watcher Not Firing

**Symptom:**
Watcher doesn't trigger when data changes.

**Cause:**
- Watching wrong property
- Deep watch not enabled
- Immediate not set

**Solution:**
```vue
<script setup>
import { ref, watch } from 'vue'

const note = ref({ title: '', content: '' })

// Watch entire object (deep)
watch(note, (newNote, oldNote) => {
  console.log('Note changed:', newNote)
}, { deep: true })

// Watch specific property
watch(() => note.value.title, (newTitle, oldTitle) => {
  console.log('Title changed:', oldTitle, '->', newTitle)
})

// Watch with immediate execution
watch(note, (newNote) => {
  console.log('Note:', newNote)
}, { immediate: true })
</script>
```

**Prevention:**
- Use deep: true for objects
- Watch specific properties when possible
- Use immediate for initial execution

---

## Performance Issues

### Issue: Slow Page Load

**Symptom:**
Initial page load takes > 3 seconds.

**Cause:**
- Loading all notes at once
- Large bundle size
- No lazy loading

**Solution:**
```vue
<script setup>
import { ref, onMounted } from 'vue'

const notes = ref([])
const loading = ref(false)

// Load notes in batches
const loadNotes = async () => {
  loading.value = true
  try {
    // Load first 20 notes
    const firstBatch = await notesApi.list({ limit: 20 })
    notes.value = firstBatch
    
    // Load remaining in background
    setTimeout(async () => {
      const remaining = await notesApi.list({ offset: 20 })
      notes.value = [...notes.value, ...remaining]
    }, 100)
  } finally {
    loading.value = false
  }
}
</script>
```

**Prevention:**
- Implement pagination
- Lazy load components
- Optimize bundle size

---

### Issue: Search Debounce Not Working

**Symptom:**
Search API called on every keystroke.

**Cause:**
Debounce not implemented correctly.

**Solution:**
```vue
<script setup>
import { ref } from 'vue'

const searchQuery = ref('')
let debounceTimer = null

const handleSearch = (query) => {
  clearTimeout(debounceTimer)
  
  debounceTimer = setTimeout(async () => {
    if (query.trim()) {
      const results = await searchApi.query(query)
      searchResults.value = results
    } else {
      searchResults.value = []
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

**Prevention:**
- Always debounce search input
- Use appropriate delay (200-500ms)
- Clear previous timer before setting new one

---

## State Management Issues

### Issue: State Lost on Navigation

**Symptom:**
Form data lost when navigating between pages.

**Cause:**
No state persistence across route changes.

**Solution:**
```vue
<script setup>
import { ref, onBeforeUnmount } from 'vue'

const formData = ref({})

// Save state before unmount
onBeforeUnmount(() => {
  sessionStorage.setItem('formData', JSON.stringify(formData.value))
})

// Restore state on mount
onMounted(() => {
  const saved = sessionStorage.getItem('formData')
  if (saved) {
    formData.value = JSON.parse(saved)
  }
})
</script>
```

**Prevention:**
- Use state management library (Pinia)
- Persist important state to localStorage
- Use route parameters for state

---

### Issue: Multiple Components Share State

**Symptom:**
Changes in one component don't reflect in others.

**Cause:**
No shared state management.

**Solution:**
```javascript
// stores/notes.js (Pinia store)
import { defineStore } from 'pinia'

export const useNotesStore = defineStore('notes', {
  state: () => ({
    notes: [],
    selectedNoteId: null
  }),
  actions: {
    async loadNotes() {
      this.notes = await notesApi.list()
    },
    selectNote(id) {
      this.selectedNoteId = id
    }
  }
})

// In component
<script setup>
import { useNotesStore } from '@/stores/notes'

const notesStore = useNotesStore()

// Access state
const notes = computed(() => notesStore.notes)

// Call actions
const loadNotes = () => notesStore.loadNotes()
</script>
```

**Prevention:**
- Use Pinia for shared state
- Define stores for related data
- Use actions for state mutations

---

## Styling Issues

### Issue: Styles Not Applying

**Symptom:**
CSS classes defined but not applied.

**Cause:**
- scoped styles not working
- CSS specificity issues
- Style not imported

**Solution:**
```vue
<style scoped>
/* Scoped to component */
.note-editor {
  background: var(--bg-secondary);
}

/* Use ::v-deep for child components */
.note-editor ::v-deep(.child-component) {
  background: var(--bg-tertiary);
}
</style>

<!-- Or use CSS modules -->
<style module>
.noteEditor {
  background: var(--bg-secondary);
}
</style>

<template>
  <div :class="$style.noteEditor">
    <!-- content -->
  </div>
</template>
```

**Prevention:**
- Use scoped styles by default
- Check CSS specificity
- Import global styles in main.js

---

### Issue: Dark Mode Not Working

**Symptom:**
Dark mode styles not applied.

**Cause:**
CSS variables not defined or not applied.

**Solution:**
```css
/* style.css */
:root {
  --bg-primary: #0f0f10;
  --bg-secondary: #1a1a1d;
  --text-primary: #e8e8ed;
}

/* Dark mode */
@media (prefers-color-scheme: dark) {
  :root {
    --bg-primary: #0f0f10;
    --bg-secondary: #1a1a1d;
    --text-primary: #e8e8ed;
  }
}

/* Manual dark mode toggle */
body.dark-mode {
  --bg-primary: #0f0f10;
  --bg-secondary: #1a1a1d;
  --text-primary: #e8e8ed;
}
```

**Prevention:**
- Use CSS variables for theming
- Test in both light and dark modes
- Provide manual toggle option

---

## Testing Issues

### Issue: Component Tests Fail

**Symptom:**
Tests fail with "element not found" error.

**Cause:**
- Component not mounted correctly
- Async operations not awaited
- Wrong selector

**Solution:**
```javascript
import { mount, flushPromises } from '@vue/test-utils'

describe('NoteEditor', () => {
  it('renders note content', async () => {
    const wrapper = mount(NoteEditor, {
      props: {
        noteId: 'test',
        note: {
          id: 'test',
          title: 'Test Note',
          content: 'Test content'
        }
      }
    })
    
    // Wait for async operations
    await flushPromises()
    
    // Check element exists
    const title = wrapper.find('.note-title')
    expect(title.exists()).toBe(true)
    expect(title.text()).toBe('Test Note')
  })
})
```

**Prevention:**
- Use async/await for async operations
- Wait for DOM updates
- Use correct selectors

---

### Issue: Mock API Calls Not Working

**Symptom:**
Tests make real API calls instead of mocks.

**Cause:**
Mock not configured correctly.

**Solution:**
```javascript
import { vi } from 'vitest'
import { notes } from '@/api/client'

// Mock entire module
vi.mock('@/api/client', () => ({
  notes: {
    list: vi.fn(() => Promise.resolve([
      { id: '1', title: 'Note 1' },
      { id: '2', title: 'Note 2' }
    ])),
    get: vi.fn((id) => Promise.resolve({
      id,
      title: `Note ${id}`
    }))
  }
}))

// In test
import { notes } from '@/api/client'

it('loads notes', async () => {
  const result = await notes.list()
  expect(result).toHaveLength(2)
  expect(notes.list).toHaveBeenCalled()
})
```

**Prevention:**
- Mock at top of test file
- Verify mock is called
- Restore mocks after tests

---

## Browser Compatibility Issues

### Issue: Features Not Working in Safari

**Symptom:**
Works in Chrome but not Safari.

**Cause:**
Browser API not supported.

**Solution:**
```javascript
// Check for API support
const supportsClipboard = navigator.clipboard && navigator.clipboard.writeText

if (supportsClipboard) {
  navigator.clipboard.writeText(text)
} else {
  // Fallback
  const textarea = document.createElement('textarea')
  textarea.value = text
  document.body.appendChild(textarea)
  textarea.select()
  document.execCommand('copy')
  document.body.removeChild(textarea)
}
```

**Prevention:**
- Check browser compatibility
- Provide fallbacks for unsupported APIs
- Test in multiple browsers

---

## Troubleshooting Checklist

When encountering frontend issues:

1. **Check browser console**: Look for JavaScript errors
2. **Check network tab**: Verify API calls are successful
3. **Clear cache**: Hard refresh (Ctrl+Shift+R)
4. **Restart dev server**: Stop and restart Vite
5. **Check dependencies**: Verify npm install completed
6. **Review recent changes**: Check git diff for issues
7. **Test in different browsers**: Check for browser-specific issues

## Related Documentation

- [Backend Issues](./backend-issues.md)
- [Solutions](./solutions.md)
- [Configuration Reference](../05-configuration/)
- [Development Guide - Frontend](../../docs/development-guide-frontend.md)

---

**See Also:**
- [Architecture - Frontend](../../docs/architecture-frontend.md)
- [Component Inventory](../../docs/component-inventory-frontend.md)
- [Frontend Patterns](../03-development-patterns/frontend-patterns.md)
