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
          &#10005;
        </button>
      </div>

      <!-- Branch Context Banner -->
      <div
        v-if="branchContext"
        class="branch-context"
      >
        <span class="branch-icon">⑂</span>
        <span class="branch-text">
          Branching from <strong>{{ branchContext.parentContent?.model?.split('/').pop() || 'response' }}</strong>
        </span>
      </div>

      <div class="dialog-body">
        <div class="form-group">
          <label for="prompt">Prompt</label>
          <textarea
            id="prompt"
            v-model="prompt"
            placeholder="Enter your prompt..."
            rows="4"
            class="prompt-input"
            @keydown.ctrl.enter="handleSubmit"
          />
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
              Knowledge Search (relevant notes)
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
          class="advanced-toggle"
          @click="showAdvanced = !showAdvanced"
        >
          <span>Advanced Options</span>
          <span class="toggle-icon">{{ showAdvanced ? '-' : '+' }}</span>
        </div>

        <div
          v-if="showAdvanced"
          class="advanced-options"
        >
          <div class="form-row">
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
            <div class="form-group">
              <label for="maxTokens">Max Tokens</label>
              <input
                id="maxTokens"
                v-model.number="maxTokens"
                type="number"
                min="100"
                max="32000"
                step="100"
                class="number-input"
              >
            </div>
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
          Send to {{ selectedModels.length }} Model{{ selectedModels.length !== 1 ? 's' : '' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import ModelSelector from './ModelSelector.vue'
import ContextBudgetDisplay from './ContextBudgetDisplay.vue'

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  branchContext: {
    type: Object,
    default: null
  }
})

const emit = defineEmits(['submit', 'cancel'])

// Form state
const prompt = ref('')
const systemPrompt = ref('')
const showSystemPrompt = ref(false)
const selectedModels = ref([])
const temperature = ref(0.7)
const maxTokens = ref(2048)
const showAdvanced = ref(false)
const contextMode = ref('knowledge_search')  // Default to knowledge search for note lookup

// Context mode descriptions
const contextModeHints = {
  none: 'No additional context - just your prompt',
  knowledge_search: 'Retrieves relevant notes (+ pinned notes) as LLM context',
  full_history: 'Include all conversation turns from the parent chain',
  compact: 'Include recent turns + summary of older context to save tokens'
}

// Computed
const canSubmit = computed(() => {
  return prompt.value.trim().length > 0 && selectedModels.value.length > 0
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
function handleSubmit() {
  if (!canSubmit.value) return

  emit('submit', {
    prompt: prompt.value.trim(),
    models: selectedModels.value,
    systemPrompt: showSystemPrompt.value ? systemPrompt.value.trim() : null,
    temperature: temperature.value,
    maxTokens: maxTokens.value,
    contextMode: contextMode.value
  })
}
</script>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: var(--spacing-lg);
}

.dialog-content {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 600px;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
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
.system-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  font-family: inherit;
  resize: vertical;
}

.prompt-input:focus,
.system-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.prompt-input {
  min-height: 100px;
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
}

.advanced-options {
  padding: var(--spacing-md);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.form-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--spacing-md);
}

.slider {
  width: 100%;
  accent-color: var(--accent-primary);
}

.number-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.number-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
}

.context-mode-group {
  margin-top: var(--spacing-md);
}

.select-input {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
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
</style>
