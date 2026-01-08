<template>
  <div class="model-selector">
    <div class="selector-header">
      <h4>Select Models</h4>
      <span class="selected-count">{{ selectedModels.length }} selected</span>
    </div>

    <div class="search-box">
      <input
        v-model="searchQuery"
        type="text"
        placeholder="Search models..."
        class="search-input"
      />
    </div>

    <div class="quick-actions">
      <button class="btn btn-sm btn-ghost" @click="selectPopular">Popular</button>
      <button class="btn btn-sm btn-ghost" @click="selectAll">All</button>
      <button class="btn btn-sm btn-ghost" @click="clearSelection">Clear</button>
    </div>

    <div class="model-groups">
      <div
        v-for="(models, provider) in groupedModels"
        :key="provider"
        class="model-group"
      >
        <div class="group-header" @click="toggleGroup(provider)">
          <span class="provider-name">{{ provider }}</span>
          <span class="group-count">({{ models.length }})</span>
          <span class="expand-icon">{{ expandedGroups.has(provider) ? '-' : '+' }}</span>
        </div>

        <div v-show="expandedGroups.has(provider)" class="group-models">
          <label
            v-for="model in models"
            :key="model.id"
            class="model-option"
            :class="{ selected: selectedModels.includes(model.id) }"
          >
            <input
              type="checkbox"
              :value="model.id"
              v-model="selectedModels"
              class="model-checkbox"
            />
            <div class="model-info">
              <span class="model-name">{{ model.name }}</span>
              <span class="model-meta">
                {{ formatContextLength(model.context_length) }} ctx
              </span>
            </div>
          </label>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  modelValue: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['update:modelValue'])

// Local state
const searchQuery = ref('')
const expandedGroups = ref(new Set(['openai', 'anthropic', 'google']))

// Computed
const selectedModels = computed({
  get: () => props.modelValue,
  set: (val) => emit('update:modelValue', val)
})

const filteredModels = computed(() => {
  if (!searchQuery.value) return props.models
  const query = searchQuery.value.toLowerCase()
  return props.models.filter(m =>
    m.name.toLowerCase().includes(query) ||
    m.id.toLowerCase().includes(query) ||
    m.provider.toLowerCase().includes(query)
  )
})

const groupedModels = computed(() => {
  const groups = {}
  for (const model of filteredModels.value) {
    const provider = model.provider || 'other'
    if (!groups[provider]) groups[provider] = []
    groups[provider].push(model)
  }
  // Sort groups by name
  return Object.fromEntries(
    Object.entries(groups).sort(([a], [b]) => a.localeCompare(b))
  )
})

// Methods
function toggleGroup(provider) {
  if (expandedGroups.value.has(provider)) {
    expandedGroups.value.delete(provider)
  } else {
    expandedGroups.value.add(provider)
  }
}

function selectPopular() {
  const popular = [
    'openai/gpt-4o',
    'openai/gpt-4o-mini',
    'anthropic/claude-3.5-sonnet',
    'anthropic/claude-3-haiku',
    'google/gemini-pro',
    'meta-llama/llama-3.1-70b-instruct'
  ]
  selectedModels.value = popular.filter(id =>
    props.models.some(m => m.id === id)
  )
}

function selectAll() {
  selectedModels.value = filteredModels.value.map(m => m.id)
}

function clearSelection() {
  selectedModels.value = []
}

function formatContextLength(length) {
  if (!length) return '?'
  if (length >= 1000000) return `${(length / 1000000).toFixed(1)}M`
  if (length >= 1000) return `${(length / 1000).toFixed(0)}k`
  return length.toString()
}
</script>

<style scoped>
.model-selector {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  max-height: 400px;
}

.selector-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.selector-header h4 {
  margin: 0;
  font-size: 0.875rem;
  color: var(--text-primary);
}

.selected-count {
  font-size: 0.75rem;
  color: var(--accent-primary);
  background: rgba(124, 92, 255, 0.15);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.search-box {
  position: relative;
}

.search-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.search-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.quick-actions {
  display: flex;
  gap: var(--spacing-xs);
}

.model-groups {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.model-group {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.group-header {
  display: flex;
  align-items: center;
  padding: var(--spacing-sm);
  cursor: pointer;
  background: rgba(0, 0, 0, 0.2);
}

.group-header:hover {
  background: rgba(0, 0, 0, 0.3);
}

.provider-name {
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--text-primary);
  text-transform: capitalize;
  flex: 1;
}

.group-count {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-right: var(--spacing-sm);
}

.expand-icon {
  color: var(--text-muted);
  font-weight: 600;
  width: 16px;
  text-align: center;
}

.group-models {
  padding: var(--spacing-xs);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.model-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-xs) var(--spacing-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s;
}

.model-option:hover {
  background: rgba(255, 255, 255, 0.05);
}

.model-option.selected {
  background: rgba(124, 92, 255, 0.15);
}

.model-checkbox {
  accent-color: var(--accent-primary);
}

.model-info {
  flex: 1;
  min-width: 0;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.model-name {
  font-size: 0.8125rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.model-meta {
  font-size: 0.6875rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.btn-sm {
  padding: 4px 8px;
  font-size: 0.75rem;
}

.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--bg-tertiary);
}

.btn-ghost:hover {
  background: var(--bg-tertiary);
  border-color: var(--text-muted);
}
</style>
