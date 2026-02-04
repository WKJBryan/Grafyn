<template>
  <div class="tree-nav">
    <!-- Canvas Exports Section -->
    <div
      v-if="canvasNotes.length > 0"
      class="nav-section"
    >
      <div 
        class="section-header clickable" 
        @click="toggleSection('canvas')"
      >
        <span class="chevron">{{ expandedSections.canvas ? '▼' : '▶' }}</span>
        <span class="icon">📰</span>
        <span class="label">Canvas Exports</span>
        <span class="count">({{ canvasNotes.length }})</span>
      </div>
      <div
        v-show="expandedSections.canvas"
        class="section-content"
      >
        <div
          v-for="note in canvasNotes"
          :key="note.id"
        >
          <div 
            class="nav-item"
            :class="{ active: selectedId === note.id }"
            @click="$emit('select', note.id)"
          >
            <span class="file-icon">📰</span>
            <span class="file-name">{{ formatTitle(note.title) }}</span>
          </div>
          <!-- Nested atomics that backlink to this canvas -->
          <div
            v-if="getAtomicsFor(note).length > 0"
            class="nested-items"
          >
            <div 
              v-for="atomic in getAtomicsFor(note)" 
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

    <!-- Topic Folders (grouped by first tag) -->
    <div 
      v-for="topic in sortedTopics" 
      :key="topic.name" 
      class="nav-section"
    >
      <div 
        class="section-header clickable" 
        @click="toggleSection(topic.name)"
      >
        <span class="chevron">{{ expandedSections[topic.name] ? '▼' : '▶' }}</span>
        <span class="icon">📂</span>
        <span class="label">{{ formatTopicName(topic.name) }}</span>
        <span class="count">({{ topic.notes.length }})</span>
      </div>
      <div
        v-show="expandedSections[topic.name]"
        class="section-content"
      >
        <div
          v-for="note in topic.notes"
          :key="note.id"
        >
          <div 
            class="nav-item"
            :class="{ active: selectedId === note.id }"
            @click="$emit('select', note.id)"
          >
            <span class="file-icon">{{ getNoteIcon(note) }}</span>
            <span class="file-name">{{ formatTitle(note.title) }}</span>
          </div>
          <!-- Nested atomics under containers -->
          <div
            v-if="getAtomicsFor(note).length > 0"
            class="nested-items"
          >
            <div 
              v-for="atomic in getAtomicsFor(note)" 
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

    <!-- Uncategorized Notes -->
    <div
      v-if="uncategorizedNotes.length > 0"
      class="nav-section"
    >
      <div 
        class="section-header clickable" 
        @click="toggleSection('uncategorized')"
      >
        <span class="chevron">{{ expandedSections.uncategorized ? '▼' : '▶' }}</span>
        <span class="icon">📝</span>
        <span class="label">Uncategorized</span>
        <span class="count">({{ uncategorizedNotes.length }})</span>
      </div>
      <div
        v-show="expandedSections.uncategorized"
        class="section-content"
      >
        <div 
          v-for="note in uncategorizedNotes" 
          :key="note.id"
          class="nav-item"
          :class="{ active: selectedId === note.id }"
          @click="$emit('select', note.id)"
        >
          <span class="file-icon">{{ getNoteIcon(note) }}</span>
          <span class="file-name">{{ formatTitle(note.title) }}</span>
        </div>
      </div>
    </div>

    <!-- Orphan Atomic Notes (not nested under containers) -->
    <div
      v-if="orphanAtomics.length > 0"
      class="nav-section"
    >
      <div 
        class="section-header clickable" 
        @click="toggleSection('atomics')"
      >
        <span class="chevron">{{ expandedSections.atomics ? '▼' : '▶' }}</span>
        <span class="icon">⚛️</span>
        <span class="label">Atomic Notes</span>
        <span class="count">({{ orphanAtomics.length }})</span>
      </div>
      <div
        v-show="expandedSections.atomics"
        class="section-content"
      >
        <div
          v-for="note in orphanAtomics"
          :key="note.id"
        >
          <div 
            class="nav-item"
            :class="{ active: selectedId === note.id }"
            @click="$emit('select', note.id)"
          >
            <span class="file-icon">⚛️</span>
            <span class="file-name">{{ formatAtomicTitle(note.title) }}</span>
          </div>
          <!-- Nested notes that backlink to this atomic -->
          <div
            v-if="getAtomicsFor(note).length > 0"
            class="nested-items"
          >
            <div 
              v-for="child in getAtomicsFor(note)" 
              :key="child.id"
              class="nav-item nested"
              :class="{ active: selectedId === child.id }"
              @click.stop="$emit('select', child.id)"
            >
              <span class="file-icon">{{ getNoteIcon(child) }}</span>
              <span class="file-name">{{ formatAtomicTitle(child.title) }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, reactive } from 'vue'

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

// Track which sections are expanded
const expandedSections = reactive({
  canvas: true,
  uncategorized: true,
  atomics: false  // Collapsed by default since there may be many
})

function toggleSection(sectionName) {
  if (expandedSections[sectionName] === undefined) {
    expandedSections[sectionName] = false
  } else {
    expandedSections[sectionName] = !expandedSections[sectionName]
  }
}

// Canvas exports (notes with source: canvas)
const canvasNotes = computed(() => 
  props.notes.filter(n => n.source === 'canvas' || n.title?.startsWith('Canvas:'))
)

// Atomic notes for nesting
const atomicNotes = computed(() => 
  props.notes.filter(n => 
    n.note_type === 'atomic' || n.title?.startsWith('Atomic:')
  )
)

// Get non-canvas, non-atomic notes grouped by first tag (topic)
const topicGroups = computed(() => {
  const groups = {}
  
  props.notes.forEach(note => {
    // Skip canvas exports and atomics (atomics are nested under containers)
    if (note.source === 'canvas' || note.title?.startsWith('Canvas:')) return
    if (note.note_type === 'atomic' || note.title?.startsWith('Atomic:')) return
    
    // Get first tag as topic
    const firstTag = note.tags?.[0]
    if (firstTag) {
      // Get root topic (before any slash for nested tags)
      const rootTopic = firstTag.split('/')[0]
      if (!groups[rootTopic]) {
        groups[rootTopic] = []
        // Initialize as expanded
        if (expandedSections[rootTopic] === undefined) {
          expandedSections[rootTopic] = true
        }
      }
      groups[rootTopic].push(note)
    }
  })
  
  return groups
})

// Sort topics alphabetically
const sortedTopics = computed(() => {
  return Object.keys(topicGroups.value)
    .sort((a, b) => a.localeCompare(b))
    .map(name => ({
      name,
      notes: topicGroups.value[name]
    }))
})

// Notes without any tags
const uncategorizedNotes = computed(() => 
  props.notes.filter(note => {
    // Skip canvas exports
    if (note.source === 'canvas' || note.title?.startsWith('Canvas:')) return false
    // Skip atomics (they're nested)
    if (note.note_type === 'atomic' || note.title?.startsWith('Atomic:')) return false
    // Include if no tags
    return !note.tags || note.tags.length === 0
  })
)

// Get notes linked to a parent note (by outgoing links, backlinks, or container_of)
// Works for any note type including atomic notes
function getAtomicsFor(containerNote) {
  if (!containerNote) return []
  
  const containerTitle = containerNote.title?.toLowerCase() || ''
  const containerTitleClean = containerTitle.replace('canvas:', '').replace('atomic:', '').trim()
  
  return atomicNotes.value.filter(atomic => {
    // IMPORTANT: Never match a note to itself
    if (atomic.id === containerNote.id) return false
    
    // 1. Check if atomic has an outgoing link to this container (wikilinks are titles)
    if (atomic.outgoing_links?.length) {
      const hasLinkToContainer = atomic.outgoing_links.some(link => {
        const linkLower = link.toLowerCase()
        // Check exact match or if the link contains the container title
        return linkLower === containerTitle || 
               linkLower === containerTitleClean ||
               linkLower.includes(containerTitleClean)
      })
      if (hasLinkToContainer) return true
    }
    
    // 2. Check if container lists this atomic in container_of
    if (containerNote.container_of?.includes(atomic.id)) {
      return true
    }
    
    // 3. Check if atomic's outgoing_links match container ID (fallback)
    if (atomic.outgoing_links?.includes(containerNote.id)) {
      return true
    }
    
    // 4. For Canvas exports only, check if atomic was likely distilled from it
    const atomicTitle = atomic.title?.toLowerCase() || ''
    if (containerNote.source === 'canvas' || containerNote.title?.startsWith('Canvas:')) {
      // Check if atomic mentions key parts of the canvas title
      const titleParts = containerTitleClean.split(/[\s\/]+/).filter(w => w.length > 2)
      if (titleParts.length > 0 && titleParts.some(part => atomicTitle.includes(part))) {
        return true
      }
    }
    
    // NOTE: Removed keyword fallback matching to avoid false positives and potential circular refs
    // Children should explicitly link to their parent via wikilinks or container_of
    
    return false
  })
}

// Orphan atomics = atomics not matched to any container (including canvas notes and other atomics)
const orphanAtomics = computed(() => {
  const matched = new Set()
  
  // Check atomics nested under canvas notes
  canvasNotes.value.forEach(note => {
    getAtomicsFor(note).forEach(a => matched.add(a.id))
  })
  
  // Check atomics nested under topic notes
  sortedTopics.value.forEach(topic => {
    topic.notes.forEach(note => {
      getAtomicsFor(note).forEach(a => matched.add(a.id))
    })
  })
  
  // Check atomics nested under uncategorized notes
  uncategorizedNotes.value.forEach(note => {
    getAtomicsFor(note).forEach(a => matched.add(a.id))
  })
  
  // First pass: get initial orphans (atomics not under other containers)
  const initialOrphans = atomicNotes.value.filter(a => !matched.has(a.id))
  
  // Check atomics nested under other atomics (to avoid duplicates)
  initialOrphans.forEach(note => {
    getAtomicsFor(note).forEach(a => matched.add(a.id))
  })
  
  return atomicNotes.value.filter(a => !matched.has(a.id))
})

// Get icon based on note type
function getNoteIcon(note) {
  if (note.note_type === 'hub' || note.title?.startsWith('Hub:')) return '🗺️'
  if (note.note_type === 'container') return '📂'
  if (note.note_type === 'atomic') return '⚛️'
  return '📄'
}

// Format topic name (capitalize first letter)
function formatTopicName(name) {
  if (!name) return 'Unknown'
  return name.charAt(0).toUpperCase() + name.slice(1)
}

// Format title (remove prefixes)
function formatTitle(title) {
  if (!title) return 'Untitled'
  return title
    .replace('Canvas: ', '')
    .replace('Hub: ', '')
}

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
  margin-bottom: var(--spacing-sm);
}

.section-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-xs) var(--spacing-sm);
  font-weight: 600;
  color: var(--text-muted);
  font-size: 0.8rem;
  letter-spacing: 0.02em;
  border-radius: var(--radius-sm);
  transition: all var(--transition-fast);
}

.section-header.clickable {
  cursor: pointer;
}

.section-header.clickable:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.chevron {
  font-size: 0.65rem;
  opacity: 0.6;
  width: 12px;
  text-align: center;
}

.count {
  margin-left: auto;
  font-size: 0.7rem;
  opacity: 0.5;
  font-weight: 400;
}

.section-content {
  margin-left: var(--spacing-md);
  border-left: 1px solid var(--bg-tertiary);
  padding-left: var(--spacing-xs);
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
  flex-shrink: 0;
}

.file-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
