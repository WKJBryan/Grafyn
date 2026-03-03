<template>
  <div
    class="debate-node"
    :class="{ dragging: isDragging, active: isActive, expanded: isExpanded }"
    :style="nodeStyle"
    @mousedown="handleMouseDown"
  >
    <div class="node-header">
      <span class="node-icon">⚔</span>
      <span class="node-type">Debate</span>
      <span class="mode-badge">{{ debate.debate_mode }}</span>
      <div class="node-actions">
        <button
          class="delete-btn"
          title="Delete debate"
          @click.stop="$emit('delete', debate.id)"
        >
          ×
        </button>
      </div>
    </div>
    
    <div class="node-content">
      <div class="stats">
        <span
          v-if="isDebateStreaming"
          class="live-badge"
        >LIVE</span>
        <span class="stat">{{ displayRounds.length }} round{{ displayRounds.length !== 1 ? 's' : '' }}</span>
        <span class="stat-divider">•</span>
        <span class="stat">{{ debate.participating_models.length }} models</span>
      </div>
      
      <div class="model-list">
        <span 
          v-for="model in debate.participating_models" 
          :key="model" 
          class="model-tag"
        >
          {{ getModelName(model) }}
        </span>
      </div>
      
      <div class="status-row">
        <span
          class="status-badge"
          :class="debate.status"
        >{{ debate.status }}</span>
      </div>
    </div>
    
    <!-- Summary preview in compact view -->
    <div
      v-if="lastRoundSummary && !isExpanded"
      class="conclusion-preview"
    >
      <div class="conclusion-label">
        Summary:
      </div>
      <div
        class="conclusion-text"
        v-html="lastRoundSummary"
      />
    </div>
    
    <div class="node-footer">
      <button
        class="expand-btn"
        :disabled="!hasRounds && !isDebateStreaming"
        @click.stop="toggleExpand"
      >
        {{ isExpanded ? '▲ Hide Fight' : '⚔ See Fight' }}
      </button>
      <button 
        v-if="debate.status !== 'completed'"
        class="continue-btn"
        @click.stop="$emit('continue', debate.id)"
      >
        Continue
      </button>
    </div>
    
    <!-- Connection points for source tiles (left side) -->
    <div class="connection-point in" />
    
    <!-- Expanded view overlay -->
    <div
      v-if="isExpanded"
      class="expanded-overlay"
      @click.stop
      @wheel.stop
    >
      <div class="expanded-header">
        <h4>Debate Rounds ({{ displayRounds.length }})</h4>
        <button
          class="close-btn"
          @click.stop="toggleExpand"
        >
          ×
        </button>
      </div>
      <div class="rounds-container">
        <div
          v-for="(round, index) in displayRounds"
          :key="index"
          class="round-card"
        >
          <div class="round-header">
            Round {{ index + 1 }}
          </div>
          <div class="round-responses">
            <div
              v-for="(response, modelId) in getRoundResponses(round)"
              :key="modelId"
              class="round-response"
            >
              <span class="response-model">{{ getModelName(modelId) }}</span>
              <div
                class="response-content"
                v-html="renderContent(response)"
              />
            </div>
          </div>
        </div>
        <!-- Show currently streaming round -->
        <div
          v-if="currentStreamingModels && Object.keys(currentStreamingModels).length > 0"
          class="round-card streaming"
        >
          <div class="round-header">
            Round {{ (streamingContent?.currentRound || displayRounds.length + 1) }}
            <span class="live-badge live-badge-sm">LIVE</span>
          </div>
          <div class="round-responses">
            <div
              v-for="(content, modelId) in currentStreamingModels"
              :key="modelId"
              class="round-response"
            >
              <span class="response-model">{{ getModelName(modelId) }}</span>
              <div
                class="response-content"
                v-html="renderContent(content)"
              />
            </div>
          </div>
        </div>
        <div
          v-if="!hasRounds && !isDebateStreaming"
          class="no-rounds"
        >
          No debate rounds yet
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onBeforeUnmount } from 'vue'
import { marked } from 'marked'

const props = defineProps({
  debate: {
    type: Object,
    required: true
  },
  isExpanded: {
    type: Boolean,
    default: false
  },
  streamingContent: {
    type: Object,
    default: null
  }
})

const emit = defineEmits(['drag', 'delete', 'expand', 'collapse', 'continue'])

// Dragging state
const isDragging = ref(false)
const dragStart = ref({ x: 0, y: 0, nodeX: 0, nodeY: 0 })

// Computed
const isActive = computed(() => props.debate.status === 'active')

// Whether this debate is currently streaming
const isDebateStreaming = computed(() => props.streamingContent != null)

// Currently streaming model content (for the in-progress round)
const currentStreamingModels = computed(() => {
  if (!props.streamingContent) return null
  return props.streamingContent.models
})

// Merge persisted rounds with completed streaming rounds
const displayRounds = computed(() => {
  const persisted = props.debate.rounds || []
  if (!props.streamingContent?.completedRounds?.length) return persisted

  // Streaming completedRounds that aren't yet persisted
  const persistedCount = persisted.length
  const streamingCompleted = props.streamingContent.completedRounds
    .filter(sr => sr.round_number > persistedCount)
    .map(sr => ({
      round_number: sr.round_number,
      topic: `Round ${sr.round_number}`,
      // Convert { models: { modelId: text } } to responses array format
      responses: Object.entries(sr.models).map(([modelId, content]) => ({
        model_id: modelId,
        model_name: modelId.split('/').pop() || modelId,
        content
      })),
      created_at: new Date().toISOString()
    }))

  return [...persisted, ...streamingCompleted]
})

const hasRounds = computed(() => displayRounds.value.length > 0)

const nodeStyle = computed(() => ({
  left: `${props.debate.position.x}px`,
  top: `${props.debate.position.y}px`,
  width: `${props.debate.position.width || 280}px`,
  minHeight: `${props.debate.position.height || 200}px`
}))

// Get a summary from the last round for the compact view
const lastRoundSummary = computed(() => {
  if (!hasRounds.value) return null

  const rounds = displayRounds.value
  const lastRound = rounds[rounds.length - 1]
  if (!lastRound) return null

  // Handle both object format and other formats
  const responses = getRoundResponses(lastRound)
  const modelIds = Object.keys(responses)
  if (modelIds.length === 0) return null

  // Get the first model's response as summary
  const firstResponse = responses[modelIds[0]]
  if (!firstResponse) return null

  marked.setOptions({ breaks: true, gfm: true })
  // Show full content (scrollable in UI)
  if (typeof firstResponse !== 'string') return null
  return marked(firstResponse)
})

// Methods
function toggleExpand() {
  if (props.isExpanded) {
    emit('collapse', props.debate.id)
  } else {
    emit('expand', props.debate.id)
  }
}

function getModelName(modelId) {
  if (!modelId || typeof modelId !== 'string') return 'Unknown'
  return modelId.split('/').pop() || modelId
}

// Helper to normalize round data structure
function getRoundResponses(round) {
  if (!round) return {}
  // Handle Rust DebateRound: { round_number, topic, responses: Vec<DebateResponse>, created_at }
  if (round.responses && Array.isArray(round.responses)) {
    const result = {}
    for (const resp of round.responses) {
      result[resp.model_id] = resp.content
    }
    return result
  }
  // Legacy fallback: already a { model_id: content } map (web backend)
  if (typeof round === 'object' && !Array.isArray(round) && !('round_number' in round)) {
    return round
  }
  return {}
}

function renderContent(content) {
  if (!content) return '<em>No content</em>'
  if (typeof content !== 'string') {
    content = JSON.stringify(content)
  }
  marked.setOptions({ breaks: true, gfm: true })
  return marked(content)  // Full content, no truncation
}

function handleMouseDown(e) {
  // Ignore clicks on interactive elements
  if (e.target.closest('.node-actions') || 
      e.target.closest('.node-footer') ||
      e.target.closest('.expanded-overlay') ||
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
    nodeX: props.debate.position.x,
    nodeY: props.debate.position.y
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
  
  emit('drag', props.debate.id, {
    x: dragStart.value.nodeX + deltaX,
    y: dragStart.value.nodeY + deltaY,
    width: props.debate.position.width,
    height: props.debate.position.height
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
.debate-node {
  position: absolute;
  background: linear-gradient(135deg, var(--bg-secondary) 0%, var(--bg-tertiary) 100%);
  border: 2px solid var(--accent-cyan);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 16px color-mix(in srgb, var(--accent-cyan) 20%, transparent);
  display: flex;
  flex-direction: column;
  overflow: visible;  /* Changed from hidden to allow expanded overlay to show */
  transition: left 0.5s ease-out, top 0.5s ease-out, box-shadow 0.15s, border-color 0.15s, transform 0.1s;
  user-select: none;
  cursor: grab;
}

.debate-node.expanded {
  z-index: 50;  /* Bring expanded node above others */
}

.debate-node:hover {
  box-shadow: 0 6px 20px color-mix(in srgb, var(--accent-cyan) 35%, transparent);
}

.debate-node.active {
  border-color: var(--accent-green);
  box-shadow: 0 4px 16px color-mix(in srgb, var(--accent-green) 30%, transparent);
}

.debate-node.dragging {
  cursor: grabbing;
  box-shadow: 0 12px 32px color-mix(in srgb, var(--accent-cyan) 40%, transparent);
  z-index: 1000;
  transform: scale(1.02);
}

.node-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-xs) var(--spacing-sm);
  background: color-mix(in srgb, var(--accent-cyan) 15%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--accent-cyan) 20%, transparent);
}

.node-icon {
  font-size: 1rem;
}

.node-type {
  font-size: 0.6875rem;
  font-weight: 600;
  color: var(--accent-cyan);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.mode-badge {
  flex: 1;
  font-size: 0.625rem;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  text-transform: capitalize;
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
  background: color-mix(in srgb, var(--accent-red) 20%, transparent);
  color: var(--accent-red);
}

.node-content {
  flex: 1;
  padding: var(--spacing-sm);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.stats {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  font-size: 0.6875rem;
  color: var(--text-muted);
}

.stat-divider {
  color: var(--bg-tertiary);
}

.model-list {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.model-tag {
  font-size: 0.625rem;
  background: var(--bg-tertiary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
}

.status-row {
  margin-top: auto;
}

/* Conclusion preview */
.conclusion-preview {
  padding: var(--spacing-sm);
  background: color-mix(in srgb, var(--accent-cyan) 5%, transparent);
  border-top: 1px solid var(--bg-tertiary);
  max-height: 150px;
  overflow-y: auto;
}

.conclusion-label {
  font-size: 0.5625rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 4px;
  font-weight: 600;
}

.conclusion-text {
  font-size: 0.75rem;
  color: var(--text-primary);
  line-height: 1.4;
  max-height: 120px;
  overflow-y: auto;
}

.conclusion-text :deep(p) {
  margin: 0 0 var(--spacing-xs) 0;
}

.conclusion-text :deep(p:last-child) {
  margin-bottom: 0;
}

.status-badge {
  font-size: 0.625rem;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  text-transform: capitalize;
}

.status-badge.active {
  background: color-mix(in srgb, var(--accent-green) 20%, transparent);
  color: var(--accent-green);
}

.status-badge.paused {
  background: color-mix(in srgb, var(--accent-yellow) 20%, transparent);
  color: var(--accent-yellow);
}

.status-badge.completed {
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
  color: var(--accent-primary);
}

.node-footer {
  display: flex;
  gap: 4px;
  padding: var(--spacing-xs) var(--spacing-sm);
  background: rgba(0, 0, 0, 0.1);
  border-top: 1px solid color-mix(in srgb, var(--accent-cyan) 20%, transparent);
}

.expand-btn, .continue-btn {
  flex: 1;
  padding: 4px 8px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.6875rem;
  cursor: pointer;
  transition: all 0.15s;
}

.expand-btn:hover:not(:disabled) {
  border-color: var(--accent-cyan);
  color: var(--accent-cyan);
}

.expand-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.continue-btn:hover {
  border-color: var(--accent-green);
  color: var(--accent-green);
  background: color-mix(in srgb, var(--accent-green) 10%, transparent);
}

/* Connection point */
.connection-point {
  position: absolute;
  width: 10px;
  height: 10px;
  background: var(--bg-primary);
  border: 2px solid var(--accent-cyan);
  border-radius: 50%;
}

.connection-point.in {
  left: -6px;
  top: 50%;
  transform: translateY(-50%);
}

/* Expanded overlay */
.expanded-overlay {
  position: absolute;
  top: 100%;
  left: 0;
  width: 450px;
  max-height: 500px;
  background: var(--bg-secondary);
  border: 2px solid var(--accent-cyan);
  border-radius: var(--radius-md);
  margin-top: var(--spacing-xs);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  z-index: 100;
  display: flex;
  flex-direction: column;
}

.expanded-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-sm);
  border-bottom: 1px solid var(--bg-tertiary);
}

.expanded-header h4 {
  margin: 0;
  font-size: 0.875rem;
  color: var(--text-primary);
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 1rem;
}

.close-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.rounds-container {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm);
}

.round-card {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  margin-bottom: var(--spacing-sm);
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
  padding: var(--spacing-sm);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.round-response {
  background: var(--bg-secondary);
  border-radius: var(--radius-sm);
  padding: var(--spacing-sm);
}

.response-model {
  font-size: 0.6875rem;
  font-weight: 600;
  color: var(--accent-cyan);
  display: block;
  margin-bottom: var(--spacing-xs);
}

.response-content {
  font-size: 0.8125rem;
  color: var(--text-primary);
  line-height: 1.5;
}

.response-content :deep(p) {
  margin: 0 0 var(--spacing-sm) 0;
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
  margin: var(--spacing-sm) 0 var(--spacing-xs) 0;
  color: var(--text-primary);
  font-weight: 600;
}

.response-content :deep(h1) { font-size: 1.25rem; }
.response-content :deep(h2) { font-size: 1.1rem; }
.response-content :deep(h3) { font-size: 1rem; }
.response-content :deep(h4) { font-size: 0.9rem; }

.response-content :deep(ul),
.response-content :deep(ol) {
  margin: var(--spacing-xs) 0;
  padding-left: var(--spacing-lg);
}

.response-content :deep(li) {
  margin: 4px 0;
}

.response-content :deep(strong),
.response-content :deep(b) {
  font-weight: 600;
  color: var(--text-primary);
}

.response-content :deep(em),
.response-content :deep(i) {
  font-style: italic;
}

.response-content :deep(code) {
  background: var(--bg-tertiary);
  padding: 2px 4px;
  border-radius: 3px;
  font-family: 'Fira Code', monospace;
  font-size: 0.75rem;
}

.response-content :deep(pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-xs) 0;
}

.response-content :deep(pre code) {
  background: none;
  padding: 0;
}

.response-content :deep(blockquote) {
  border-left: 3px solid var(--accent-cyan);
  padding-left: var(--spacing-sm);
  margin: var(--spacing-xs) 0;
  color: var(--text-secondary);
  font-style: italic;
}

.no-rounds {
  text-align: center;
  color: var(--text-muted);
  padding: var(--spacing-lg);
  font-size: 0.875rem;
}

/* LIVE badge */
.live-badge {
  font-size: 0.5625rem;
  font-weight: 700;
  color: var(--accent-red, #ef4444);
  background: color-mix(in srgb, var(--accent-red, #ef4444) 15%, transparent);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  animation: pulse-live 1.5s ease-in-out infinite;
}

.live-badge-sm {
  font-size: 0.5rem;
  padding: 1px 4px;
  margin-left: var(--spacing-xs);
}

@keyframes pulse-live {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

/* Streaming round card */
.round-card.streaming {
  border: 1px solid color-mix(in srgb, var(--accent-green) 40%, transparent);
  box-shadow: 0 0 8px color-mix(in srgb, var(--accent-green) 15%, transparent);
}
</style>
