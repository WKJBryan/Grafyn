<template>
  <div
    class="prompt-node"
    :class="{ dragging: isDragging, selected }"
    :style="nodeStyle"
    @mousedown="handleMouseDown"
  >
    <div class="node-header">
      <span class="node-icon">💬</span>
      <span class="node-type">Prompt</span>
      <div class="node-actions">
        <button
          class="delete-btn"
          @click.stop="$emit('delete', tile.id)"
          title="Delete prompt"
        >
          ×
        </button>
      </div>
    </div>
    
    <div class="node-content">
      <p class="prompt-text" :title="tile.prompt">{{ truncatedPrompt }}</p>
    </div>
    
    <div class="node-footer">
      <span class="model-count">→ {{ modelCount }} model{{ modelCount !== 1 ? 's' : '' }}</span>
      <span class="timestamp">{{ formatTime(tile.created_at) }}</span>
    </div>
    
    <!-- Connection point (right side) - visual indicator for edges -->
    <div class="connection-point out"></div>
    
    <!-- Branch indicator if this is a child prompt -->
    <div v-if="tile.parent_tile_id" class="branch-indicator">
      ⑂
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount } from 'vue'

const props = defineProps({
  tile: {
    type: Object,
    required: true
  },
  selected: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['drag', 'delete'])

// Dragging state
const isDragging = ref(false)
const dragStart = ref({ x: 0, y: 0, nodeX: 0, nodeY: 0 })

// Computed
const modelCount = computed(() => Object.keys(props.tile.responses).length)

const truncatedPrompt = computed(() => {
  const prompt = props.tile.prompt
  return prompt.length > 100 ? prompt.slice(0, 100) + '...' : prompt
})

const nodeStyle = computed(() => ({
  left: `${props.tile.position.x}px`,
  top: `${props.tile.position.y}px`,
  width: `${props.tile.position.width || 200}px`,
  minHeight: `${props.tile.position.height || 120}px`
}))

// Methods
function formatTime(dateStr) {
  if (!dateStr) return ''
  const date = new Date(dateStr)
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

function handleMouseDown(e) {
  // Ignore clicks on interactive elements
  if (e.target.closest('.node-actions') || e.target.closest('.delete-btn')) {
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
    nodeX: props.tile.position.x,
    nodeY: props.tile.position.y
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
  
  emit('drag', props.tile.id, {
    x: dragStart.value.nodeX + deltaX,
    y: dragStart.value.nodeY + deltaY,
    width: props.tile.position.width,
    height: props.tile.position.height
  })
}

function stopDrag() {
  isDragging.value = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.body.classList.remove('tile-dragging')
}

// Cleanup on unmount
onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.body.classList.remove('tile-dragging')
})
</script>

<style scoped>
.prompt-node {
  position: absolute;
  background: linear-gradient(135deg, var(--bg-secondary) 0%, var(--bg-tertiary) 100%);
  border: 2px solid var(--accent-primary);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 16px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: box-shadow 0.15s, border-color 0.15s, transform 0.1s;
  user-select: none;
  cursor: grab;
}

.prompt-node:hover {
  box-shadow: 0 6px 20px color-mix(in srgb, var(--accent-primary) 35%, transparent);
}

.prompt-node.selected {
  border-color: var(--accent-cyan);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-cyan) 30%, transparent), 0 6px 20px color-mix(in srgb, var(--accent-primary) 35%, transparent);
}

.prompt-node.dragging {
  cursor: grabbing;
  box-shadow: 0 12px 32px color-mix(in srgb, var(--accent-primary) 40%, transparent);
  z-index: 1000;
  transform: scale(1.02);
}

.node-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-xs) var(--spacing-sm);
  background: color-mix(in srgb, var(--accent-primary) 15%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--accent-primary) 20%, transparent);
}

.node-icon {
  font-size: 1rem;
}

.node-type {
  font-size: 0.6875rem;
  font-weight: 600;
  color: var(--accent-primary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  flex: 1;
}

.node-actions {
  display: flex;
  gap: 2px;
}

.delete-btn {
  width: 20px;
  height: 20px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1rem;
  transition: all 0.15s;
  line-height: 1;
}

.delete-btn:hover {
  background: rgba(248, 113, 113, 0.2);
  color: var(--accent-red);
}

.node-content {
  flex: 1;
  padding: var(--spacing-sm);
  min-height: 40px;
}

.prompt-text {
  font-size: 0.8125rem;
  color: var(--text-primary);
  line-height: 1.4;
  margin: 0;
  word-wrap: break-word;
  overflow-wrap: break-word;
}

.node-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.1);
  font-size: 0.6875rem;
  color: var(--text-muted);
}

.model-count {
  font-weight: 500;
  color: var(--accent-primary);
}

/* Connection point - visual indicator on right side */
.connection-point {
  position: absolute;
  width: 10px;
  height: 10px;
  background: var(--accent-primary);
  border: 2px solid var(--bg-primary);
  border-radius: 50%;
}

.connection-point.out {
  right: -6px;
  top: 50%;
  transform: translateY(-50%);
}

/* Branch indicator for child prompts */
.branch-indicator {
  position: absolute;
  left: -6px;
  top: 50%;
  transform: translateY(-50%);
  width: 12px;
  height: 12px;
  background: var(--bg-primary);
  border: 2px solid var(--accent-primary);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.5rem;
  color: var(--accent-primary);
}
</style>
