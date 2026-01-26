<template>
  <div class="model-selector">
    <div class="selector-header">
      <h4>Select Models</h4>
      <span class="selected-count">{{ selectedModels.length }} selected</span>
    </div>

    <!-- Selected Models Tags -->
    <div v-if="selectedModels.length > 0" class="selected-tags">
      <span
        v-for="modelId in selectedModels"
        :key="modelId"
        class="model-tag"
      >
        <span class="tag-name">{{ getModelDisplayName(modelId) }}</span>
        <button class="tag-remove" @click.stop="removeModel(modelId)" title="Remove">&times;</button>
      </span>
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
        <div class="group-header" @click="toggleGroup(provider.toLowerCase())">
          <span class="provider-name">{{ provider }}</span>
          <span class="group-count">({{ models.length }})</span>
          <span class="expand-icon">{{ expandedGroups.has(provider.toLowerCase()) ? '-' : '+' }}</span>
        </div>

        <div v-show="expandedGroups.has(provider.toLowerCase())" class="group-models">
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
              <div class="model-pricing">
                <span class="pricing-item" title="Input cost per 1M tokens">${{ formatPrice(model.pricing?.prompt) }}</span>
                <span class="pricing-divider">/</span>
                <span class="pricing-item" title="Output cost per 1M tokens">${{ formatPrice(model.pricing?.completion) }}</span>
              </div>
              <span class="model-ctx">{{ formatContextLength(model.context_length) }}</span>
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

function formatPrice(price) {
  if (price === undefined || price === null) return '?'
  // Price is typically per token, convert to per 1M tokens
  const perMillion = price * 1000000
  if (perMillion < 0.01) return perMillion.toFixed(4)
  if (perMillion < 1) return perMillion.toFixed(2)
  return perMillion.toFixed(1)
}

function getModelDisplayName(modelId) {
  const model = props.models.find(m => m.id === modelId)
  if (model) {
    // Return short name - extract model name without provider prefix
    const parts = model.name.split(':')
    return parts.length > 1 ? parts[1].trim() : model.name
  }
  // Fallback: extract from model ID
  const parts = modelId.split('/')
  return parts.length > 1 ? parts[1] : modelId
}

function removeModel(modelId) {
  selectedModels.value = selectedModels.value.filter(id => id !== modelId)
}
</script>

<style scoped>
.model-selector {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  max-height: 350px;
  min-height: 200px;
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

.selected-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  min-height: 40px;
  max-height: 80px;
  overflow-y: auto;
}

.model-tag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  background: rgba(124, 92, 255, 0.15);
  border: 1px solid rgba(124, 92, 255, 0.3);
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  color: var(--text-primary);
}

.tag-name {
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tag-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
  border-radius: 50%;
}

.tag-remove:hover {
  background: rgba(255, 255, 255, 0.1);
  color: var(--text-primary);
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
  flex: 1 1 auto;
  overflow-y: auto;
  display: block;
  min-height: 0;
}

.model-group {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: visible;
  margin-bottom: var(--spacing-xs);
}

.group-header {
  display: flex;
  align-items: center;
  padding: var(--spacing-sm);
  cursor: pointer;
  background: var(--bg-hover);
}

.group-header:hover {
  background: var(--bg-tertiary);
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
}

.model-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-xs) var(--spacing-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s;
  min-height: 32px;
}

.model-option:hover {
  background: rgba(255, 255, 255, 0.05);
}

.model-option.selected {
  background: rgba(124, 92, 255, 0.15);
}

.model-checkbox {
  accent-color: var(--accent-primary);
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  flex-grow: 0;
}

.model-info {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.model-name {
  flex: 1;
  font-size: 0.8125rem;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-pricing {
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 0.6875rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.pricing-item {
  min-width: 32px;
  text-align: right;
}

.pricing-divider {
  color: var(--bg-tertiary);
}

.model-ctx {
  font-size: 0.6875rem;
  color: var(--text-muted);
  min-width: 35px;
  text-align: right;
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
