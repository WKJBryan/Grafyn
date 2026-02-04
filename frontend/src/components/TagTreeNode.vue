<template>
  <div class="tree-node">
    <div 
      class="node-header"
      :class="{ selected: isSelected, 'has-children': hasChildren }"
      @click.stop="handleClick"
    >
      <span 
        v-if="hasChildren" 
        class="expand-icon"
        @click.stop="isExpanded = !isExpanded"
      >
        {{ isExpanded ? '▼' : '▶' }}
      </span>
      <span
        v-else
        class="expand-spacer"
      />
      
      <span class="tag-icon">🏷️</span>
      <span class="tag-name">{{ name }}</span>
      
      <span
        v-if="count > 0"
        class="tag-count"
      >{{ count }}</span>
    </div>
    
    <div
      v-if="hasChildren && isExpanded"
      class="node-children"
    >
      <TagTreeNode
        v-for="(grandchildren, childName) in children"
        :key="childName"
        :name="childName"
        :children="grandchildren"
        :counts="counts"
        :parent-path="fullPath"
        :selected-tags="selectedTags"
        @toggle="$emit('toggle', $event)"
      />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, defineAsyncComponent } from 'vue'

// Self-reference for recursion
const TagTreeNode = defineAsyncComponent(() => import('./TagTreeNode.vue'))

const props = defineProps({
  name: {
    type: String,
    required: true
  },
  children: {
    type: Object,
    default: () => ({})
  },
  counts: {
    type: Object,
    default: () => ({})
  },
  parentPath: {
    type: String,
    default: ''
  },
  selectedTags: {
    type: Set,
    default: () => new Set()
  }
})

const emit = defineEmits(['toggle'])

const isExpanded = ref(false)

const fullPath = computed(() => {
  return props.parentPath ? `${props.parentPath}/${props.name}` : props.name
})

const hasChildren = computed(() => {
  return Object.keys(props.children).length > 0
})

const count = computed(() => {
  return props.counts[fullPath.value] || 0
})

const isSelected = computed(() => {
  return props.selectedTags.has(fullPath.value)
})

function handleClick() {
  emit('toggle', fullPath.value)
}
</script>

<style scoped>
.tree-node {
  display: flex;
  flex-direction: column;
}

.node-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.node-header:hover {
  background: var(--bg-hover);
}

.node-header.selected {
  background: var(--accent-primary);
  color: white;
}

.node-header.selected .tag-count {
  background: rgba(255, 255, 255, 0.3);
  color: white;
}

.expand-icon {
  font-size: 0.625rem;
  color: var(--text-muted);
  width: 12px;
  text-align: center;
  cursor: pointer;
}

.expand-spacer {
  width: 12px;
}

.tag-icon {
  font-size: 0.75rem;
}

.tag-name {
  flex: 1;
  font-size: 0.8125rem;
  color: var(--text-primary);
}

.node-header.selected .tag-name {
  color: white;
}

.tag-count {
  background: var(--bg-tertiary);
  color: var(--text-muted);
  font-size: 0.6875rem;
  padding: 1px 5px;
  border-radius: var(--radius-full);
  min-width: 16px;
  text-align: center;
}

.node-children {
  margin-left: 16px;
  border-left: 1px dashed var(--bg-tertiary);
  padding-left: 8px;
}
</style>
