<template>
  <div class="unlinked-mentions">
    <div
      class="section-header"
      @click="isExpanded = !isExpanded"
    >
      <span class="section-icon">{{ isExpanded ? '▼' : '▶' }}</span>
      <h3 class="section-title">
        Unlinked Mentions
      </h3>
      <span
        v-if="mentions.length > 0"
        class="mention-count"
      >{{ mentions.length }}</span>
    </div>
    
    <div
      v-if="isExpanded"
      class="mentions-content"
    >
      <div
        v-if="loading"
        class="loading-state"
      >
        <span class="loading-spinner">⏳</span>
        <span>Finding mentions...</span>
      </div>
      
      <div
        v-else-if="mentions.length === 0"
        class="empty-state"
      >
        <span class="empty-icon">✓</span>
        <span>No unlinked mentions found</span>
      </div>
      
      <div
        v-else
        class="mentions-list"
      >
        <div 
          v-for="mention in mentions" 
          :key="mention.note_id"
          class="mention-item"
        >
          <div class="mention-header">
            <span
              class="mention-title"
              @click="$emit('navigate', mention.note_id)"
            >
              {{ mention.title }}
            </span>
            <button 
              class="link-btn"
              title="Convert to wikilink"
              @click="handleLinkIt(mention)"
            >
              🔗 Link it
            </button>
          </div>
          <div
            v-if="mention.context"
            class="mention-context"
          >
            "...{{ mention.context }}..."
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onMounted } from 'vue'
import { graph } from '@/api/client'

const props = defineProps({
  noteId: {
    type: String,
    required: true
  },
  noteTitle: {
    type: String,
    default: ''
  }
})

const emit = defineEmits(['navigate', 'link-created'])

const isExpanded = ref(false)
const loading = ref(false)
const mentions = ref([])

async function fetchUnlinkedMentions() {
  if (!props.noteId) return
  
  loading.value = true
  try {
    const response = await graph.unlinkedMentions(props.noteId)
    mentions.value = response || []
  } catch (error) {
    console.error('Failed to fetch unlinked mentions:', error)
    mentions.value = []
  } finally {
    loading.value = false
  }
}

function handleLinkIt(mention) {
  // Emit event for parent to handle the linking
  emit('link-created', {
    sourceNoteId: mention.note_id,
    targetTitle: props.noteTitle,
    context: mention.context
  })
}

// Fetch when expanded
watch(isExpanded, (expanded) => {
  if (expanded && mentions.value.length === 0) {
    fetchUnlinkedMentions()
  }
})

// Refetch when note changes
watch(() => props.noteId, () => {
  mentions.value = []
  if (isExpanded.value) {
    fetchUnlinkedMentions()
  }
})

onMounted(() => {
  if (isExpanded.value) {
    fetchUnlinkedMentions()
  }
})
</script>

<style scoped>
.unlinked-mentions {
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

.mention-count {
  background: var(--accent-primary);
  color: white;
  font-size: 0.75rem;
  padding: 2px 6px;
  border-radius: var(--radius-full);
  min-width: 20px;
  text-align: center;
}

.mentions-content {
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

.empty-icon {
  color: var(--success-color, #22c55e);
}

.mentions-list {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
}

.mention-item {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  padding: var(--spacing-sm);
}

.mention-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--spacing-sm);
}

.mention-title {
  font-weight: 500;
  color: var(--accent-primary);
  cursor: pointer;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mention-title:hover {
  text-decoration: underline;
}

.link-btn {
  background: var(--accent-primary);
  color: white;
  border: none;
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--transition-fast);
}

.link-btn:hover {
  background: var(--accent-secondary, var(--accent-primary));
  transform: scale(1.05);
}

.mention-context {
  margin-top: var(--spacing-xs);
  font-size: 0.75rem;
  color: var(--text-muted);
  font-style: italic;
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
}
</style>
