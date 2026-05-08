<template>
  <div
    class="dialog-overlay"
    @click.self="$emit('cancel')"
  >
    <div class="dialog-content">
      <div class="dialog-header">
        <h3>{{ branchContext ? 'Branch Prompt' : 'New Prompt' }}</h3>
        <button
          class="close-btn"
          @click="$emit('cancel')"
        >
          <GIcon
            name="x"
            :size="14"
          />
        </button>
      </div>

      <!-- Branch Context Banner -->
      <div
        v-if="branchContext"
        class="branch-context"
      >
        <GIcon
          name="git-branch"
          :size="16"
          class="branch-icon"
        />
        <span class="branch-text">
          Branching from <strong>{{ branchContext.parentContent?.model?.split('/').pop() || 'response' }}</strong>
        </span>
      </div>

      <div class="dialog-body">
        <div class="form-group prompt-type-group">
          <label>Type</label>
          <div class="segmented-control">
            <button
              type="button"
              :class="{ active: promptType === 'standard' }"
              @click="setPromptType('standard')"
            >
              Prompt
            </button>
            <button
              type="button"
              :class="{ active: promptType === 'decision' }"
              @click="setPromptType('decision')"
            >
              Decision
            </button>
          </div>
        </div>

        <div class="form-group">
          <label for="prompt">{{ promptLabel }}</label>
          <textarea
            id="prompt"
            v-model="prompt"
            :placeholder="promptPlaceholder"
            rows="4"
            class="prompt-input"
            @keydown.ctrl.enter="handleSubmit"
          />
        </div>

        <div
          v-if="promptType === 'decision'"
          class="decision-fields"
        >
          <div class="form-group">
            <label for="decisionOptions">Options</label>
            <textarea
              id="decisionOptions"
              v-model="decisionOptionsText"
              placeholder="One option per line"
              rows="3"
              class="system-input"
            />
          </div>
          <div class="form-group">
            <label for="decisionStakes">Stakes</label>
            <input
              id="decisionStakes"
              v-model="decisionStakes"
              type="text"
              class="text-input"
              placeholder="What changes if this goes right or wrong?"
            >
          </div>
          <div class="form-group">
            <label for="decisionLeaning">Initial Leaning</label>
            <input
              id="decisionLeaning"
              v-model="decisionInitialLeaning"
              type="text"
              class="text-input"
              placeholder="Optional current leaning"
            >
          </div>
          <div class="form-group">
            <label for="decisionReviewDate">Follow-up</label>
            <input
              id="decisionReviewDate"
              v-model="decisionReviewDate"
              type="date"
              class="text-input"
            >
          </div>
        </div>

        <div class="form-group checkbox-group">
          <label class="checkbox-label">
            <input
              v-model="showSystemPrompt"
              type="checkbox"
            >
            <span>Add system prompt</span>
          </label>
          <textarea
            v-if="showSystemPrompt"
            v-model="systemPrompt"
            placeholder="Optional system prompt..."
            rows="2"
            class="system-input"
          />
        </div>

        <div class="form-group">
          <ModelSelector
            v-model="selectedModels"
            :models="models"
            :presets="presets"
            @create-preset="emit('create-preset', $event)"
            @update-preset="emit('update-preset', $event)"
            @delete-preset="emit('delete-preset', $event)"
          />
        </div>

        <!-- Context Mode Selector (always visible) -->
        <div class="form-group context-mode-group">
          <label for="contextMode">Context Mode</label>
          <select
            id="contextMode"
            v-model="contextMode"
            class="select-input"
          >
            <option value="knowledge_search">
              Vault Notes (relevant notes)
            </option>
            <option value="twin">
              Twin (notes + user records)
            </option>
            <option value="none">
              None (no additional context)
            </option>
            <option
              v-if="branchContext"
              value="full_history"
            >
              Full History (all previous turns)
            </option>
            <option
              v-if="branchContext"
              value="compact"
            >
              Compact (summary + recent)
            </option>
          </select>
          <span class="context-mode-hint">
            {{ contextModeHints[contextMode] }}
          </span>
        </div>

        <div
          v-if="contextMode === 'twin'"
          class="form-group twin-mode-group"
        >
          <label>Twin Answer Mode</label>
          <div class="segmented-control">
            <button
              type="button"
              :class="{ active: twinAnswerMode === 'advisor' }"
              @click="twinAnswerMode = 'advisor'"
            >
              Advisor
            </button>
            <button
              type="button"
              :class="{ active: twinAnswerMode === 'simulation' }"
              @click="twinAnswerMode = 'simulation'"
            >
              Simulation
            </button>
          </div>
          <span class="context-mode-hint">
            {{ twinModeHints[twinAnswerMode] }}
          </span>
          <span class="context-mode-hint">
            Selected vault notes and twin records may be sent to the chosen model runtime.
          </span>
        </div>

        <div
          class="web-search-hint"
          :class="{ disabled: !resolvedWebSearch }"
        >
          <GIcon
            name="globe"
            :size="14"
            class="hint-icon"
          />
          <span class="hint-text">{{ webSearchHint }}</span>
        </div>

        <div
          class="advanced-toggle"
          @click="showAdvanced = !showAdvanced"
        >
          <span>Advanced Options</span>
          <GIcon
            name="chevron-down"
            :size="14"
            class="toggle-icon"
            :style="{ transform: showAdvanced ? 'rotate(180deg)' : 'rotate(0)' }"
          />
        </div>

        <div
          v-if="showAdvanced"
          class="advanced-options"
        >
          <div class="form-group">
            <label for="temperature">Temperature: {{ temperature }}</label>
            <input
              id="temperature"
              v-model.number="temperature"
              type="range"
              min="0"
              max="2"
              step="0.1"
              class="slider"
            >
          </div>

          <!-- Context Budget Display -->
          <div
            v-if="selectedModels.length > 0"
            class="form-group"
          >
            <ContextBudgetDisplay
              :current-tokens="currentTokens"
              :max-tokens="maxContextTokens"
            />
          </div>
        </div>
      </div>

      <div class="dialog-footer">
        <button
          class="btn btn-secondary"
          @click="$emit('cancel')"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="!canSubmit"
          @click="handleSubmit"
        >
          {{ submitLabel }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import ModelSelector from './ModelSelector.vue'
import ContextBudgetDisplay from './ContextBudgetDisplay.vue'
import GIcon from '@/components/ui/GIcon.vue'
import { detectWebSearch } from '@/composables/useWebSearchDetection'

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  presets: {
    type: Array,
    default: () => []
  },
  branchContext: {
    type: Object,
    default: null
  },
  smartWebSearch: {
    type: Boolean,
    default: true
  },
  openRouterConfigured: {
    type: Boolean,
    default: true
  }
})

const emit = defineEmits(['submit', 'cancel', 'create-preset', 'update-preset', 'delete-preset'])

// Form state
const prompt = ref('')
const systemPrompt = ref('')
const showSystemPrompt = ref(false)
const selectedModels = ref([])
const temperature = ref(0.7)
const showAdvanced = ref(false)
const contextMode = ref('knowledge_search')  // Default to knowledge search for note lookup
const twinAnswerMode = ref('advisor')
const promptType = ref('standard')
const decisionOptionsText = ref('')
const decisionStakes = ref('')
const decisionInitialLeaning = ref('')
const decisionReviewDate = ref('')

const searchDetection = computed(() => detectWebSearch(prompt.value))
const resolvedWebSearch = computed(() => (
  props.openRouterConfigured &&
  props.smartWebSearch &&
  searchDetection.value.shouldSearch
))
const webSearchHint = computed(() => {
  if (!props.openRouterConfigured) {
    return 'Live web search is off because OpenRouter is not configured.'
  }

  if (!props.smartWebSearch) {
    return 'Live web search is off for this prompt. Enable Canvas Web Search in Settings to use live sources.'
  }

  if (!prompt.value.trim()) {
    return 'Live web search turns on automatically for prompts that look freshness-sensitive or explicitly ask for research.'
  }

  if (searchDetection.value.shouldSearch) {
    return `Live web search will run for this prompt (${searchDetection.value.reason}).`
  }

  return 'This prompt looks self-contained, so live web search will stay off.'
})

// Context mode descriptions
const contextModeHints = {
  none: 'No additional context - just your prompt',
  knowledge_search: 'Retrieves relevant notes (+ pinned notes) from your vault as LLM context. This does not search the live web.',
  twin: 'Retrieves relevant notes plus approved user records and relevant candidate records from Twin Review.',
  full_history: 'Include all conversation turns from the parent chain',
  compact: 'Include recent turns + summary of older context to save tokens'
}

const twinModeHints = {
  advisor: 'Decision-support mode uses reviewed memory to help the user reason.',
  simulation: 'Simulation mode is labeled as likely-user-style reflection, not the user actual view.'
}

// Computed
const canSubmit = computed(() => {
  return prompt.value.trim().length > 0 && selectedModels.value.length > 0
})

const promptLabel = computed(() => promptType.value === 'decision' ? 'Decision' : 'Prompt')

const promptPlaceholder = computed(() => promptType.value === 'decision'
  ? 'What decision are you making?'
  : 'Enter your prompt...')

const decisionOptions = computed(() => decisionOptionsText.value
  .split('\n')
  .map(option => option.trim())
  .filter(Boolean))

const decisionMetadata = computed(() => {
  if (promptType.value !== 'decision') return null

  return {
    decision: prompt.value.trim(),
    options: decisionOptions.value,
    stakes: decisionStakes.value.trim() || null,
    initial_leaning: decisionInitialLeaning.value.trim() || null,
    review_date: decisionReviewDate.value || null
  }
})

const submitLabel = computed(() => {
  const subject = promptType.value === 'decision' ? 'Create Reflection Card' : 'Send'
  return `${subject} (${selectedModels.value.length} model${selectedModels.value.length !== 1 ? 's' : ''})`
})

// Token counting for context budget
const estimatedTokens = computed(() => {
  // Estimate tokens: ~4 characters per token for English text
  const charsPerToken = 4
  
  // Count tokens from prompt
  let totalChars = prompt.value?.length || 0
  
  // Count tokens from system prompt if present
  if (showSystemPrompt.value) {
    totalChars += systemPrompt.value?.length || 0
  }
  
  // Estimate context tokens based on mode
  const contextMultiplier = {
    none: 1.0,          // No additional context
    knowledge_search: 1.3,  // Knowledge search results
    twin: 1.6,              // Notes + twin records
    full_history: 1.5,  // Include conversation history
    compact: 1.2        // Compact summary
  }
  const multiplier = contextMultiplier[contextMode.value] || 1.0
  totalChars *= multiplier
  
  return Math.ceil(totalChars / charsPerToken)
})

const maxContextTokens = computed(() => {
  // Get the first selected model's context limit
  const firstModelId = selectedModels.value[0]
  if (!firstModelId) return 4096
  
  const model = props.models.find(m => m.id === firstModelId)
  return model?.context_length || 4096
})

const currentTokens = computed(() => estimatedTokens.value)

// Methods
function setPromptType(type) {
  promptType.value = type
}

watch(promptType, (type) => {
  if (type === 'decision') {
    contextMode.value = 'twin'
    twinAnswerMode.value = 'advisor'
    temperature.value = Math.min(temperature.value, 0.5)
  }
})

function handleSubmit() {
  if (!canSubmit.value) return

  emit('submit', {
    prompt: prompt.value.trim(),
    promptType: promptType.value,
    decisionMetadata: decisionMetadata.value,
    models: selectedModels.value,
    systemPrompt: showSystemPrompt.value ? systemPrompt.value.trim() : null,
    temperature: temperature.value,
    contextMode: contextMode.value,
    twinAnswerMode: twinAnswerMode.value,
    webSearch: resolvedWebSearch.value
  })
}
</script>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: var(--spacing-lg);
}

.dialog-content {
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 600px;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  box-shadow: var(--shadow-xl);
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--border-subtle);
}

.dialog-header h3 {
  margin: 0;
  font-size: 1.125rem;
  color: var(--text-primary);
}

.branch-context {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-lg);
  background: rgba(124, 92, 255, 0.1);
  border-bottom: 1px solid rgba(124, 92, 255, 0.2);
  color: var(--accent-primary);
  font-size: 0.875rem;
}

.branch-icon {
  font-size: 1rem;
}

.branch-text strong {
  color: var(--text-primary);
}

.close-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  font-size: 1rem;
}

.close-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.dialog-body {
  flex: 1;
  padding: var(--spacing-lg);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.form-group label {
  font-size: 0.875rem;
  color: var(--text-secondary);
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.checkbox-group {
  align-items: flex-start;
}

.checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: var(--spacing-sm);
  cursor: pointer;
  white-space: nowrap;
  font-size: 0.8125rem;
  color: var(--text-secondary);
}

.checkbox-label input[type="checkbox"] {
  margin: 0;
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  accent-color: var(--accent-primary);
}

.prompt-input,
.system-input,
.text-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  font-family: inherit;
}

.prompt-input,
.system-input {
  resize: vertical;
}

.prompt-input:focus,
.system-input:focus,
.text-input:focus {
  border-color: var(--accent-primary);
  outline: none;
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-primary) 30%, transparent);
}

.prompt-input {
  min-height: 100px;
}

.decision-fields {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing-md);
  padding: var(--spacing-md);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-tertiary) 70%, transparent);
}

.decision-fields .form-group:first-child {
  grid-column: 1 / -1;
}

.advanced-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.advanced-toggle:hover {
  background: var(--bg-hover);
}

.toggle-icon {
  font-weight: 600;
  transition: transform var(--transition-fast);
  display: inline-flex;
}

.advanced-options {
  padding: var(--spacing-md);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.slider {
  width: 100%;
  accent-color: var(--accent-primary);
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--border-subtle);
}

.context-mode-group {
  margin-top: var(--spacing-md);
}

.select-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  cursor: pointer;
}

.select-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.context-mode-hint {
  display: block;
  margin-top: var(--spacing-xs);
  font-size: 0.75rem;
  color: var(--text-muted);
}

.segmented-control {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 2px;
  padding: 2px;
  background: var(--bg-primary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
}

.segmented-control button {
  border: none;
  border-radius: calc(var(--radius-sm) - 2px);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 0.8125rem;
  font-weight: 600;
  min-height: 32px;
}

.segmented-control button.active {
  background: var(--accent-primary);
  color: white;
}

.web-search-hint {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-xs) var(--spacing-sm);
  background: color-mix(in srgb, var(--accent-cyan, #06b6d4) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent-cyan, #06b6d4) 25%, transparent);
  border-radius: var(--radius-sm);
  font-size: 0.8125rem;
  color: var(--accent-cyan, #06b6d4);
}

.web-search-hint.disabled {
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--bg-tertiary) 72%, transparent);
  border-color: color-mix(in srgb, var(--bg-tertiary) 82%, transparent);
}

.hint-icon {
  font-size: 0.875rem;
  flex-shrink: 0;
}

.hint-text {
  flex: 1;
}
</style>
