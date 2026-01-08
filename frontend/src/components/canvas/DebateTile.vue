<template>
  <div
    class="debate-tile"
    :class="{ active: debate.status === 'active', dragging: isDragging, expanded: isExpanded }"
    :style="tileStyle"
    @mousedown="handleMouseDown"
  >
    <div class="tile-header" @mousedown="handleMouseDown">
      <div class="debate-info">
        <span class="debate-icon">&#9876;</span>
        <span class="debate-title">Debate</span>
        <span class="debate-mode">{{ debate.debate_mode }}</span>
      </div>
      <div class="tile-actions">
        <span class="status-badge" :class="debate.status">
          {{ debate.status }}
        </span>
        <button
          class="delete-btn"
          @click.stop="$emit('delete', debate.id)"
          title="Delete debate"
        >
          &#10005;
        </button>
      </div>
    </div>

    <div class="debate-models">
      <span
        v-for="model in debate.participating_models"
        :key="model"
        class="model-badge"
      >
        {{ getModelName(model) }}
      </span>
    </div>

    <!-- Compact Summary View -->
    <div v-if="!isExpanded" class="debate-summary">
      <div class="summary-stats">
        <span class="stat">{{ debate.rounds.length }} round{{ debate.rounds.length !== 1 ? 's' : '' }}</span>
        <span class="stat-divider">•</span>
        <span class="stat">{{ debate.participating_models.length }} models</span>
      </div>
      
      <!-- Show conclusion from last round if available -->
      <div v-if="lastRoundSummary" class="conclusion-preview">
        <div class="conclusion-label">Latest Response:</div>
        <div class="conclusion-text" v-html="lastRoundSummary"></div>
      </div>
      
      <button class="expand-btn" @click.stop="isExpanded = true" v-if="debate.rounds.length > 0">
        <span class="expand-icon">&#9660;</span>
        View Full Debate
      </button>
    </div>

    <!-- Expanded Full Debate View -->
    <div v-else class="debate-rounds">
      <button class="collapse-btn" @click.stop="isExpanded = false">
        <span class="collapse-icon">&#9650;</span>
        Collapse
      </button>
      
      <div
        v-for="(round, index) in debate.rounds"
        :key="index"
        class="debate-round"
      >
        <div class="round-header">
          <span>Round {{ index + 1 }}</span>
        </div>
        <div class="round-responses">
          <div
            v-for="(response, modelId) in round"
            :key="modelId"
            class="round-response"
          >
            <div class="response-model">{{ getModelName(modelId) }}</div>
            <div class="response-content" v-html="renderContent(response)"></div>
          </div>
        </div>
      </div>

      <div v-if="debate.rounds.length === 0" class="no-rounds">
        No debate rounds yet
      </div>
    </div>

    <DebateControls
      v-if="debate.status !== 'completed'"
      :debate="debate"
      @continue="handleContinue"
      @pause="handlePause"
      @resume="handleResume"
      @end="handleEnd"
    />

    <!-- Resize handle -->
    <div class="resize-handle" @mousedown.stop="startResize"></div>
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount } from 'vue'
import { marked } from 'marked'
import DebateControls from './DebateControls.vue'

const props = defineProps({
  debate: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['drag', 'continue', 'pause', 'resume', 'end', 'delete'])

// Dragging state
const isDragging = ref(false)
const isResizing = ref(false)
const dragStart = ref({ x: 0, y: 0, tileX: 0, tileY: 0 })

// Expand/collapse state
const isExpanded = ref(false)

// Computed
const tileStyle = computed(() => ({
  left: `${props.debate.position.x}px`,
  top: `${props.debate.position.y}px`,
  width: `${props.debate.position.width}px`,
  minHeight: `${props.debate.position.height}px`
}))

// Get a summary from the last round for the compact view (no truncation, scrollable)
const lastRoundSummary = computed(() => {
  if (!props.debate.rounds || props.debate.rounds.length === 0) return null
  
  const lastRound = props.debate.rounds[props.debate.rounds.length - 1]
  const models = Object.keys(lastRound)
  if (models.length === 0) return null
  
  // Get the first model's response as summary
  const firstResponse = lastRound[models[0]]
  if (!firstResponse) return null
  
  marked.setOptions({ breaks: true, gfm: true })
  return marked(firstResponse)
})

// Methods
function getModelName(modelId) {
  return modelId.split('/').pop() || modelId
}

function renderContent(content) {
  if (!content) return ''
  marked.setOptions({ breaks: true, gfm: true })
  // Truncate for display in tiles
  const truncated = content.length > 500 ? content.slice(0, 500) + '...' : content
  return marked(truncated)
}

function handleContinue(prompt) {
  emit('continue', props.debate.id, prompt)
}

function handlePause() {
  emit('pause', props.debate.id)
}

function handleResume() {
  emit('resume', props.debate.id)
}

function handleEnd() {
  emit('end', props.debate.id)
}

function handleMouseDown(e) {
  // Ignore clicks on interactive elements and scrollable content
  if (e.target.closest('.tile-actions') || 
      e.target.closest('.resize-handle') || 
      e.target.closest('.debate-controls') ||
      e.target.closest('.debate-rounds') ||
      e.target.closest('.status-badge')) {
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
    tileX: props.debate.position.x,
    tileY: props.debate.position.y
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

  emit('drag', props.debate.id, {
    x: dragStart.value.tileX + deltaX,
    y: dragStart.value.tileY + deltaY,
    width: props.debate.position.width,
    height: props.debate.position.height
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
  const startWidth = props.debate.position.width
  const startHeight = props.debate.position.height

  function onResize(e) {
    const newWidth = Math.max(400, startWidth + (e.clientX - startX))
    const newHeight = Math.max(300, startHeight + (e.clientY - startY))

    emit('drag', props.debate.id, {
      x: props.debate.position.x,
      y: props.debate.position.y,
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

// Cleanup on unmount
onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
})
</script>

<style scoped>
.debate-tile {
  position: absolute;
  background: var(--bg-secondary);
  border: 2px solid var(--accent-blue);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  user-select: none;
}

.debate-tile.dragging {
  cursor: grabbing;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  z-index: 1000;
}

.debate-tile.active {
  border-color: var(--accent-green);
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

.debate-info {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.debate-icon {
  font-size: 1.25rem;
}

.debate-title {
  font-weight: 600;
  color: var(--text-primary);
}

.debate-mode {
  font-size: 0.75rem;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  text-transform: capitalize;
}

.status-badge {
  font-size: 0.75rem;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  text-transform: capitalize;
}

.status-badge.active {
  background: rgba(52, 211, 153, 0.2);
  color: var(--accent-green);
}

.status-badge.paused {
  background: rgba(251, 191, 36, 0.2);
  color: var(--accent-yellow);
}

.status-badge.completed {
  background: rgba(124, 92, 255, 0.2);
  color: var(--accent-primary);
}

.delete-btn {
  width: 24px;
  height: 24px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
  font-size: 0.75rem;
  margin-left: var(--spacing-xs);
}

.delete-btn:hover {
  border-color: var(--accent-red, #f87171);
  color: var(--accent-red, #f87171);
  background: rgba(248, 113, 113, 0.1);
}

.debate-models {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
  padding: var(--spacing-sm) var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
}

.model-badge {
  font-size: 0.75rem;
  background: var(--bg-tertiary);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
}

.debate-rounds {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm);
}

.debate-round {
  margin-bottom: var(--spacing-md);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.round-header {
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.2);
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-secondary);
}

.round-responses {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  padding: var(--spacing-sm);
}

.round-response {
  background: var(--bg-secondary);
  border-radius: var(--radius-sm);
  padding: var(--spacing-sm);
}

.response-model {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--accent-primary);
  margin-bottom: var(--spacing-xs);
}

.response-content {
  font-size: 0.8125rem;
  color: var(--text-primary);
  line-height: 1.5;
}

.response-content :deep(*) {
  color: inherit;
}

.response-content :deep(p) {
  margin: 0 0 var(--spacing-xs) 0;
  color: var(--text-primary);
}

.response-content :deep(p:last-child) {
  margin-bottom: 0;
}

.response-content :deep(h1),
.response-content :deep(h2),
.response-content :deep(h3),
.response-content :deep(h4),
.response-content :deep(h5),
.response-content :deep(h6) {
  color: var(--text-primary);
  font-weight: 600;
  margin: var(--spacing-xs) 0;
}

.response-content :deep(code) {
  background: var(--bg-tertiary);
  padding: 2px 4px;
  border-radius: 3px;
  font-size: 0.75rem;
  font-family: 'Fira Code', monospace;
  color: var(--text-primary);
}

.response-content :deep(pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-xs) 0;
  color: var(--text-primary);
}

.response-content :deep(pre code) {
  background: none;
  padding: 0;
  color: var(--text-primary);
}

.response-content :deep(ul),
.response-content :deep(ol) {
  margin: var(--spacing-xs) 0;
  padding-left: var(--spacing-md);
  color: var(--text-primary);
}

.response-content :deep(li) {
  margin-bottom: 2px;
  color: var(--text-primary);
}

.response-content :deep(a) {
  color: var(--accent-primary);
}

.no-rounds {
  text-align: center;
  color: var(--text-muted);
  padding: var(--spacing-lg);
}

.resize-handle {
  position: absolute;
  bottom: 0;
  right: 0;
  width: 16px;
  height: 16px;
  cursor: se-resize;
  background: linear-gradient(135deg, transparent 50%, var(--accent-blue) 50%);
  border-radius: 0 0 var(--radius-md) 0;
}

/* Compact Summary View */
.debate-summary {
  padding: var(--spacing-md);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
}

.summary-stats {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  font-size: 0.75rem;
  color: var(--text-muted);
}

.stat-divider {
  color: var(--bg-tertiary);
}

.conclusion-preview {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  padding: var(--spacing-sm);
  max-height: 200px;
  overflow-y: auto;
}

.conclusion-label {
  font-size: 0.6875rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: var(--spacing-xs);
}

.conclusion-text {
  font-size: 0.8125rem;
  color: var(--text-primary);
  line-height: 1.4;
}

.conclusion-text :deep(*) {
  color: inherit;
}

.conclusion-text :deep(p) {
  margin: 0;
  color: var(--text-primary);
}

.conclusion-text :deep(h1),
.conclusion-text :deep(h2),
.conclusion-text :deep(h3),
.conclusion-text :deep(h4),
.conclusion-text :deep(h5),
.conclusion-text :deep(h6) {
  color: var(--text-primary);
  font-weight: 600;
  margin: var(--spacing-xs) 0;
}

.conclusion-text :deep(code) {
  background: var(--bg-tertiary);
  padding: 2px 4px;
  border-radius: 3px;
  font-size: 0.75rem;
  font-family: 'Fira Code', monospace;
  color: var(--text-primary);
}

.conclusion-text :deep(pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-xs) 0;
  color: var(--text-primary);
}

.conclusion-text :deep(pre code) {
  background: none;
  padding: 0;
  color: var(--text-primary);
}

.conclusion-text :deep(ul),
.conclusion-text :deep(ol) {
  margin: var(--spacing-xs) 0;
  padding-left: var(--spacing-md);
  color: var(--text-primary);
}

.conclusion-text :deep(li) {
  margin-bottom: 2px;
  color: var(--text-primary);
}

.conclusion-text :deep(a) {
  color: var(--accent-primary);
}

.expand-btn,
.collapse-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.8125rem;
  cursor: pointer;
  transition: all 0.15s;
  width: 100%;
}

.expand-btn:hover,
.collapse-btn:hover {
  background: rgba(124, 92, 255, 0.1);
  border-color: var(--accent-primary);
  color: var(--accent-primary);
}

.expand-icon,
.collapse-icon {
  font-size: 0.625rem;
}

.collapse-btn {
  margin-bottom: var(--spacing-sm);
}

/* Expanded state adjustments */
.debate-tile.expanded {
  min-height: auto;
}
</style>
