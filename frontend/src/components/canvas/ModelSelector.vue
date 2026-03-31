<template>
  <div class="model-selector">
    <div class="selector-header">
      <h4>Select Models</h4>
      <span class="selected-count">{{ selectedModels.length }} selected</span>
    </div>

    <!-- Selected Models Tags -->
    <div
      v-if="selectedModels.length > 0"
      class="selected-tags"
    >
      <span
        v-for="modelId in selectedModels"
        :key="modelId"
        class="model-tag"
        :title="getModelDisplayName(modelId)"
      >
        <span class="tag-name">{{ getModelDisplayName(modelId) }}</span>
        <button
          class="tag-remove"
          title="Remove"
          @click.stop="removeModel(modelId)"
        ><GIcon
          name="x"
          :size="10"
        /></button>
      </span>
    </div>

    <div class="search-box">
      <input
        v-model="searchQuery"
        type="text"
        placeholder="Search models..."
        class="search-input"
      >
    </div>

    <div class="quick-actions">
      <div
        v-for="preset in presetSummaries"
        :key="preset.id"
        class="preset-inline"
        :class="{ active: selectedPreset && preset.id === selectedPreset.id, disabled: preset.validModelIds.length === 0 }"
      >
        <button
          class="btn btn-sm preset-chip"
          :disabled="preset.validModelIds.length === 0"
          :title="presetTooltip(preset)"
          @click="applyPreset(preset)"
        >
          <span class="preset-name">{{ preset.name }}</span>
          <span class="preset-meta">{{ preset.validModelIds.length }}</span>
        </button>
      </div>
      <button
        class="btn btn-sm btn-ghost"
        @click="selectPopular"
      >
        Popular
      </button>
      <button
        class="btn btn-sm btn-ghost"
        @click="selectAll"
      >
        All
      </button>
      <button
        class="btn btn-sm btn-ghost"
        @click="clearSelection"
      >
        Clear
      </button>
      <button
        class="btn btn-sm btn-ghost preset-manage-btn"
        :disabled="editPresetDisabled"
        :title="editPresetTitle"
        @click="updateSelectedPreset"
      >
        Update
      </button>
      <button
        class="btn btn-sm btn-ghost preset-manage-btn danger"
        :disabled="deletePresetDisabled"
        :title="deletePresetTitle"
        @click="deleteSelectedPreset"
      >
        Delete
      </button>
      <button
        class="btn btn-sm btn-ghost save-current-btn"
        :disabled="saveCurrentDisabled"
        :title="saveCurrentTitle"
        @click="openCreatePreset"
      >
        New Preset
      </button>
    </div>

    <div
      v-if="showPresetEditor"
      class="preset-editor"
    >
      <input
        ref="presetNameInput"
        v-model="presetDraftName"
        type="text"
        class="preset-input"
        placeholder="Preset name"
        @keydown.enter.prevent="submitPresetEditor"
        @keydown.escape.prevent="closePresetEditor"
      >
      <button
        class="btn btn-sm btn-primary"
        :disabled="!canSubmitPreset"
        @click="submitPresetEditor"
      >
        Save
      </button>
      <button
        class="btn btn-sm btn-ghost"
        @click="closePresetEditor"
      >
        Cancel
      </button>
    </div>

    <p
      v-if="presetHelper"
      class="preset-helper"
    >
      {{ presetHelper }}
    </p>

    <p
      v-if="presetError"
      class="preset-error"
    >
      {{ presetError }}
    </p>

    <div class="model-groups">
      <div
        v-for="(models, provider) in groupedModels"
        :key="provider"
        class="model-group"
      >
        <div
          class="group-header"
          @click="toggleGroup(provider.toLowerCase())"
        >
          <span class="provider-name">{{ provider }}</span>
          <span class="group-count">({{ models.length }})</span>
          <GIcon
            name="chevron-down"
            :size="14"
            class="expand-icon"
            :style="{ transform: expandedGroups.has(provider.toLowerCase()) ? 'rotate(180deg)' : 'rotate(0)', transition: 'transform 150ms ease' }"
          />
        </div>

        <div
          v-show="expandedGroups.has(provider.toLowerCase())"
          class="group-models"
        >
          <label
            v-for="model in models"
            :key="model.id"
            class="model-option"
            :class="{ selected: selectedModels.includes(model.id) }"
          >
            <input
              v-model="selectedModels"
              type="checkbox"
              :value="model.id"
              class="model-checkbox"
            >
            <div class="model-info">
              <span class="model-name">{{ model.name }}</span>
              <div class="model-pricing">
                <span
                  class="pricing-item"
                  title="Input cost per 1M tokens"
                >${{ formatPrice(model.pricing?.prompt) }}</span>
                <span class="pricing-divider">/</span>
                <span
                  class="pricing-item"
                  title="Output cost per 1M tokens"
                >${{ formatPrice(model.pricing?.completion) }}</span>
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
import { ref, computed, nextTick } from 'vue'
import GIcon from '@/components/ui/GIcon.vue'

const MAX_PRESETS = 8

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  modelValue: {
    type: Array,
    default: () => []
  },
  presets: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['update:modelValue', 'create-preset', 'update-preset', 'delete-preset'])

// Local state
const searchQuery = ref('')
const expandedGroups = ref(new Set(['openai', 'anthropic', 'google']))
const presetNameInput = ref(null)
const presetDraftName = ref(null)
const activePresetId = ref(null)
const presetError = ref('')

// Computed
const selectedModels = computed({
  get: () => props.modelValue,
  set: (val) => emit('update:modelValue', val)
})

const showPresetEditor = computed(() => presetDraftName.value !== null)

const availableModelIds = computed(() => new Set(props.models.map(model => model.id)))

const presetSummaries = computed(() => {
  return props.presets.map(preset => {
    const validModelIds = preset.model_ids.filter(modelId => availableModelIds.value.has(modelId))
    return {
      ...preset,
      validModelIds,
      isActive: areSameSelection(validModelIds, selectedModels.value)
    }
  })
})

const selectedPreset = computed(() => {
  if (activePresetId.value) {
    const focusedPreset = props.presets.find(preset => preset.id === activePresetId.value)
    if (focusedPreset) return focusedPreset
  }
  return presetSummaries.value.find(preset => preset.isActive) ?? null
})
const canSubmitPreset = computed(() => (presetDraftName.value ?? '').trim().length > 0)
const saveCurrentDisabled = computed(() => selectedModels.value.length === 0 || props.presets.length >= MAX_PRESETS)
const saveCurrentTitle = computed(() => {
  if (selectedModels.value.length === 0) return 'Select one or more models first'
  if (props.presets.length >= MAX_PRESETS) return `Maximum ${MAX_PRESETS} presets reached`
  return 'Create a new preset from the current model selection'
})
const editPresetDisabled = computed(() => !selectedPreset.value || selectedModels.value.length === 0)
const editPresetTitle = computed(() => {
  if (!selectedPreset.value) return 'Select a preset first'
  if (selectedModels.value.length === 0) return 'Select one or more models to update this preset'
  return `Replace "${selectedPreset.value.name}" with the current selection`
})
const deletePresetDisabled = computed(() => !selectedPreset.value)
const deletePresetTitle = computed(() => {
  if (!selectedPreset.value) return 'Select a preset first'
  return `Delete "${selectedPreset.value.name}"`
})
const presetHelper = computed(() => {
  if (presetError.value) return ''
  if (props.presets.length >= MAX_PRESETS) {
    return `Maximum ${MAX_PRESETS} presets reached. Delete one to save another.`
  }
  if (props.presets.length === 0) {
    return 'Save a model group to reuse it across canvases.'
  }
  if (!selectedPreset.value) {
    return 'Click a preset once to target it for Update/Delete, or click it again to apply it.'
  }
  return `Updating "${selectedPreset.value.name}" with the current selected models.`
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

function presetTooltip(preset) {
  if (preset.validModelIds.length === 0) {
    return 'No models from this preset are available here'
  }
  if (selectedPreset.value?.id === preset.id && !preset.isActive) {
    return `Click again to apply "${preset.name}"`
  }
  return `Target "${preset.name}" for edit/delete, or click again to apply it`
}

function openCreatePreset() {
  if (saveCurrentDisabled.value) return
  presetDraftName.value = ''
  presetError.value = ''
  focusPresetInput()
}

function closePresetEditor() {
  presetDraftName.value = null
  presetError.value = ''
}

function focusPresetInput() {
  nextTick(() => {
    presetNameInput.value?.focus()
    presetNameInput.value?.select()
  })
}

function applyPreset(preset) {
  if (preset.validModelIds.length === 0) return
  const isTargeted = selectedPreset.value?.id === preset.id
  activePresetId.value = preset.id

  if (!isTargeted && !preset.isActive && selectedModels.value.length > 0) {
    return
  }

  selectedModels.value = [...preset.validModelIds]
}

function submitPresetEditor() {
  const name = presetDraftName.value.trim()
  if (!name) {
    presetError.value = 'Preset name cannot be empty.'
    return
  }

  const normalized = name.toLowerCase()
  const duplicate = props.presets.find(preset => preset.name.trim().toLowerCase() === normalized)

  if (duplicate) {
    presetError.value = 'Preset names must be unique.'
    return
  }

  if (props.presets.length >= MAX_PRESETS) {
    presetError.value = `You can save up to ${MAX_PRESETS} presets.`
    return
  }

  emit('create-preset', {
    name,
    modelIds: [...selectedModels.value]
  })

  closePresetEditor()
}

function updateSelectedPreset() {
  if (editPresetDisabled.value || !selectedPreset.value) return
  emit('update-preset', {
    id: selectedPreset.value.id,
    modelIds: [...selectedModels.value]
  })
}

function deleteSelectedPreset() {
  if (deletePresetDisabled.value || !selectedPreset.value) return
  activePresetId.value = null
  emit('delete-preset', selectedPreset.value.id)
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

function areSameSelection(left, right) {
  if (left.length !== right.length) return false
  const leftSorted = [...left].sort()
  const rightSorted = [...right].sort()
  return leftSorted.every((value, index) => value === rightSorted[index])
}
</script>

<style scoped>
.model-selector {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  max-height: 350px;
  min-height: 200px;
}

.selector-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-shrink: 0;
}

.selector-header h4 {
  margin: 0;
  font-size: 0.875rem;
  color: var(--text-primary);
}

.preset-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 160px;
  border-color: var(--border-subtle);
  color: var(--text-secondary);
  background: transparent;
}

.preset-chip:hover:not(:disabled) {
  background: var(--bg-tertiary);
  border-color: var(--text-muted);
}

.preset-chip:disabled {
  cursor: not-allowed;
}

.preset-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.8125rem;
}

.preset-meta {
  flex-shrink: 0;
  font-size: 0.6875rem;
  color: var(--text-muted);
}

.preset-inline {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.preset-inline.active .preset-chip {
  border-color: color-mix(in srgb, var(--accent-primary) 60%, transparent);
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-secondary));
}

.preset-inline.disabled {
  opacity: 0.65;
}

.preset-manage-btn.danger:hover {
  color: #9a5d0d;
}

.preset-helper,
.preset-error {
  margin: 0;
  font-size: 0.75rem;
}

.preset-helper {
  color: var(--text-muted);
}

.preset-error {
  color: var(--accent-red, #f87171);
}

.preset-editor {
  display: flex;
  align-items: center;
  gap: 6px;
}

.preset-input {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8125rem;
}

.preset-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.selected-count {
  font-size: 0.6875rem;
  color: var(--accent-primary);
  background: rgba(124, 92, 255, 0.15);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
}

.selected-tags {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 6px;
  padding: 6px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  min-height: 34px;
  flex-shrink: 0;
}

.model-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 7px;
  background: rgba(124, 92, 255, 0.15);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  line-height: 1.1;
  color: var(--text-primary);
}

.tag-name {
  max-width: 112px;
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
  flex-shrink: 0;
}

.search-input {
  width: 100%;
  padding: 8px 10px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
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
  flex-wrap: wrap;
  gap: var(--spacing-xs);
  flex-shrink: 0;
  align-items: center;
}

.model-groups {
  flex: 1 1 auto;
  overflow-y: auto;
  display: block;
  min-height: 0;
}

.model-group {
  background: color-mix(in srgb, var(--bg-tertiary) 85%, transparent);
  border-radius: var(--radius-sm);
  overflow: visible;
  margin-bottom: var(--spacing-xs);
}

.group-header {
  display: flex;
  align-items: center;
  padding: 8px 10px;
  cursor: pointer;
  background: var(--bg-hover);
}

.group-header:hover {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
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
  transition: transform var(--transition-fast);
  display: inline-flex;
}

.group-models {
  padding: 4px;
}

.model-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s;
  min-height: 28px;
}

.model-option:hover {
  background: rgba(255, 255, 255, 0.05);
}

.model-option.selected {
  background: rgba(124, 92, 255, 0.15);
}

.model-checkbox {
  accent-color: var(--accent-primary);
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  flex-grow: 0;
}

.model-info {
  flex: 1;
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 8px;
}

.model-name {
  flex: 1;
  font-size: 0.775rem;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-pricing {
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 0.625rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.pricing-item {
  min-width: 28px;
  text-align: right;
}

.pricing-divider {
  color: var(--bg-tertiary);
}

.model-ctx {
  font-size: 0.625rem;
  color: var(--text-muted);
  min-width: 32px;
  text-align: right;
  flex-shrink: 0;
}

.btn-sm {
  padding: 3px 8px;
  font-size: 0.6875rem;
}

.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border-subtle);
}

.btn-ghost:hover {
  background: var(--bg-tertiary);
  border-color: var(--text-muted);
}

.preset-manage-btn {
  background: rgba(217, 145, 51, 0.12);
  border-color: rgba(217, 145, 51, 0.3);
  color: #9a5d0d;
}

.preset-manage-btn:hover:not(:disabled) {
  background: rgba(217, 145, 51, 0.18);
  border-color: rgba(217, 145, 51, 0.42);
}

.save-current-btn {
  background: rgba(37, 153, 137, 0.14);
  border-color: rgba(37, 153, 137, 0.34);
  color: #1f7f74;
}

.save-current-btn:hover:not(:disabled) {
  background: rgba(37, 153, 137, 0.2);
  border-color: rgba(37, 153, 137, 0.45);
}
</style>
