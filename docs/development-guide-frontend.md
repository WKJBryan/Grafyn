# OrgAI Frontend Development Guide

> **Part:** Frontend | **Framework:** Vue 3 | **Build Tool:** Vite

## Prerequisites

| Requirement | Version | Check Command |
|-------------|---------|---------------|
| Node.js | 18+ | `node --version` |
| npm | 9+ | `npm --version` |

---

## Quick Start

### 1. Install Dependencies

```bash
cd frontend
npm install
```

### 2. Start Development Server

```bash
npm run dev
```

### 3. Access the Application

- **Frontend UI:** http://localhost:5173
- **Note:** Ensure backend is running on port 8080

---

## Project Structure

```
frontend/
├── index.html            # HTML entry point
├── package.json          # Dependencies
├── vite.config.js        # Vite config (proxy)
└── src/
    ├── main.js           # Vue app bootstrap
    ├── App.vue           # Root component
    ├── style.css         # Design system
    ├── api/
    │   └── client.js     # Backend API client
    └── components/
        ├── SearchBar.vue
        ├── NoteList.vue
        ├── NoteEditor.vue
        ├── BacklinksPanel.vue
        └── GraphView.vue
```

---

## Scripts

| Command | Description |
|---------|-------------|
| `npm run dev` | Start Vite dev server with HMR |
| `npm run build` | Build for production |
| `npm run preview` | Preview production build |

---

## Dependencies

```json
{
    "dependencies": {
        "vue": "^3.4.0",
        "vue-router": "^4.2.0",
        "axios": "^1.6.0",
        "marked": "^11.0.0"
    },
    "devDependencies": {
        "@vitejs/plugin-vue": "^4.5.0",
        "vite": "^5.0.0"
    }
}
```

---

## Proxy Configuration

The Vite dev server proxies API requests to the backend:

```javascript
// vite.config.js
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

## Component Development

### Creating a New Component

1. **Create the file** (`src/components/NewComponent.vue`)
```vue
<template>
  <div class="new-component">
    <h3>{{ title }}</h3>
    <slot></slot>
  </div>
</template>

<script>
export default {
  name: 'NewComponent',
  props: {
    title: {
      type: String,
      required: true,
    },
  },
}
</script>

<style scoped>
.new-component {
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
  padding: var(--spacing-md);
}
</style>
```

2. **Import and use**
```vue
<script>
import NewComponent from './components/NewComponent.vue'

export default {
  components: { NewComponent },
}
</script>

<template>
  <NewComponent title="Hello">Content here</NewComponent>
</template>
```

---

## API Client Usage

### Import Functions
```javascript
import { notes, search, graph } from './api/client.js'
```

### Examples
```javascript
// List all notes
const allNotes = await notes.list()

// Get specific note
const note = await notes.get('Welcome')

// Search semantically
const results = await search.query('knowledge graph', { limit: 5 })

// Get backlinks
const backlinks = await graph.backlinks('Welcome')

// Create note
const newNote = await notes.create({
    title: 'New Note',
    content: '# New Note\n\nContent...',
    tags: ['example'],
    status: 'draft',
})

// Update note
await notes.update('New_Note', {
    content: 'Updated content...',
})

// Delete note
await notes.delete('New_Note')
```

---

## Design System

### CSS Variables

All styling uses CSS custom properties from `src/style.css`:

| Category | Examples |
|----------|----------|
| **Backgrounds** | `--bg-primary`, `--bg-secondary`, `--bg-tertiary` |
| **Text** | `--text-primary`, `--text-secondary`, `--text-muted` |
| **Accents** | `--accent-primary`, `--accent-success`, `--accent-warning` |
| **Spacing** | `--spacing-xs`, `--spacing-sm`, `--spacing-md`, `--spacing-lg` |
| **Radius** | `--radius-sm`, `--radius-md`, `--radius-lg` |
| **Transitions** | `--transition-fast`, `--transition-normal` |

### Component Classes

| Class | Usage |
|-------|-------|
| `.btn-primary` | Primary action buttons |
| `.btn-secondary` | Secondary actions |
| `.btn-ghost` | Subtle/icon buttons |
| `.tag` | Tag pills |
| `.status` | Status badges |
| `.status-draft` | Draft status (yellow) |
| `.status-canonical` | Canonical status (green) |
| `.status-evidence` | Evidence status (purple) |
| `.card` | Card containers |
| `.card-hover` | Cards with hover effect |
| `.wikilink` | Rendered wikilinks |

---

## Composition API Patterns

### Reactive State
```javascript
import { ref, computed, onMounted, watch } from 'vue'

setup() {
    // Reactive references
    const items = ref([])
    const loading = ref(false)
    
    // Computed properties
    const itemCount = computed(() => items.value.length)
    
    // Lifecycle
    onMounted(async () => {
        await loadItems()
    })
    
    // Watchers
    watch(() => props.id, async (newId) => {
        await loadItem(newId)
    })
    
    return { items, loading, itemCount }
}
```

### Emitting Events
```javascript
setup(props, { emit }) {
    function handleClick(id) {
        emit('select', id)
    }
    return { handleClick }
}
```

---

## Common Tasks

### Add a New Route (when routing is enabled)
```javascript
// router.js
const routes = [
    { path: '/', component: Home },
    { path: '/note/:id', component: NoteView },
]
```

### Add a New API Function
```javascript
// api/client.js
export const newApi = {
    getData: () => request('/new-endpoint'),
    postData: (data) => request('/new-endpoint', { method: 'POST', body: data }),
}
```

---

## Debugging

### Vue DevTools
Install the [Vue DevTools](https://devtools.vuejs.org/) browser extension for:
- Component tree inspection
- Reactive state tracking
- Event monitoring

### Console Logging
```javascript
console.log('Data:', JSON.parse(JSON.stringify(reactiveData.value)))
```

### Network Issues
Check browser DevTools → Network tab for API calls.

---

## Building for Production

```bash
# Build
npm run build

# Output in dist/
ls dist/

# Preview build locally
npm run preview
```

### Deploy
Serve the `dist/` folder with any static file server.
Configure the server to redirect all routes to `index.html` for SPA routing.

---

## Common Issues

### "Failed to fetch" errors
**Cause:** Backend not running  
**Solution:** Start backend on port 8080

### Styles not applying
**Cause:** Missing CSS variable  
**Solution:** Check `style.css` for variable definition

### Component not reactive
**Cause:** Missing `.value` on refs  
**Solution:** Access refs with `.value` in script, not in template

### Hot reload not working
**Solution:** Restart Vite dev server
