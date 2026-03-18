<template>
  <div
    class="llm-node"
    :class="{ dragging: isDragging, resizing: isResizing, streaming: isStreaming, selected, error: hasError }"
    :style="nodeStyle"
    @mousedown="handleMouseDown"
  >
    <div
      class="node-header"
      :style="headerStyle"
    >
      <span class="model-badge">{{ modelName }}</span>
      <div class="header-right">
        <span
          v-if="webSearch"
          class="web-search-badge"
          title="Live web search used for this prompt"
        >
          <GIcon
            name="globe"
            :size="12"
          />
          <span>Web</span>
        </span>
        <GIcon
          v-if="isStreaming"
          name="loader"
          :size="14"
          class="streaming-indicator"
          icon-class="spinning"
        />
        <GIcon
          v-else-if="hasError"
          name="alert-circle"
          :size="14"
          class="error-indicator"
        />
        <GIcon
          v-else-if="isCompleted"
          name="check-circle"
          :size="14"
          class="complete-indicator"
        />
      </div>
    </div>
    
    <div
      ref="contentRef"
      class="node-content"
      @wheel.stop
    >
      <div
        v-if="isStreaming"
        class="response-text streaming-text"
      >
        {{ response.content }}
      </div>
      <div
        v-else-if="isCompleted"
        class="response-text"
        v-html="renderedContent"
      />
      <div
        v-else-if="hasError"
        class="error-message"
      >
        <GIcon
          name="alert-triangle"
          :size="16"
          class="error-icon"
        />
        <span>{{ response.error_message || 'Error occurred' }}</span>
      </div>
      <div
        v-else
        class="pending-message"
      >
        <span class="pending-spinner" />
        <span>Waiting...</span>
      </div>
    </div>
    
    <div
      v-if="!isStreaming"
      class="node-footer"
    >
      <button 
        class="branch-btn" 
        :disabled="!isCompleted" 
        title="Branch from this response"
        @click.stop="toggleBranch"
      >
        <GIcon
          name="git-branch"
          :size="14"
        /> Branch
      </button>
      <button
        v-if="isCompleted"
        class="think-harder-btn"
        title="Ask this model to think harder"
        @click.stop="toggleThinkHarder"
      >
        Think Harder
      </button>
      <button 
        class="regenerate-btn" 
        :disabled="!isCompleted"
        title="Regenerate response"
        @click.stop="$emit('regenerate', { tileId, modelId })"
      >
        <GIcon
          name="refresh-cw"
          :size="14"
        />
      </button>
      <button 
        class="select-btn"
        :class="{ active: selected }"
        title="Select for debate"
        @click.stop="$emit('select', { tileId, modelId })"
      >
        <GIcon
          v-if="selected"
          name="check"
          :size="14"
        /><GIcon
          v-else
          name="circle"
          :size="14"
        />
      </button>
      <button
        class="delete-btn"
        title="Remove this response"
        @click.stop="$emit('delete', { tileId, modelId })"
      >
        <GIcon
          name="x"
          :size="14"
        />
      </button>
    </div>
    
    <!-- Connection point (left side - input from prompt) -->
    <div
      class="connection-point in"
      :style="{ borderColor: response.color }"
    />
    <!-- Connection point (right side - output for branching) -->
    <div
      v-if="isCompleted"
      class="connection-point out"
    />
    
    <!-- Follow-up button (+) on right edge -->
    <button
      v-if="isCompleted"
      class="followup-btn"
      title="Quick follow-up with this model"
      @click.stop="toggleFollowup"
    >
      <GIcon
        name="plus"
        :size="16"
      />
    </button>

    <!-- Follow-up input overlay -->
    <div
      v-if="showFollowup"
      class="followup-overlay"
      @click.stop
    >
      <textarea
        ref="followupInputRef"
        v-model="followupPrompt"
        placeholder="Follow up..."
        rows="2"
        class="followup-textarea"
        @keydown.ctrl.enter="submitFollowup"
        @keydown.escape="showFollowup = false"
      />
      <div class="followup-actions">
        <button
          class="followup-cancel"
          @click.stop="showFollowup = false"
        >
          Cancel
        </button>
        <button
          class="followup-submit"
          :disabled="!followupPrompt.trim()"
          @click.stop="submitFollowup"
        >
          Send
        </button>
      </div>
    </div>

    <!-- Branch input overlay -->
    <div
      v-if="showBranch"
      class="branch-overlay"
      @click.stop
    >
      <textarea
        ref="branchInputRef"
        v-model="branchPrompt"
        placeholder="Continue this conversation..."
        rows="2"
        class="branch-textarea"
        @keydown.ctrl.enter="submitBranch"
        @keydown.escape="showBranch = false"
      />
      
      <!-- Model selection for branch -->
      <div class="branch-models">
        <div class="branch-models-header">
          <span class="models-label">Models:</span>
          <button 
            class="models-toggle" 
            :title="branchModels.length + ' model(s) selected'"
            @click.stop="showModelPicker = !showModelPicker"
          >
            {{ branchModels.length }} selected ▾
          </button>
        </div>
        
        <!-- Selected model tags -->
        <div
          v-if="branchModels.length > 0"
          class="branch-model-tags"
        >
          <span 
            v-for="mId in branchModels" 
            :key="mId" 
            class="branch-model-tag"
          >
            {{ getShortModelName(mId) }}
            <button
              class="tag-remove-btn"
              @click.stop="removeModel(mId)"
            ><GIcon
              name="x"
              :size="10"
            /></button>
          </span>
        </div>
        
        <!-- Model picker dropdown -->
        <div
          v-if="showModelPicker"
          class="model-picker-dropdown"
        >
          <input 
            v-model="modelSearchQuery" 
            type="text" 
            placeholder="Search models..." 
            class="model-search-input"
            @click.stop
          >
          <div class="model-picker-list">
            <label 
              v-for="model in filteredModels" 
              :key="model.id" 
              class="model-picker-item"
              :class="{ selected: branchModels.includes(model.id) }"
            >
              <input 
                v-model="branchModels" 
                type="checkbox" 
                :value="model.id"
                @click.stop
              >
              <span class="model-picker-name">{{ model.name }}</span>
            </label>
          </div>
          <div class="model-picker-actions">
            <button
              class="picker-btn"
              @click.stop="branchModels = [modelId]"
            >
              Reset
            </button>
            <button
              class="picker-btn picker-btn-done"
              @click.stop="showModelPicker = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
      
      <div class="branch-options">
        <select
          v-model="branchContextMode"
          class="context-select"
        >
          <option value="full_history">
            Full History
          </option>
          <option value="compact">
            Compact
          </option>
          <option value="knowledge_search">
            Knowledge Search
          </option>
        </select>
      </div>
      <div class="branch-actions">
        <button
          class="branch-cancel"
          @click.stop="showBranch = false"
        >
          Cancel
        </button>
        <button
          class="branch-submit"
          :disabled="!branchPrompt.trim() || branchModels.length === 0"
          @click.stop="submitBranch"
        >
          Send ({{ branchModels.length }})
        </button>
      </div>
    </div>

    <div
      v-if="showThinkHarder"
      class="think-harder-overlay"
      @click.stop
    >
      <div class="think-harder-header">
        <span class="think-harder-title">Think Harder</span>
        <span class="think-harder-model">{{ modelName }}</span>
      </div>
      <p class="think-harder-copy">
        This creates a deeper second-pass branch and asks the model to verify more information before answering.
      </p>
      <label class="think-harder-checkbox">
        <input
          v-model="thinkHarderWebSearch"
          type="checkbox"
        >
        <span>Web search more sources</span>
      </label>
      <p class="think-harder-hint">
        Enabled by default. This pass searches more results than a normal web-search prompt when supported.
      </p>
      <div class="think-harder-actions">
        <button
          class="think-harder-cancel"
          @click.stop="showThinkHarder = false"
        >
          Cancel
        </button>
        <button
          class="think-harder-submit"
          @click.stop="submitThinkHarder"
        >
          Think Harder
        </button>
      </div>
    </div>

    <div
      class="resize-handle"
      @mousedown.stop="startResize"
    />
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount, watch, nextTick } from 'vue'
import { marked } from 'marked'
import GIcon from '@/components/ui/GIcon.vue'

const MIN_WIDTH = 280
const MIN_HEIGHT = 200

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
  webSearch: {
    type: Boolean,
    default: false
  },
  isStreaming: {
    type: Boolean,
    default: false
  },
  selected: {
    type: Boolean,
    default: false
  },
  availableModels: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['drag', 'branch', 'select', 'delete', 'regenerate', 'follow-up', 'think-harder'])

// Refs
const contentRef = ref(null)
const branchInputRef = ref(null)
const followupInputRef = ref(null)

// Dragging state
const isDragging = ref(false)
const isResizing = ref(false)
const dragStart = ref({ x: 0, y: 0, nodeX: 0, nodeY: 0 })

// Follow-up state
const showFollowup = ref(false)
const followupPrompt = ref('')
const showThinkHarder = ref(false)
const thinkHarderWebSearch = ref(true)

// Branch state
const showBranch = ref(false)
const branchPrompt = ref('')
const branchContextMode = ref('full_history')
const branchModels = ref([])
const showModelPicker = ref(false)
const modelSearchQuery = ref('')


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
  if (!props.response.content || props.isStreaming) return ''
  marked.setOptions({ breaks: true, gfm: true })
  return marked(props.response.content)
})

// Watch for branch input focus
watch(showBranch, async (isShowing) => {
  if (isShowing) {
    // Initialize with current model
    branchModels.value = [props.modelId]
    modelSearchQuery.value = ''
    showModelPicker.value = false
    await nextTick()
    branchInputRef.value?.focus()
  }
})

// Computed for filtered models
const filteredModels = computed(() => {
  if (!modelSearchQuery.value) return props.availableModels
  const query = modelSearchQuery.value.toLowerCase()
  return props.availableModels.filter(m =>
    m.name.toLowerCase().includes(query) ||
    m.id.toLowerCase().includes(query)
  )
})


function getShortModelName(modelId) {
  const model = props.availableModels.find(m => m.id === modelId)
  if (model) {
    const parts = model.name.split(':')
    return parts.length > 1 ? parts[1].trim() : model.name
  }
  const parts = modelId.split('/')
  return parts.length > 1 ? parts[1] : modelId
}

function removeModel(modelId) {
  branchModels.value = branchModels.value.filter(id => id !== modelId)
}


// Methods
function handleMouseDown(e) {
  // Ignore clicks on interactive elements and scrollable content
  if (e.target.closest('.node-footer') ||
      e.target.closest('.branch-overlay') ||
      e.target.closest('.followup-overlay') ||
      e.target.closest('.think-harder-overlay') ||
      e.target.closest('.node-content') ||
      e.target.closest('.resize-handle') ||
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

function startResize(e) {
  isResizing.value = true
  const startX = e.clientX
  const startY = e.clientY
  const startWidth = props.response.position.width || MIN_WIDTH
  const startHeight = props.response.position.height || MIN_HEIGHT

  e.preventDefault()
  e.stopPropagation()

  function onResize(moveEvent) {
    const newWidth = Math.max(MIN_WIDTH, startWidth + (moveEvent.clientX - startX))
    const newHeight = Math.max(MIN_HEIGHT, startHeight + (moveEvent.clientY - startY))

    emit('drag', props.tileId, props.modelId, {
      x: props.response.position.x,
      y: props.response.position.y,
      width: newWidth,
      height: newHeight
    })
  }

  function stopResize() {
    isResizing.value = false
    document.removeEventListener('mousemove', onResize)
    document.removeEventListener('mouseup', stopResize)
  }

  document.addEventListener('mousemove', onResize)
  document.addEventListener('mouseup', stopResize)
}

function toggleBranch() {
  showThinkHarder.value = false
  showBranch.value = !showBranch.value
  if (!showBranch.value) {
    branchPrompt.value = ''
  }
}

function submitBranch() {
  if (!branchPrompt.value.trim() || branchModels.value.length === 0) return

  emit('branch', props.tileId, props.modelId, branchPrompt.value.trim(), branchContextMode.value, branchModels.value)
  showBranch.value = false
  branchPrompt.value = ''
  branchModels.value = []
  showModelPicker.value = false
}

// Follow-up methods
function toggleFollowup() {
  showThinkHarder.value = false
  showFollowup.value = !showFollowup.value
  if (!showFollowup.value) {
    followupPrompt.value = ''
  }
}

function toggleThinkHarder() {
  showBranch.value = false
  showFollowup.value = false
  showThinkHarder.value = !showThinkHarder.value
  if (showThinkHarder.value) {
    thinkHarderWebSearch.value = true
  }
}

function submitThinkHarder() {
  emit('think-harder', {
    tileId: props.tileId,
    modelId: props.modelId,
    webSearch: thinkHarderWebSearch.value
  })
  showThinkHarder.value = false
}

function submitFollowup() {
  if (!followupPrompt.value.trim()) return

  emit('follow-up', {
    tileId: props.tileId,
    modelId: props.modelId,
    prompt: followupPrompt.value.trim()
  })
  showFollowup.value = false
  followupPrompt.value = ''
}

// Watch for follow-up input focus
watch(showFollowup, async (isShowing) => {
  if (isShowing) {
    await nextTick()
    followupInputRef.value?.focus()
  }
})

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
  box-shadow: var(--shadow-md);
  display: flex;
  flex-direction: column;
  overflow: visible;
  transition: left 0.5s ease-out, top 0.5s ease-out, box-shadow 0.15s, border-color 0.15s, transform 0.1s;
  user-select: none;
}

.llm-node:hover {
  box-shadow: var(--shadow-lg);
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

.llm-node.resizing {
  user-select: none;
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

.web-search-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--accent-cyan) 35%, transparent);
  background: color-mix(in srgb, var(--accent-cyan) 14%, transparent);
  color: var(--accent-cyan);
  font-size: 0.625rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  line-height: 1;
  text-transform: uppercase;
}

.web-search-icon {
  font-size: 0.6875rem;
  line-height: 1;
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

.streaming-text {
  white-space: pre-wrap;
  word-break: break-word;
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
  border: 2px solid var(--border-subtle);
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
  border-top: 1px solid var(--border-subtle);
  background: transparent;
}

.branch-btn, .think-harder-btn, .select-btn, .delete-btn {
  padding: 4px 8px;
  border: 1px solid var(--border-subtle);
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

.think-harder-btn {
  white-space: nowrap;
}

.think-harder-btn:hover {
  border-color: var(--accent-cyan);
  color: var(--accent-cyan);
  background: color-mix(in srgb, var(--accent-cyan) 10%, transparent);
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

/* Regenerate button */
.regenerate-btn {
  padding: 4px 8px;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
}

.regenerate-btn:hover:not(:disabled) {
  border-color: var(--accent-blue);
  color: var(--accent-blue);
  background: color-mix(in srgb, var(--accent-blue) 10%, transparent);
}

.regenerate-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Follow-up button (+) */
.followup-btn {
  position: absolute;
  right: -14px;
  top: 35%;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: var(--bg-secondary);
  border: 2px solid var(--accent-blue);
  color: var(--accent-blue);
  font-size: 1.25rem;
  font-weight: bold;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s;
  opacity: 0;
  transform: scale(0.8);
  z-index: 5;
}

.llm-node:hover .followup-btn {
  opacity: 1;
  transform: scale(1);
}

.followup-btn:hover {
  background: var(--accent-blue);
  color: white;
  transform: scale(1.1);
}

/* Follow-up overlay */
.followup-overlay {
  position: absolute;
  bottom: 100%;
  left: 0;
  right: 0;
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  backdrop-filter: blur(12px);
  box-shadow: var(--shadow-lg);
  z-index: 10;
}

.followup-textarea {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8125rem;
  font-family: inherit;
  resize: none;
}

.followup-textarea:focus {
  outline: none;
  border-color: var(--accent-blue);
}

.followup-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-xs);
}

.followup-cancel, .followup-submit {
  padding: 4px 12px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.15s;
}

.followup-cancel {
  background: transparent;
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
}

.followup-cancel:hover {
  border-color: var(--text-muted);
}

.followup-submit {
  background: var(--accent-blue);
  border: none;
  color: white;
}

.followup-submit:hover:not(:disabled) {
  filter: brightness(1.15);
}

.followup-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.think-harder-overlay {
  position: absolute;
  right: -8px;
  bottom: calc(100% + 12px);
  width: 280px;
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  backdrop-filter: blur(12px);
  box-shadow: var(--shadow-lg);
  z-index: 50;
}

.think-harder-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
}

.think-harder-title {
  font-size: 0.8125rem;
  font-weight: 700;
  color: var(--text-primary);
}

.think-harder-model {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.75rem;
  color: var(--accent-cyan);
}

.think-harder-copy,
.think-harder-hint {
  margin: 0;
  font-size: 0.75rem;
  line-height: 1.5;
  color: var(--text-secondary);
}

.think-harder-copy {
  margin-bottom: var(--spacing-sm);
}

.think-harder-checkbox {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  font-size: 0.8125rem;
  color: var(--text-primary);
  cursor: pointer;
}

.think-harder-checkbox input[type="checkbox"] {
  margin: 0;
  accent-color: var(--accent-cyan);
}

.think-harder-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  margin-top: var(--spacing-md);
}

.think-harder-cancel,
.think-harder-submit {
  padding: 4px 12px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.15s;
}

.think-harder-cancel {
  background: transparent;
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
}

.think-harder-cancel:hover {
  border-color: var(--text-muted);
}

.think-harder-submit {
  background: color-mix(in srgb, var(--accent-cyan) 20%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent-cyan) 35%, transparent);
  color: var(--accent-cyan);
}

.think-harder-submit:hover {
  background: color-mix(in srgb, var(--accent-cyan) 28%, transparent);
}

/* Branch overlay */
.branch-overlay {
  position: absolute;
  bottom: 100%;
  left: 0;
  width: max(100%, 340px);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  backdrop-filter: blur(12px);
  box-shadow: var(--shadow-lg);
  z-index: 10;
}

.branch-textarea {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
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
  border: 1px solid var(--border-subtle);
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
  border: 1px solid var(--border-subtle);
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

/* Branch Model Picker Styles */
.branch-models {
  margin: var(--spacing-xs) 0;
  position: relative;
}

.branch-models-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 4px;
}

.models-label {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.models-toggle {
  padding: 2px 8px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.15s;
}

.models-toggle:hover {
  border-color: var(--accent-primary);
  color: var(--text-primary);
}

.branch-model-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: var(--spacing-xs);
}

.branch-model-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px;
  background: rgba(124, 92, 255, 0.15);
  border: 1px solid rgba(124, 92, 255, 0.3);
  border-radius: var(--radius-sm);
  font-size: 0.6875rem;
  color: var(--text-primary);
}

.tag-remove-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 0.875rem;
  line-height: 1;
}

.tag-remove-btn:hover {
  color: var(--accent-primary);
}

.model-picker-dropdown {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  background: var(--bg-secondary);
  border: 1px solid var(--accent-primary);
  border-radius: var(--radius-sm);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  z-index: 100;
  max-height: 250px;
  display: flex;
  flex-direction: column;
}

.model-search-input {
  width: 100%;
  padding: 8px;
  background: var(--bg-tertiary);
  border: none;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-primary);
  font-size: 0.75rem;
}

.model-search-input:focus {
  outline: none;
  background: var(--bg-primary);
}

.model-picker-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px;
}

.model-picker-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.1s;
}

.model-picker-item:hover {
  background: var(--bg-tertiary);
}

.model-picker-item.selected {
  background: rgba(124, 92, 255, 0.15);
}

.model-picker-item input[type="checkbox"] {
  accent-color: var(--accent-primary);
  width: 14px;
  height: 14px;
}

.model-picker-name {
  font-size: 0.75rem;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-picker-actions {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  padding: 6px 8px;
  border-top: 1px solid var(--border-subtle);
}

.picker-btn {
  padding: 4px 10px;
  border-radius: var(--radius-sm);
  font-size: 0.6875rem;
  cursor: pointer;
  background: transparent;
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
}

.picker-btn:hover {
  border-color: var(--text-muted);
}

.picker-btn-done {
  background: var(--accent-primary);
  border-color: var(--accent-primary);
  color: white;
}

.resize-handle {
  position: absolute;
  right: 6px;
  bottom: 6px;
  width: 14px;
  height: 14px;
  border-right: 2px solid color-mix(in srgb, var(--node-color, #7c5cff) 75%, white);
  border-bottom: 2px solid color-mix(in srgb, var(--node-color, #7c5cff) 75%, white);
  opacity: 0.7;
  cursor: se-resize;
}

.resize-handle:hover {
  opacity: 1;
}

.picker-btn-done:hover {
  background: #6b4fd9;
}
</style>
