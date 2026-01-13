<template>
  <div class="backlinks-panel">
    <div class="panel-header">
      <h3>Backlinks</h3>
      <span v-if="backlinks.length > 0" class="backlink-count">
        {{ backlinks.length }}
      </span>
    </div>

    <div v-if="loading" class="loading-state">
      <p class="text-muted">Loading...</p>
    </div>

    <div v-else-if="backlinks.length === 0" class="empty-state">
      <p class="text-muted">No backlinks yet</p>
    </div>

    <div v-else class="backlinks-list">
      <div
        v-for="backlink in backlinks"
        :key="backlink.note_id"
        class="backlink-item card card-hover"
        @click="$emit('navigate', backlink.note_id)"
      >
        <div class="backlink-title">{{ backlink.title }}</div>
        <div v-if="backlink.context" class="backlink-context">
          {{ backlink.context }}
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onMounted } from 'vue'
import { graph as graphApi } from '../api/client'

const props = defineProps({
  noteId: {
    type: String,
    required: true
  }
})

defineEmits(['navigate'])

const backlinks = ref([])
const loading = ref(false)

async function loadBacklinks() {
  if (!props.noteId) return
  
  loading.value = true
  try {
    const data = await graphApi.backlinks(props.noteId)
    backlinks.value = data
  } catch (error) {
    console.error('Failed to load backlinks:', error)
    backlinks.value = []
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadBacklinks()
})

watch(() => props.noteId, () => {
  loadBacklinks()
})
</script>

<style scoped>
.backlinks-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: var(--spacing-md);
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--spacing-md);
}

.panel-header h3 {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
}

.backlink-count {
  font-size: 0.75rem;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.loading-state,
.empty-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
}

.loading-state p,
.empty-state p {
  margin: 0;
  font-size: 0.875rem;
}

.backlinks-list {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
}

.backlink-item {
  cursor: pointer;
  padding: var(--spacing-sm);
}

.backlink-title {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: var(--spacing-xs);
}

.backlink-context {
  font-size: 0.75rem;
  color: var(--text-secondary);
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}
</style>
