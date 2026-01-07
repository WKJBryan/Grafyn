# Seedream Frontend Development Guide

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
    ├── main.js           # Vue app bootstrap with Pinia & Router
    ├── App.vue           # Root component (router-view)
    ├── style.css         # Design system
    ├── router/
    │   └── index.js      # Route definitions
    ├── stores/
    │   ├── auth.js       # Authentication state
    │   └── notes.js      # Notes state
    ├── views/
    │   ├── HomeView.vue
    │   ├── LoginView.vue
    │   ├── OAuthCallbackView.vue
    │   └── NotFoundView.vue
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
| `npm run lint` | Run ESLint |
| `npm run format` | Format with Prettier |

---

## Dependencies

```json
{
    "dependencies": {
        "vue": "^3.4.0",
        "pinia": "^2.1.0",
        "vue-router": "^4.2.0",
        "marked": "^11.0.0"
    },
    "devDependencies": {
        "@vitejs/plugin-vue": "^4.5.0",
        "vite": "^5.0.0",
        "eslint": "^8.0.0",
        "prettier": "^3.0.0"
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

## State Management with Pinia

### Using Stores

```javascript
import { useNotesStore } from '@/stores/notes'
import { useAuthStore } from '@/stores/auth'

// In setup()
const notesStore = useNotesStore()
const authStore = useAuthStore()

// Access state
console.log(notesStore.notes)
console.log(authStore.isAuthenticated)

// Call actions
await notesStore.loadNotes()
await authStore.login()
```

### Creating a New Store

```javascript
// stores/newStore.js
import { defineStore } from 'pinia'

export const useNewStore = defineStore('new', {
  state: () => ({
    items: [],
    loading: false,
  }),
  
  getters: {
    itemCount: (state) => state.items.length,
  },
  
  actions: {
    async loadItems() {
      this.loading = true
      try {
        this.items = await api.getItems()
      } finally {
        this.loading = false
      }
    },
  },
})
```

---

## Vue Router

### Route Configuration

```javascript
// router/index.js
import { createRouter, createWebHistory } from 'vue-router'
import HomeView from '@/views/HomeView.vue'
import LoginView from '@/views/LoginView.vue'

const routes = [
  { path: '/', name: 'home', component: HomeView },
  { path: '/login', name: 'login', component: LoginView },
  { path: '/oauth/callback', component: OAuthCallbackView },
  { path: '/:pathMatch(.*)*', component: NotFoundView },
]

export default createRouter({
  history: createWebHistory(),
  routes,
})
```

### Navigation Guards

```javascript
router.beforeEach((to, from, next) => {
  const auth = useAuthStore()
  if (to.meta.requiresAuth && !auth.isAuthenticated) {
    next('/login')
  } else {
    next()
  }
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

<script setup>
defineProps({
  title: {
    type: String,
    required: true,
  },
})
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
<script setup>
import NewComponent from '@/components/NewComponent.vue'
</script>

<template>
  <NewComponent title="Hello">Content here</NewComponent>
</template>
```

---

## API Client Usage

### Import Functions
```javascript
import { notes, search, graph, auth } from '@/api/client'
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

// Auth
const authUrl = await auth.getGithubUrl()
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
```

### Using Stores in Components
```javascript
import { storeToRefs } from 'pinia'
import { useNotesStore } from '@/stores/notes'

const store = useNotesStore()

// Destructure reactive state
const { notes, selectedNote, loading } = storeToRefs(store)

// Call actions directly
await store.loadNotes()
```

---

## Common Tasks

### Add a New View

1. Create view in `src/views/NewView.vue`
2. Add route in `src/router/index.js`:
```javascript
import NewView from '@/views/NewView.vue'

const routes = [
  // ...
  { path: '/new', name: 'new', component: NewView },
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
- Pinia store state tracking
- Router debugging
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

### Store state not updating
**Cause:** Missing `storeToRefs` for reactive access  
**Solution:** Use `storeToRefs(store)` for reactive destructuring

### Route not found
**Cause:** Missing route definition  
**Solution:** Add route to `router/index.js`

### Hot reload not working
**Solution:** Restart Vite dev server
