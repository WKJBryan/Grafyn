<template>
  <div class="tag-tree">
    <PanelHeader
      title="Tags"
      :count="totalTags > 0 ? totalTags : null"
      :collapsible="true"
      :expanded="isExpanded"
      @toggle="isExpanded = !isExpanded"
    />

    <div
      v-if="isExpanded"
      class="tree-content"
    >
      <AsyncListState
        :loading="loading"
        :empty="Object.keys(tagTree).length === 0"
      >
        <template #loading>
          <div class="tag-tree__state">
            <span class="loading-spinner">⏳</span>
            <span>Loading tags...</span>
          </div>
        </template>
        <template #empty>
          <div class="tag-tree__state">
            <span>🏷️</span>
            <span>No tags found</span>
          </div>
        </template>

        <div class="tree-list">
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
      </AsyncListState>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, defineAsyncComponent } from 'vue'
import PanelHeader from './PanelHeader.vue'
import AsyncListState from './AsyncListState.vue'

// Self-referencing component for recursive tree rendering
const TagTreeNode = defineAsyncComponent(() => import('./TagTreeNode.vue'))

const props = defineProps({
  tags: { type: Array, default: () => [] }
})

const emit = defineEmits(['filter'])

const isExpanded = ref(true)
const loading = ref(false)
const selectedTags = ref(new Set())

const tagTree = computed(() => {
  const tree = {}
  for (const tag of props.tags) {
    const parts = tag.split('/')
    let current = tree
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      if (!current[part]) current[part] = {}
      current = current[part]
    }
  }
  return tree
})

const tagCounts = computed(() => {
  const counts = {}
  for (const tag of props.tags) {
    counts[tag] = (counts[tag] || 0) + 1
    const parts = tag.split('/')
    let path = ''
    for (const part of parts) {
      path = path ? `${path}/${part}` : part
      if (!counts[path]) counts[path] = 0
    }
  }
  return counts
})

const totalTags = computed(() => new Set(props.tags).size)

function handleTagToggle(tagPath) {
  if (selectedTags.value.has(tagPath)) {
    selectedTags.value.delete(tagPath)
  } else {
    selectedTags.value.add(tagPath)
  }
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

.tree-content {
  padding: var(--spacing-sm) var(--spacing-md) var(--spacing-md);
  border-top: 1px solid var(--bg-tertiary);
}

.tag-tree__state {
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
