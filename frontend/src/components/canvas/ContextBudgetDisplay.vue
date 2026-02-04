<template>
  <div
    class="context-budget-display"
    :class="{ warning: isWarning, error: isError }"
  >
    <div class="budget-header">
      <span class="budget-label">Context Budget</span>
      <span class="budget-count">{{ currentTokens.toLocaleString() }} / {{ maxTokens.toLocaleString() }}</span>
    </div>
    
    <div class="progress-bar-container">
      <div 
        class="progress-bar" 
        :style="{ width: `${percentage}%` }"
        :class="{ 'warning-bar': isWarning, 'error-bar': isError }"
      />
    </div>
    
    <div class="budget-footer">
      <span class="percentage-text">{{ percentage.toFixed(1) }}%</span>
      <span
        v-if="isError"
        class="status-text error-text"
      >Over limit!</span>
      <span
        v-else-if="isWarning"
        class="status-text warning-text"
      >Approaching limit</span>
      <span
        v-else
        class="status-text"
      >OK</span>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  // Current token count
  currentTokens: {
    type: Number,
    default: 0
  },
  // Maximum context limit for the model
  maxTokens: {
    type: Number,
    default: 4096
  },
  // Optional: Show compact mode for smaller displays
  compact: {
    type: Boolean,
    default: false
  }
})

// Computed
const percentage = computed(() => {
  if (props.maxTokens === 0) return 0
  return Math.min((props.currentTokens / props.maxTokens) * 100, 100)
})

const isWarning = computed(() => {
  return percentage.value >= 80 && percentage.value < 100
})

const isError = computed(() => {
  return percentage.value >= 100
})
</script>

<style scoped>
.context-budget-display {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  border: 1px solid var(--bg-tertiary);
  transition: border-color 0.2s;
}

.context-budget-display.warning {
  border-color: var(--accent-yellow, #fbbf24);
  background: rgba(251, 191, 36, 0.1);
}

.context-budget-display.error {
  border-color: var(--accent-red, #f87171);
  background: rgba(248, 113, 113, 0.1);
}

.budget-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 0.75rem;
}

.budget-label {
  color: var(--text-secondary);
  font-weight: 500;
}

.budget-count {
  color: var(--text-primary);
  font-weight: 600;
}

.progress-bar-container {
  width: 100%;
  height: 6px;
  background: var(--bg-primary);
  border-radius: 3px;
  overflow: hidden;
}

.progress-bar {
  height: 100%;
  background: var(--accent-primary);
  border-radius: 3px;
  transition: width 0.3s ease, background-color 0.3s ease;
}

.progress-bar.warning-bar {
  background: var(--accent-yellow, #fbbf24);
}

.progress-bar.error-bar {
  background: var(--accent-red, #f87171);
}

.budget-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 0.6875rem;
}

.percentage-text {
  color: var(--text-muted);
}

.status-text {
  color: var(--text-muted);
}

.status-text.warning-text {
  color: var(--accent-yellow, #fbbf24);
  font-weight: 500;
}

.status-text.error-text {
  color: var(--accent-red, #f87171);
  font-weight: 600;
}

/* Compact mode for smaller displays */
.context-budget-display.compact {
  gap: 2px;
  padding: var(--spacing-xs);
}

.context-budget-display.compact .budget-header {
  font-size: 0.6875rem;
}

.context-budget-display.compact .progress-bar-container {
  height: 4px;
}

.context-budget-display.compact .budget-footer {
  font-size: 0.625rem;
}
</style>
