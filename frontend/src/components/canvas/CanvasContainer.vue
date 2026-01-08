<template>
  <div class="canvas-container" ref="container">
    <div class="canvas-toolbar">
      <div class="toolbar-left">
        <span class="session-title" v-if="session">{{ session.title }}</span>
        <span class="toolbar-stats" v-if="session">
          <span class="stat-item">{{ session.prompt_tiles?.length || 0 }} Tiles</span>
          <span class="stat-divider">|</span>
          <span class="stat-item">{{ session.debates?.length || 0 }} Debates</span>
        </span>
      </div>
      <div class="toolbar-actions">
        <button class="btn btn-secondary btn-sm" @click="resetZoom" title="Reset View">
          <span class="icon">&#8693;</span>
        </button>
        <button class="btn btn-secondary btn-sm" @click="zoomIn" title="Zoom In">
          <span class="icon">+</span>
        </button>
        <button class="btn btn-secondary btn-sm" @click="zoomOut" title="Zoom Out">
          <span class="icon">-</span>
        </button>
        <span class="zoom-level">{{ Math.round(viewport.zoom * 100) }}%</span>
      </div>
    </div>

    <div class="canvas-surface" ref="surface">
      <div class="canvas-content" :style="transformStyle">
        <!-- Prompt Tiles -->
        <PromptTile
          v-for="tile in promptTiles"
          :key="tile.id"
          :tile="tile"
          :selected="selectedTiles.includes(tile.id)"
          :streaming-models="streamingModels"
          @drag="handleTileDrag"
          @select="handleTileSelect"
        />

        <!-- Debate Tiles -->
        <DebateTile
          v-for="debate in debates"
          :key="debate.id"
          :debate="debate"
          @drag="handleDebateDrag"
          @continue="handleDebateContinue"
        />
      </div>
    </div>

    <!-- Floating Actions -->
    <div class="canvas-floating-actions">
      <button
        class="btn btn-primary"
        @click="showPromptDialog = true"
        :disabled="!session"
      >
        + New Prompt
      </button>
      <button
        class="btn btn-secondary"
        :disabled="selectedTiles.length < 2"
        @click="handleStartDebate"
      >
        Debate ({{ selectedTiles.length }})
      </button>
      <button
        v-if="selectedTiles.length > 0"
        class="btn btn-ghost"
        @click="clearSelection"
      >
        Clear Selection
      </button>
    </div>

    <!-- Prompt Dialog -->
    <PromptDialog
      v-if="showPromptDialog"
      :models="availableModels"
      @submit="handlePromptSubmit"
      @cancel="showPromptDialog = false"
    />

    <!-- Loading Overlay -->
    <div class="loading-overlay" v-if="loading">
      <div class="spinner"></div>
      <p>Loading...</p>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import * as d3 from 'd3'
import { useCanvasStore } from '@/stores/canvas'
import PromptTile from './PromptTile.vue'
import DebateTile from './DebateTile.vue'
import PromptDialog from './PromptDialog.vue'

const props = defineProps({
  sessionId: {
    type: String,
    default: null
  }
})

const emit = defineEmits(['session-loaded'])

const canvasStore = useCanvasStore()

// Refs
const container = ref(null)
const surface = ref(null)

// Local state
const viewport = ref({ x: 0, y: 0, zoom: 1 })
const selectedTiles = ref([])
const showPromptDialog = ref(false)

// D3 zoom
let zoom = null

// Computed
const session = computed(() => canvasStore.currentSession)
const promptTiles = computed(() => canvasStore.promptTiles)
const debates = computed(() => canvasStore.debates)
const availableModels = computed(() => canvasStore.availableModels)
const loading = computed(() => canvasStore.loading)
const streamingModels = computed(() => canvasStore.streamingModels)

const transformStyle = computed(() => ({
  transform: `translate(${viewport.value.x}px, ${viewport.value.y}px) scale(${viewport.value.zoom})`
}))

// Watch for session changes
watch(() => props.sessionId, async (newId) => {
  if (newId) {
    await canvasStore.loadSession(newId)
    // Restore viewport from session
    if (session.value?.viewport) {
      viewport.value = { ...session.value.viewport }
      if (zoom && surface.value) {
        const transform = d3.zoomIdentity
          .translate(viewport.value.x, viewport.value.y)
          .scale(viewport.value.zoom)
        d3.select(surface.value).call(zoom.transform, transform)
      }
    }
    emit('session-loaded', session.value)
  }
}, { immediate: true })

// Lifecycle
onMounted(() => {
  initZoom()
  canvasStore.loadModels()
})

onBeforeUnmount(() => {
  // Save viewport state before unmounting
  if (session.value) {
    canvasStore.updateViewport(viewport.value)
  }
})

// Methods
function initZoom() {
  zoom = d3.zoom()
    .scaleExtent([0.1, 3])
    .on('zoom', (event) => {
      viewport.value = {
        x: event.transform.x,
        y: event.transform.y,
        zoom: event.transform.k
      }
    })

  if (surface.value) {
    d3.select(surface.value).call(zoom)
  }
}

function resetZoom() {
  if (surface.value && zoom) {
    d3.select(surface.value)
      .transition()
      .duration(500)
      .call(zoom.transform, d3.zoomIdentity)
  }
}

function zoomIn() {
  if (surface.value && zoom) {
    d3.select(surface.value)
      .transition()
      .duration(300)
      .call(zoom.scaleBy, 1.3)
  }
}

function zoomOut() {
  if (surface.value && zoom) {
    d3.select(surface.value)
      .transition()
      .duration(300)
      .call(zoom.scaleBy, 0.7)
  }
}

function handleTileDrag(tileId, position) {
  canvasStore.updateTilePosition(tileId, position)
}

function handleDebateDrag(debateId, position) {
  canvasStore.updateTilePosition(debateId, position)
}

function handleTileSelect(tileId) {
  const index = selectedTiles.value.indexOf(tileId)
  if (index === -1) {
    selectedTiles.value.push(tileId)
  } else {
    selectedTiles.value.splice(index, 1)
  }
}

function clearSelection() {
  selectedTiles.value = []
}

async function handlePromptSubmit({ prompt, models, systemPrompt, temperature, maxTokens }) {
  showPromptDialog.value = false
  try {
    await canvasStore.sendPrompt(prompt, models, systemPrompt, temperature, maxTokens)
  } catch (err) {
    console.error('Failed to send prompt:', err)
  }
}

async function handleStartDebate() {
  if (selectedTiles.value.length < 2) return

  // Collect all models from selected tiles
  const models = new Set()
  for (const tileId of selectedTiles.value) {
    const tile = promptTiles.value.find(t => t.id === tileId)
    if (tile) {
      Object.keys(tile.responses).forEach(m => models.add(m))
    }
  }

  try {
    await canvasStore.startDebate(selectedTiles.value, Array.from(models), 'auto', 3)
    clearSelection()
  } catch (err) {
    console.error('Failed to start debate:', err)
  }
}

async function handleDebateContinue(debateId, prompt) {
  try {
    await canvasStore.continueDebate(debateId, prompt)
  } catch (err) {
    console.error('Failed to continue debate:', err)
  }
}
</script>

<style scoped>
.canvas-container {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-primary);
  position: relative;
  overflow: hidden;
}

.canvas-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-xs) var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
  background: var(--bg-secondary);
  z-index: 20;
  flex-shrink: 0;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
}

.session-title {
  font-weight: 600;
  color: var(--text-primary);
}

.toolbar-stats {
  font-size: 0.75rem;
  color: var(--text-muted);
  display: flex;
  gap: var(--spacing-xs);
}

.stat-divider {
  color: var(--bg-tertiary);
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
}

.zoom-level {
  font-size: 0.75rem;
  color: var(--text-muted);
  min-width: 45px;
  text-align: center;
}

.canvas-surface {
  flex: 1;
  width: 100%;
  height: 100%;
  overflow: hidden;
  cursor: grab;
  background:
    radial-gradient(circle at center, var(--bg-secondary) 0%, var(--bg-primary) 100%),
    repeating-linear-gradient(
      0deg,
      transparent,
      transparent 49px,
      var(--bg-tertiary) 49px,
      var(--bg-tertiary) 50px
    ),
    repeating-linear-gradient(
      90deg,
      transparent,
      transparent 49px,
      var(--bg-tertiary) 49px,
      var(--bg-tertiary) 50px
    );
  background-size: 100% 100%, 50px 50px, 50px 50px;
}

.canvas-surface:active {
  cursor: grabbing;
}

.canvas-content {
  transform-origin: 0 0;
  position: relative;
  width: 100%;
  height: 100%;
}

.canvas-floating-actions {
  position: absolute;
  bottom: var(--spacing-lg);
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  z-index: 10;
}

.loading-overlay {
  position: absolute;
  inset: 0;
  background: rgba(15, 15, 16, 0.8);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  z-index: 100;
}

.spinner {
  width: 32px;
  height: 32px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
  margin-bottom: var(--spacing-sm);
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.btn-sm {
  padding: 4px 8px;
  font-size: 0.75rem;
}

.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
}

.btn-ghost:hover {
  background: var(--bg-tertiary);
}

.icon {
  font-size: 1rem;
  line-height: 1;
}
</style>
