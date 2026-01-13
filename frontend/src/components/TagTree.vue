<template>
  <div class="tag-tree">
    <div class="section-header" @click="isExpanded = !isExpanded">
      <span class="section-icon">{{ isExpanded ? '▼' : '▶' }}</span>
      <h3 class="section-title">Tags</h3>
      <span v-if="totalTags > 0" class="tag-count">{{ totalTags }}</span>
    </div>
    
    <div v-if="isExpanded" class="tree-content">
      <div v-if="loading" class="loading-state">
        <span class="loading-spinner">⏳</span>
        <span>Loading tags...</span>
      </div>
      
      <div v-else-if="Object.keys(tagTree).length === 0" class="empty-state">
        <span class="empty-icon">🏷️</span>
        <span>No tags found</span>
      </div>
      
      <div v-else class="tree-list">
        <TagTreeNode
          v-for="(children, tag) in tagTree"
          :key="tag"
          :name="tag"
          :children="children"
          :counts="tagCounts"
          :selected-tags="selectedTags"
          @toggle="handleTagToggle"
        />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, defineAsyncComponent } from 'vue'

// Self-referencing component for recursive tree rendering
const TagTreeNode = defineAsyncComponent(() => import('./TagTreeNode.vue'))

const props = defineProps({
  tags: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['filter'])

const isExpanded = ref(true)
const loading = ref(false)
const selectedTags = ref(new Set())

// Build hierarchical tree from flat tag list
const tagTree = computed(() => {
  const tree = {}
  
  for (const tag of props.tags) {
    const parts = tag.split('/')
    let current = tree
    
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      if (!current[part]) {
        current[part] = {}
      }
      current = current[part]
    }
  }
  
  return tree
})

// Count occurrences of each tag
const tagCounts = computed(() => {
  const counts = {}
  for (const tag of props.tags) {
    counts[tag] = (counts[tag] || 0) + 1
    
    // Also count parent tags
    const parts = tag.split('/')
    let path = ''
    for (const part of parts) {
      path = path ? `${path}/${part}` : part
      if (!counts[path]) {
        counts[path] = 0
      }
    }
  }
  return counts
})

const totalTags = computed(() => {
  return new Set(props.tags).size
})

function handleTagToggle(tagPath) {
  if (selectedTags.value.has(tagPath)) {
    selectedTags.value.delete(tagPath)
  } else {
    selectedTags.value.add(tagPath)
  }
  
  // Emit filter event with selected tags
  emit('filter', Array.from(selectedTags.value))
}
</script>

<style scoped>
.tag-tree {
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
  overflow: hidden;
}

.section-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  cursor: pointer;
  user-select: none;
  transition: background var(--transition-fast);
}

.section-header:hover {
  background: var(--bg-hover);
}

.section-icon {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.section-title {
  flex: 1;
  margin: 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-secondary);
}

.tag-count {
  background: var(--accent-primary);
  color: white;
  font-size: 0.75rem;
  padding: 2px 6px;
  border-radius: var(--radius-full);
  min-width: 20px;
  text-align: center;
}

.tree-content {
  padding: var(--spacing-sm) var(--spacing-md) var(--spacing-md);
  border-top: 1px solid var(--bg-tertiary);
}

.loading-state,
.empty-state {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: var(--spacing-sm) 0;
}

.loading-spinner {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.tree-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
