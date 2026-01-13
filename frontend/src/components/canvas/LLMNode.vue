<template>
  <div
    class="llm-node"
    :class="{ dragging: isDragging, streaming: isStreaming, selected, error: hasError }"
    :style="nodeStyle"
    @mousedown="handleMouseDown"
  >
    <div class="node-header" :style="headerStyle">
      <span class="model-badge">{{ modelName }}</span>
      <div class="header-right">
        <span v-if="isStreaming" class="streaming-indicator">●</span>
        <span v-else-if="hasError" class="error-indicator">!</span>
        <span v-else-if="isCompleted" class="complete-indicator">✓</span>
      </div>
    </div>
    
    <div class="node-content" ref="contentRef" @wheel.stop>
      <div v-if="isStreaming || isCompleted" class="response-text" v-html="renderedContent"></div>
      <div v-else-if="hasError" class="error-message">
        <span class="error-icon">⚠️</span>
        <span>{{ response.error_message || 'Error occurred' }}</span>
      </div>
      <div v-else class="pending-message">
        <span class="pending-spinner"></span>
        <span>Waiting...</span>
      </div>
    </div>
    
    <div class="node-footer" v-if="!isStreaming">
      <button 
        class="branch-btn" 
        @click.stop="toggleBranch" 
        :disabled="!isCompleted"
        title="Branch from this response"
      >
        ⑂ Branch
      </button>
      <button 
        class="select-btn"
        :class="{ active: selected }"
        @click.stop="$emit('select', { tileId, modelId })"
        title="Select for debate"
      >
        {{ selected ? '✓' : '○' }}
      </button>
      <button
        class="delete-btn"
        @click.stop="$emit('delete', { tileId, modelId })"
        title="Remove this response"
      >
        ×
      </button>
    </div>
    
    <!-- Connection point (left side - input from prompt) -->
    <div class="connection-point in" :style="{ borderColor: response.color }"></div>
    <!-- Connection point (right side - output for branching) -->
    <div class="connection-point out" v-if="isCompleted"></div>
    
    <!-- Branch input overlay -->
    <div v-if="showBranch" class="branch-overlay" @click.stop>
      <textarea
        ref="branchInputRef"
        v-model="branchPrompt"
        placeholder="Continue this conversation..."
        rows="2"
        class="branch-textarea"
        @keydown.ctrl.enter="submitBranch"
        @keydown.escape="showBranch = false"
      ></textarea>
      <div class="branch-options">
        <select v-model="branchContextMode" class="context-select">
          <option value="full_history">Full History</option>
          <option value="compact">Compact</option>
          <option value="semantic">Semantic</option>
        </select>
      </div>
      <div class="branch-actions">
        <button class="branch-cancel" @click.stop="showBranch = false">Cancel</button>
        <button class="branch-submit" @click.stop="submitBranch" :disabled="!branchPrompt.trim()">
          Send
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount, watch, nextTick } from 'vue'
import { marked } from 'marked'

const props = defineProps({
  tileId: {
    type: String,
    required: true
  },
  modelId: {
    type: String,
    required: true
  },
  response: {
    type: Object,
    required: true
  },
  isStreaming: {
    type: Boolean,
    default: false
  },
  selected: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['drag', 'branch', 'select', 'delete'])

// Refs
const contentRef = ref(null)
const branchInputRef = ref(null)

// Dragging state
const isDragging = ref(false)
const dragStart = ref({ x: 0, y: 0, nodeX: 0, nodeY: 0 })

// Branch state
const showBranch = ref(false)
const branchPrompt = ref('')
const branchContextMode = ref('full_history')

// Computed
const modelName = computed(() => {
  return props.response.model_name || props.modelId.split('/').pop() || props.modelId
})

const isCompleted = computed(() => props.response.status === 'completed')
const hasError = computed(() => props.response.status === 'error')

const nodeStyle = computed(() => ({
  left: `${props.response.position.x}px`,
  top: `${props.response.position.y}px`,
  width: `${props.response.position.width || 280}px`,
  minHeight: `${props.response.position.height || 200}px`,
  '--node-color': props.response.color || '#7c5cff'
}))

const headerStyle = computed(() => ({
  background: `linear-gradient(135deg, ${props.response.color || '#7c5cff'}22 0%, ${props.response.color || '#7c5cff'}11 100%)`,
  borderBottomColor: `${props.response.color || '#7c5cff'}33`
}))

const renderedContent = computed(() => {
  if (!props.response.content) return ''
  marked.setOptions({ breaks: true, gfm: true })
  // Truncate for display if very long
  const content = props.response.content.length > 3000 
    ? props.response.content.slice(0, 3000) + '\n\n*[Content truncated...]*'
    : props.response.content
  return marked(content)
})

// Watch for branch input focus
watch(showBranch, async (isShowing) => {
  if (isShowing) {
    await nextTick()
    branchInputRef.value?.focus()
  }
})

// Methods
function handleMouseDown(e) {
  // Ignore clicks on interactive elements and scrollable content
  if (e.target.closest('.node-footer') || 
      e.target.closest('.branch-overlay') ||
      e.target.closest('.node-content') ||
      e.target.closest('button')) {
    return
  }
  if (e.button !== 0) return

  startDrag(e)
}

function startDrag(e) {
  isDragging.value = true
  dragStart.value = {
    x: e.clientX,
    y: e.clientY,
    nodeX: props.response.position.x,
    nodeY: props.response.position.y
  }

  e.preventDefault()
  e.stopPropagation()

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
  document.body.classList.add('tile-dragging')
}

function onDrag(e) {
  if (!isDragging.value) return
  
  const deltaX = e.clientX - dragStart.value.x
  const deltaY = e.clientY - dragStart.value.y
  
  emit('drag', props.tileId, props.modelId, {
    x: dragStart.value.nodeX + deltaX,
    y: dragStart.value.nodeY + deltaY,
    width: props.response.position.width,
    height: props.response.position.height
  })
}

function stopDrag() {
  isDragging.value = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.body.classList.remove('tile-dragging')
}

function toggleBranch() {
  showBranch.value = !showBranch.value
  if (!showBranch.value) {
    branchPrompt.value = ''
  }
}

function submitBranch() {
  if (!branchPrompt.value.trim()) return
  
  emit('branch', props.tileId, props.modelId, branchPrompt.value.trim(), branchContextMode.value)
  showBranch.value = false
  branchPrompt.value = ''
}

// Cleanup on unmount
onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.body.classList.remove('tile-dragging')
})
</script>

<style scoped>
.llm-node {
  position: absolute;
  background: var(--bg-secondary);
  border: 2px solid var(--node-color, #7c5cff);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: left 0.5s ease-out, top 0.5s ease-out, box-shadow 0.15s, border-color 0.15s, transform 0.1s;
  user-select: none;
}

.llm-node:hover {
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.3);
}

.llm-node.selected {
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-cyan) 30%, transparent), 0 6px 24px color-mix(in srgb, var(--bg-primary) 30%, transparent);
}

.llm-node.dragging {
  cursor: grabbing;
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.4);
  z-index: 1000;
  transform: scale(1.02);
}

.llm-node.streaming {
  border-color: var(--accent-blue);
  animation: streamingPulse 1.5s ease-in-out infinite;
}

.llm-node.error {
  border-color: var(--accent-red);
}

@keyframes streamingPulse {
  0%, 100% { box-shadow: 0 4px 16px color-mix(in srgb, var(--accent-cyan) 20%, transparent); }
  50% { box-shadow: 0 4px 24px color-mix(in srgb, var(--accent-cyan) 40%, transparent); }
}

.node-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-xs) var(--spacing-sm);
  border-bottom: 1px solid;
  cursor: grab;
}

.node-header:active {
  cursor: grabbing;
}

.model-badge {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-primary);
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 4px;
}

.streaming-indicator {
  color: var(--accent-blue);
  animation: blink 1s ease-in-out infinite;
}

.error-indicator {
  color: var(--accent-red);
  font-weight: bold;
}

.complete-indicator {
  color: var(--accent-green);
}

@keyframes blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

.node-content {
  flex: 1;
  padding: var(--spacing-sm);
  overflow-y: auto;
  max-height: 300px;
  font-size: 0.8125rem;
  line-height: 1.5;
  color: var(--text-primary);
}

.node-content :deep(p) {
  margin: 0 0 var(--spacing-xs) 0;
  color: var(--text-primary);
}

.node-content :deep(p:last-child) {
  margin-bottom: 0;
}

.node-content :deep(code) {
  background: var(--bg-tertiary);
  padding: 2px 4px;
  border-radius: 3px;
  font-size: 0.75rem;
  font-family: 'Fira Code', monospace;
}

.node-content :deep(pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-xs) 0;
}

.node-content :deep(pre code) {
  background: none;
  padding: 0;
}

.node-content :deep(ul), .node-content :deep(ol) {
  margin: var(--spacing-xs) 0;
  padding-left: var(--spacing-md);
}

.pending-message, .error-message {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-md);
  color: var(--text-muted);
}

.pending-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.error-message {
  color: var(--accent-red);
}

.node-footer {
  display: flex;
  gap: 4px;
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.1);
  border-top: 1px solid var(--bg-tertiary);
}

.branch-btn, .select-btn, .delete-btn {
  padding: 4px 8px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.6875rem;
  cursor: pointer;
  transition: all 0.15s;
}

.branch-btn {
  flex: 1;
}

.branch-btn:hover:not(:disabled) {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
}

.branch-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.select-btn {
  width: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.select-btn:hover {
  border-color: var(--accent-cyan);
  color: var(--accent-cyan);
}

.select-btn.active {
  background: var(--accent-cyan);
  border-color: var(--accent-cyan);
  color: white;
}

.delete-btn {
  width: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1rem;
}

.delete-btn:hover {
  border-color: var(--accent-red);
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 10%, transparent);
}

/* Connection points */
.connection-point {
  position: absolute;
  width: 10px;
  height: 10px;
  background: var(--bg-primary);
  border: 2px solid var(--node-color, #7c5cff);
  border-radius: 50%;
}

.connection-point.in {
  left: -6px;
  top: 50%;
  transform: translateY(-50%);
}

.connection-point.out {
  right: -6px;
  top: 50%;
  transform: translateY(-50%);
  background: var(--node-color, #7c5cff);
}

/* Branch overlay */
.branch-overlay {
  position: absolute;
  bottom: 100%;
  left: 0;
  right: 0;
  background: var(--bg-secondary);
  border: 1px solid var(--accent-primary);
  border-radius: var(--radius-md);
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  z-index: 10;
}

.branch-textarea {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8125rem;
  font-family: inherit;
  resize: none;
}

.branch-textarea:focus {
  outline: none;
  border-color: var(--accent-primary);
}

.branch-options {
  margin: var(--spacing-xs) 0;
}

.context-select {
  width: 100%;
  padding: 4px 8px;
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.75rem;
}

.branch-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-xs);
}

.branch-cancel, .branch-submit {
  padding: 4px 12px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.15s;
}

.branch-cancel {
  background: transparent;
  border: 1px solid var(--bg-tertiary);
  color: var(--text-muted);
}

.branch-cancel:hover {
  border-color: var(--text-muted);
}

.branch-submit {
  background: var(--accent-primary);
  border: none;
  color: white;
}

.branch-submit:hover:not(:disabled) {
  background: #6b4fd9;
}

.branch-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
