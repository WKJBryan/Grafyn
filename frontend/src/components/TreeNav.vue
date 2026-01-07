<template>
  <div class="tree-nav">
    <div class="nav-section">
      <div class="section-header">
        <span class="icon">📂</span>
        <span class="label">Notes</span>
      </div>
      <div class="section-content">
        <div 
          v-for="note in notes" 
          :key="note.id"
          class="nav-item"
          :class="{ active: selectedId === note.id }"
          @click="$emit('select', note.id)"
        >
          <span class="file-icon">📄</span>
          <span class="file-name">{{ note.title || 'Untitled' }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
defineProps({
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

.file-icon {
  opacity: 0.7;
  font-size: 1rem;
}

.file-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
