<template>
  <div
    class="debate-tile"
    :class="{ active: debate.status === 'active' }"
    :style="tileStyle"
    @mousedown.self="startDrag"
  >
    <div class="tile-header" @mousedown="startDrag">
      <div class="debate-info">
        <span class="debate-icon">&#9876;</span>
        <span class="debate-title">Debate</span>
        <span class="debate-mode">{{ debate.debate_mode }}</span>
      </div>
      <div class="tile-actions">
        <span class="status-badge" :class="debate.status">
          {{ debate.status }}
        </span>
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

    <div class="debate-rounds">
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
import { ref, computed } from 'vue'
import { marked } from 'marked'
import DebateControls from './DebateControls.vue'

const props = defineProps({
  debate: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['drag', 'continue', 'pause', 'resume', 'end'])

// Dragging state
const isDragging = ref(false)
const dragOffset = ref({ x: 0, y: 0 })

// Computed
const tileStyle = computed(() => ({
  left: `${props.debate.position.x}px`,
  top: `${props.debate.position.y}px`,
  width: `${props.debate.position.width}px`,
  minHeight: `${props.debate.position.height}px`
}))

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

function startDrag(e) {
  if (e.target.closest('.tile-actions') || e.target.closest('.resize-handle') || e.target.closest('.debate-controls')) return

  isDragging.value = true
  dragOffset.value = {
    x: e.clientX - props.debate.position.x,
    y: e.clientY - props.debate.position.y
  }

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
}

function onDrag(e) {
  if (!isDragging.value) return

  emit('drag', props.debate.id, {
    x: e.clientX - dragOffset.value.x,
    y: e.clientY - dragOffset.value.y,
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
    document.removeEventListener('mousemove', onResize)
    document.removeEventListener('mouseup', stopResize)
  }

  document.addEventListener('mousemove', onResize)
  document.addEventListener('mouseup', stopResize)
}
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

.response-content :deep(p) {
  margin: 0 0 var(--spacing-xs) 0;
}

.response-content :deep(p:last-child) {
  margin-bottom: 0;
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
</style>
