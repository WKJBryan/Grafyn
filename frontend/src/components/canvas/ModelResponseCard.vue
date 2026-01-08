<template>
  <div class="model-response-card" :class="statusClass">
    <div class="response-header">
      <span class="model-name">{{ response.model_name }}</span>
      <span class="status-indicator" :title="response.status">
        <span v-if="isStreaming" class="streaming-dot"></span>
        <span v-else-if="response.status === 'completed'" class="status-complete">&#10003;</span>
        <span v-else-if="response.status === 'error'" class="status-error">&#10005;</span>
        <span v-else class="status-pending">&#8230;</span>
      </span>
    </div>

    <div class="response-content" ref="contentRef">
      <div v-if="response.status === 'pending'" class="loading-state">
        <div class="loading-dots">
          <span></span><span></span><span></span>
        </div>
        <span>Waiting for response...</span>
      </div>
      <div v-else-if="response.status === 'error'" class="error-state">
        <span class="error-icon">!</span>
        <span class="error-message">{{ response.error_message || 'An error occurred' }}</span>
      </div>
      <div v-else class="content-text" v-html="renderedContent"></div>
    </div>

    <div class="response-footer" v-if="response.tokens_used">
      <span class="tokens">{{ response.tokens_used }} tokens</span>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch, nextTick } from 'vue'
import { marked } from 'marked'

const props = defineProps({
  response: {
    type: Object,
    required: true
  },
  isStreaming: {
    type: Boolean,
    default: false
  }
})

const contentRef = ref(null)

// Computed
const statusClass = computed(() => ({
  streaming: props.isStreaming,
  completed: props.response.status === 'completed',
  error: props.response.status === 'error',
  pending: props.response.status === 'pending'
}))

const renderedContent = computed(() => {
  if (!props.response.content) return ''
  // Configure marked for safe rendering
  marked.setOptions({
    breaks: true,
    gfm: true
  })
  return marked(props.response.content)
})

// Auto-scroll during streaming
watch(() => props.response.content, () => {
  if (props.isStreaming && contentRef.value) {
    nextTick(() => {
      contentRef.value.scrollTop = contentRef.value.scrollHeight
    })
  }
})
</script>

<style scoped>
.model-response-card {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 120px;
  max-height: 300px;
}

.model-response-card.streaming {
  border: 1px solid var(--accent-blue);
}

.model-response-card.completed {
  border: 1px solid transparent;
}

.model-response-card.error {
  border: 1px solid var(--accent-red);
}

.response-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.2);
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.model-name {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: capitalize;
}

.status-indicator {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
}

.streaming-dot {
  width: 8px;
  height: 8px;
  background: var(--accent-blue);
  border-radius: 50%;
  animation: pulse 1s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(0.8); }
}

.status-complete {
  color: var(--accent-green);
  font-size: 0.875rem;
}

.status-error {
  color: var(--accent-red);
  font-size: 0.875rem;
}

.status-pending {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.response-content {
  flex: 1;
  padding: var(--spacing-sm);
  overflow-y: auto;
  font-size: 0.8125rem;
  line-height: 1.5;
}

.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-muted);
  gap: var(--spacing-sm);
}

.loading-dots {
  display: flex;
  gap: 4px;
}

.loading-dots span {
  width: 6px;
  height: 6px;
  background: var(--text-muted);
  border-radius: 50%;
  animation: bounce 1.4s infinite ease-in-out both;
}

.loading-dots span:nth-child(1) { animation-delay: -0.32s; }
.loading-dots span:nth-child(2) { animation-delay: -0.16s; }
.loading-dots span:nth-child(3) { animation-delay: 0s; }

@keyframes bounce {
  0%, 80%, 100% { transform: scale(0); }
  40% { transform: scale(1); }
}

.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--accent-red);
  gap: var(--spacing-sm);
  text-align: center;
}

.error-icon {
  width: 32px;
  height: 32px;
  background: rgba(248, 113, 113, 0.2);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
}

.error-message {
  font-size: 0.75rem;
  max-width: 200px;
}

.content-text {
  color: var(--text-primary);
}

.content-text :deep(p) {
  margin: 0 0 var(--spacing-sm) 0;
}

.content-text :deep(p:last-child) {
  margin-bottom: 0;
}

.content-text :deep(code) {
  background: rgba(0, 0, 0, 0.3);
  padding: 2px 4px;
  border-radius: 3px;
  font-size: 0.75rem;
  font-family: 'Fira Code', monospace;
}

.content-text :deep(pre) {
  background: rgba(0, 0, 0, 0.3);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-sm) 0;
}

.content-text :deep(pre code) {
  background: none;
  padding: 0;
}

.content-text :deep(ul), .content-text :deep(ol) {
  margin: var(--spacing-sm) 0;
  padding-left: var(--spacing-md);
}

.content-text :deep(li) {
  margin-bottom: 4px;
}

.response-footer {
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.2);
  border-top: 1px solid rgba(255, 255, 255, 0.05);
}

.tokens {
  font-size: 0.625rem;
  color: var(--text-muted);
}
</style>
