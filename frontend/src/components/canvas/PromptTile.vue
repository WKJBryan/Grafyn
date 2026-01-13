<template>
  <div
    class="prompt-tile"
    :class="{ selected, streaming: isStreaming, dragging: isDragging }"
    :style="tileStyle"
    @mousedown="handleMouseDown"
  >
    <div class="tile-header">
      <div class="prompt-preview">
        <span class="prompt-icon">?</span>
        <span class="prompt-text">{{ truncatedPrompt }}</span>
      </div>
      <div class="tile-actions">
        <button
          class="select-btn"
          :class="{ active: selected }"
          @click.stop="$emit('select', tile.id)"
          title="Select for debate"
        >
          <span v-if="selected">&#10003;</span>
          <span v-else>&#9675;</span>
        </button>
        <button
          class="delete-btn"
          @click.stop="$emit('delete', tile.id)"
          title="Delete tile"
        >
          &#10005;
        </button>
      </div>
    </div>

    <div class="responses-grid" :style="gridStyle">
      <ModelResponseCard
        v-for="(response, modelId) in tile.responses"
        :key="modelId"
        :response="response"
        :is-streaming="streamingModels.has(modelId)"
      />
    </div>

    <!-- Branch Input Section -->
    <div class="branch-section" v-if="!isStreaming">
      <div v-if="!showBranchInput" class="branch-trigger" @click.stop="showBranchInput = true">
        <span class="branch-icon">⑂</span>
        <span>Branch from here...</span>
      </div>
      <div v-else class="branch-input-container" @click.stop>
        <textarea
          ref="branchInputRef"
          v-model="branchPrompt"
          placeholder="Continue this conversation..."
          rows="2"
          class="branch-textarea"
          @keydown.ctrl.enter="submitBranch"
          @keydown.escape="closeBranchInput"
        ></textarea>
        <div class="branch-options">
          <label class="context-label">Context:</label>
          <select v-model="branchContextMode" class="context-select">
            <option value="full_history">Full History</option>
            <option value="compact">Compact</option>
            <option value="semantic">Semantic</option>
          </select>
        </div>
        <ContextBudgetDisplay
          :current-tokens="currentTokens"
          :max-tokens="maxContextTokens"
          :compact="true"
        />
        <div class="branch-actions">
          <button class="branch-cancel" @click.stop="closeBranchInput">Cancel</button>
          <button class="branch-submit" @click.stop="submitBranch" :disabled="!branchPrompt.trim()">
            Send to {{ modelCount }} model{{ modelCount !== 1 ? 's' : '' }}
          </button>
        </div>
      </div>
    </div>

    <div class="tile-footer">
      <span class="model-count">{{ modelCount }} models</span>
      <span class="timestamp">{{ formatTime(tile.created_at) }}</span>
    </div>

    <!-- Resize handle -->
    <div class="resize-handle" @mousedown.stop="startResize"></div>
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount, nextTick } from 'vue'
import ModelResponseCard from './ModelResponseCard.vue'

const props = defineProps({
  tile: {
    type: Object,
    required: true
  },
  selected: {
    type: Boolean,
    default: false
  },
  streamingModels: {
    type: Set,
    default: () => new Set()
  }
})

const emit = defineEmits(['drag', 'select', 'branch', 'delete'])

// Dragging state
const isDragging = ref(false)
const isResizing = ref(false)
const dragStart = ref({ x: 0, y: 0, tileX: 0, tileY: 0 })

// Branch input state
const showBranchInput = ref(false)
const branchPrompt = ref('')
const branchInputRef = ref(null)
const branchContextMode = ref('full_history')

// Computed
const isStreaming = computed(() => {
  return Object.values(props.tile.responses).some(r => r.status === 'streaming')
})

const modelCount = computed(() => Object.keys(props.tile.responses).length)

const truncatedPrompt = computed(() => {
  const prompt = props.tile.prompt
  return prompt.length > 80 ? prompt.slice(0, 80) + '...' : prompt
})

const tileStyle = computed(() => ({
  left: `${props.tile.position.x}px`,
  top: `${props.tile.position.y}px`,
  width: `${props.tile.position.width}px`,
  minHeight: `${props.tile.position.height}px`
}))

const gridStyle = computed(() => {
  const count = modelCount.value
  if (count <= 2) return { gridTemplateColumns: `repeat(${count}, 1fr)` }
  if (count <= 4) return { gridTemplateColumns: 'repeat(2, 1fr)' }
  return { gridTemplateColumns: 'repeat(3, 1fr)' }
})

// Methods
function formatTime(dateStr) {
  if (!dateStr) return ''
  const date = new Date(dateStr)
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

function handleMouseDown(e) {
  // Ignore clicks on interactive elements and scrollable content
  if (e.target.closest('.tile-actions') || 
      e.target.closest('.resize-handle') || 
      e.target.closest('.branch-section') ||
      e.target.closest('.select-btn') ||
      e.target.closest('.responses-grid')) {
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
    tileX: props.tile.position.x,
    tileY: props.tile.position.y
  }

  e.preventDefault()
  e.stopPropagation()

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
}

function onDrag(e) {
  if (!isDragging.value) return
  
  const deltaX = e.clientX - dragStart.value.x
  const deltaY = e.clientY - dragStart.value.y
  
  emit('drag', props.tile.id, {
    x: dragStart.value.tileX + deltaX,
    y: dragStart.value.tileY + deltaY,
    width: props.tile.position.width,
    height: props.tile.position.height
  })
}

function stopDrag() {
  isDragging.value = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
}

function startResize(e) {
  isResizing.value = true
  const startX = e.clientX
  const startY = e.clientY
  const startWidth = props.tile.position.width
  const startHeight = props.tile.position.height

  function onResize(e) {
    const newWidth = Math.max(300, startWidth + (e.clientX - startX))
    const newHeight = Math.max(200, startHeight + (e.clientY - startY))

    emit('drag', props.tile.id, {
      x: props.tile.position.x,
      y: props.tile.position.y,
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

// Branch methods
function closeBranchInput() {
  showBranchInput.value = false
  branchPrompt.value = ''
}

function submitBranch() {
  if (!branchPrompt.value.trim()) return

  // Get the first model from the tile (or all models)
  const models = Object.keys(props.tile.responses)
  const firstModel = models[0] || null

  emit('branch', props.tile.id, firstModel, branchPrompt.value.trim(), models, branchContextMode.value)
  closeBranchInput()
}

// Focus input when shown
nextTick(() => {
  if (showBranchInput.value && branchInputRef.value) {
    branchInputRef.value.focus()
  }
})

// Cleanup on unmount
onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
})
</script>

<style scoped>
.prompt-tile {
  position: absolute;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: border-color 0.15s, box-shadow 0.15s;
  user-select: none;
}

.prompt-tile.dragging {
  cursor: grabbing;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  z-index: 1000;
}

.prompt-tile:hover {
  border-color: var(--accent-primary);
}

.prompt-tile.selected {
  border-color: var(--accent-primary);
  box-shadow: 0 0 0 2px rgba(124, 92, 255, 0.3), 0 4px 12px rgba(0, 0, 0, 0.2);
}

.prompt-tile.streaming {
  border-color: var(--accent-blue);
}

.tile-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-primary);
  border-bottom: 1px solid var(--bg-tertiary);
  cursor: grab;
}

.tile-header:active {
  cursor: grabbing;
}

.prompt-preview {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  flex: 1;
  min-width: 0;
}

.prompt-icon {
  width: 24px;
  height: 24px;
  background: var(--accent-primary);
  color: white;
  border-radius: var(--radius-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 600;
  flex-shrink: 0;
}

.prompt-text {
  font-size: 0.875rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tile-actions {
  display: flex;
  gap: var(--spacing-xs);
}

.select-btn {
  width: 28px;
  height: 28px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.select-btn:hover {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
}

.select-btn.active {
  background: var(--accent-primary);
  border-color: var(--accent-primary);
  color: white;
}

.delete-btn {
  width: 28px;
  height: 28px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
  font-size: 0.875rem;
}

.delete-btn:hover {
  border-color: var(--accent-red, #f87171);
  color: var(--accent-red, #f87171);
  background: rgba(248, 113, 113, 0.1);
}

.responses-grid {
  display: grid;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

/* Branch Section */
.branch-section {
  padding: var(--spacing-sm);
  border-top: 1px solid var(--bg-tertiary);
  background: var(--bg-primary);
}

.branch-trigger {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  border: 1px dashed var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
  font-size: 0.8125rem;
}

.branch-trigger:hover {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
  background: rgba(124, 92, 255, 0.05);
}

.branch-icon {
  font-size: 1rem;
}

.branch-input-container {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
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
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  margin-bottom: var(--spacing-xs);
}

.context-label {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.context-select {
  flex: 1;
  padding: 4px 8px;
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.75rem;
  cursor: pointer;
}

.context-select:focus {
  outline: none;
  border-color: var(--accent-primary);
}

.branch-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-xs);
}

.branch-cancel,
.branch-submit {
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
  background: var(--accent-primary-hover, #6b4fd9);
}

.branch-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.tile-footer {
  display: flex;
  justify-content: space-between;
  padding: var(--spacing-xs) var(--spacing-md);
  background: var(--bg-primary);
  border-top: 1px solid var(--bg-tertiary);
  font-size: 0.75rem;
  color: var(--text-muted);
}

.resize-handle {
  position: absolute;
  bottom: 0;
  right: 0;
  width: 16px;
  height: 16px;
  cursor: se-resize;
  background: linear-gradient(135deg, transparent 50%, var(--bg-tertiary) 50%);
  border-radius: 0 0 var(--radius-md) 0;
}

.resize-handle:hover {
  background: linear-gradient(135deg, transparent 50%, var(--accent-primary) 50%);
}
</style>
