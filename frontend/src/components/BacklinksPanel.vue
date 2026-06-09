<template>
  <div class="backlinks-panel">
    <PanelHeader
      title="Backlinks"
      :count="backlinks.length > 0 ? backlinks.length : null"
      badge-variant="muted"
      class="backlinks-panel__header"
    />

    <AsyncListState
      :loading="loading"
      :empty="backlinks.length === 0"
    >
      <template #empty>
        <div class="async-list-state__empty">
          <p class="text-muted">
            No backlinks yet
          </p>
        </div>
      </template>

      <div class="backlinks-list">
        <div
          v-for="backlink in backlinks"
          :key="backlink.note_id"
          class="backlink-item card card-hover"
          @click="$emit('navigate', backlink.note_id)"
        >
          <div class="backlink-title">
            {{ backlink.title }}
          </div>
          <div
            v-if="backlink.context"
            class="backlink-context"
          >
            {{ backlink.context }}
          </div>
        </div>
      </div>
    </AsyncListState>
  </div>
</template>

<script setup>
import { ref, watch, onMounted } from 'vue'
import { graph as graphApi } from '../api/client'
import PanelHeader from './PanelHeader.vue'
import AsyncListState from './AsyncListState.vue'

const props = defineProps({
  noteId: { type: String, required: true }
})

defineEmits(['navigate'])

const backlinks = ref([])
const loading = ref(false)

async function loadBacklinks() {
  if (!props.noteId) return
  loading.value = true
  try {
    backlinks.value = await graphApi.backlinks(props.noteId)
  } catch (error) {
    console.error('Failed to load backlinks:', error)
    backlinks.value = []
  } finally {
    loading.value = false
  }
}

onMounted(() => loadBacklinks())
watch(() => props.noteId, () => loadBacklinks())
</script>

<style scoped>
.backlinks-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: var(--spacing-md);
}

.backlinks-panel__header {
  justify-content: space-between;
  margin-bottom: var(--spacing-md);
}

:deep(.panel-header-base__title) {
  font-size: 1rem;
  color: var(--text-primary);
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
