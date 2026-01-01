<template>
  <div class="note-list">
    <div class="note-list-header">
      <h3>Notes</h3>
      <span class="note-count">{{ notes.length }}</span>
    </div>
    
    <div class="note-list-items">
      <div
        v-for="note in notes"
        :key="note.id"
        :class="['note-item', { selected: note.id === selected }]"
        @click="$emit('select', note.id)"
      >
        <div class="note-item-title">{{ note.title || 'Untitled' }}</div>
        <div class="note-item-meta">
          <span :class="['status', `status-${note.status}`]">
            {{ note.status }}
          </span>
          <span v-if="note.link_count !== undefined" class="link-count">
            {{ note.link_count }} links
          </span>
        </div>
        <div v-if="note.tags && note.tags.length > 0" class="note-item-tags">
          <span
            v-for="(tag, index) in note.tags.slice(0, 3)"
            :key="index"
            class="tag"
          >
            {{ tag }}
          </span>
          <span v-if="note.tags.length > 3" class="tag">
            +{{ note.tags.length - 3 }}
          </span>
        </div>
      </div>
      
      <div v-if="notes.length === 0" class="empty-list">
        <p class="text-muted">No notes yet</p>
      </div>
    </div>
  </div>
</template>

<script setup>
defineProps({
  notes: {
    type: Array,
    required: true
  },
  selected: {
    type: String,
    default: null
  }
})

defineEmits(['select'])
</script>

<style scoped>
.note-list {
  height: 100%;
  display: flex;
  flex-direction: column;
}

.note-list-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
}

.note-list-header h3 {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
}

.note-count {
  font-size: 0.75rem;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.note-list-items {
  flex: 1;
  overflow-y: auto;
}

.note-item {
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
  cursor: pointer;
  transition: all var(--transition-fast);
  border-left: 3px solid transparent;
}

.note-item:hover {
  background: var(--bg-hover);
}

.note-item.selected {
  background: var(--bg-tertiary);
  border-left-color: var(--accent-primary);
}

.note-item-title {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: var(--spacing-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.note-item-meta {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.link-count {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.note-item-tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
}

.empty-list {
  padding: var(--spacing-lg);
  text-align: center;
}

.empty-list p {
  margin: 0;
  font-size: 0.875rem;
}
</style>
