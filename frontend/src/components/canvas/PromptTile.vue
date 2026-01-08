<template>
  <div
    class="prompt-tile"
    :class="{ selected, streaming: isStreaming }"
    :style="tileStyle"
    @mousedown.self="startDrag"
  >
    <div class="tile-header" @mousedown="startDrag">
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

    <div class="tile-footer">
      <span class="model-count">{{ Object.keys(tile.responses).length }} models</span>
      <span class="timestamp">{{ formatTime(tile.created_at) }}</span>
    </div>

    <!-- Resize handle -->
    <div class="resize-handle" @mousedown.stop="startResize"></div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
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

const emit = defineEmits(['drag', 'select'])

// Dragging state
const isDragging = ref(false)
const isResizing = ref(false)
const dragOffset = ref({ x: 0, y: 0 })

// Computed
const isStreaming = computed(() => {
  return Object.values(props.tile.responses).some(r => r.status === 'streaming')
})

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
  const count = Object.keys(props.tile.responses).length
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

function startDrag(e) {
  if (e.target.closest('.tile-actions') || e.target.closest('.resize-handle')) return

  isDragging.value = true
  dragOffset.value = {
    x: e.clientX - props.tile.position.x,
    y: e.clientY - props.tile.position.y
  }

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
}

function onDrag(e) {
  if (!isDragging.value) return

  emit('drag', props.tile.id, {
    x: e.clientX - dragOffset.value.x,
    y: e.clientY - dragOffset.value.y,
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

.responses-grid {
  display: grid;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
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
