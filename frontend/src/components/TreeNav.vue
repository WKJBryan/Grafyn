<template>
  <div class="tree-nav">
    <!-- Container Notes (top-level) -->
    <div class="nav-section" v-if="containerNotes.length > 0">
      <div class="section-header">
        <span class="icon">📂</span>
        <span class="label">Notes</span>
      </div>
      <div class="section-content">
        <div v-for="container in containerNotes" :key="container.id">
          <div 
            class="nav-item"
            :class="{ active: selectedId === container.id }"
            @click="$emit('select', container.id)"
          >
            <span class="file-icon">📄</span>
            <span class="file-name">{{ container.title || 'Untitled' }}</span>
          </div>
          <!-- Atomic notes under this container -->
          <div v-if="getAtomicsFor(container.title).length > 0" class="nested-items">
            <div 
              v-for="atomic in getAtomicsFor(container.title)" 
              :key="atomic.id"
              class="nav-item nested"
              :class="{ active: selectedId === atomic.id }"
              @click.stop="$emit('select', atomic.id)"
            >
              <span class="file-icon">⚛️</span>
              <span class="file-name">{{ formatAtomicTitle(atomic.title) }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Standalone Atomic Notes (orphaned) -->
    <div class="nav-section" v-if="orphanAtomics.length > 0">
      <div class="section-header">
        <span class="icon">⚛️</span>
        <span class="label">Atomic Notes</span>
      </div>
      <div class="section-content">
        <div 
          v-for="note in orphanAtomics" 
          :key="note.id"
          class="nav-item"
          :class="{ active: selectedId === note.id }"
          @click="$emit('select', note.id)"
        >
          <span class="file-icon">📝</span>
          <span class="file-name">{{ formatAtomicTitle(note.title) }}</span>
        </div>
      </div>
    </div>

    <!-- Hub Notes -->
    <div class="nav-section" v-if="hubNotes.length > 0">
      <div class="section-header">
        <span class="icon">🔗</span>
        <span class="label">Hubs</span>
      </div>
      <div class="section-content">
        <div 
          v-for="note in hubNotes" 
          :key="note.id"
          class="nav-item"
          :class="{ active: selectedId === note.id }"
          @click="$emit('select', note.id)"
        >
          <span class="file-icon">🏷️</span>
          <span class="file-name">{{ note.title.replace('Hub: ', '') }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  notes: {
    type: Array,
    default: () => []
  },
  selectedId: {
    type: String,
    default: null
  }
})

defineEmits(['select'])

// Separate notes into categories using note_type field (with title fallback)
const atomicNotes = computed(() => 
  props.notes.filter(n => 
    n.note_type === 'atomic' || n.title?.startsWith('Atomic:')
  )
)

const hubNotes = computed(() => 
  props.notes.filter(n => 
    n.note_type === 'hub' || n.title?.startsWith('Hub:')
  )
)

const containerNotes = computed(() => 
  props.notes.filter(n => {
    // Explicit container type
    if (n.note_type === 'container') return true
    // General notes (not atomic/hub)
    if (n.note_type === 'general' || !n.note_type) {
      return !n.title?.startsWith('Atomic:') && !n.title?.startsWith('Hub:')
    }
    return false
  })
)

// Get atomics that reference a container by title
function getAtomicsFor(containerTitle) {
  const container = containerNotes.value.find(c => c.title === containerTitle)
  if (!container) return []

  return atomicNotes.value.filter(atomic => {
    // Check if atomic has an outgoing link to this container (by ID)
    const hasLinkToContainer = atomic.outgoing_links?.includes(container.id)
    
    if (hasLinkToContainer) {
      return true
    }
    
    // Fallback: use keyword matching if outgoing_links is not available
    if (!atomic.outgoing_links || atomic.outgoing_links.length === 0) {
      const keywords = containerTitle.toLowerCase().split(/\s+/).filter(w => w.length > 3)
      const atomicLower = atomic.title.toLowerCase()
      return keywords.some(kw => atomicLower.includes(kw))
    }
    
    return false
  })
}

// Orphan atomics = atomics not matched to any container
const orphanAtomics = computed(() => {
  const matched = new Set()
  containerNotes.value.forEach(container => {
    getAtomicsFor(container.title).forEach(a => matched.add(a.id))
  })
  return atomicNotes.value.filter(a => !matched.has(a.id))
})

function formatAtomicTitle(title) {
  return title?.replace('Atomic: ', '') || 'Untitled'
}
</script>

<style scoped>
.tree-nav {
  padding: var(--spacing-sm);
  color: var(--text-secondary);
  font-size: 0.9rem;
}

.nav-section {
  margin-bottom: var(--spacing-md);
}

.section-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-xs) var(--spacing-sm);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  font-size: 0.75rem;
  letter-spacing: 0.05em;
}

.section-content {
  margin-left: var(--spacing-sm);
}

.nav-item {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: 4px var(--spacing-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
  margin-bottom: 1px;
}

.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.nav-item.active {
  background: var(--bg-tertiary);
  color: var(--accent-primary);
}

.nav-item.nested {
  margin-left: var(--spacing-md);
  font-size: 0.85rem;
  opacity: 0.9;
}

.nested-items {
  border-left: 1px solid var(--bg-tertiary);
  margin-left: var(--spacing-md);
  padding-left: var(--spacing-xs);
}

.file-icon {
  opacity: 0.7;
  font-size: 0.9rem;
}

.file-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>

