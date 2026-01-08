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
        <span v-if="session?.linked_note_id" class="linked-note-badge" title="Saved as note">
          Linked
        </span>
      </div>
      <div class="toolbar-actions">
        <button
          class="btn btn-secondary btn-sm"
          @click="handleSaveAsNote"
          :disabled="!session || saving"
          title="Save as Note"
        >
          {{ saving ? 'Saving...' : 'Save as Note' }}
        </button>
        <span class="toolbar-divider"></span>
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
        <!-- SVG layer for edges (inside content for shared coordinates) -->
        <svg class="edges-layer">
          <path
            v-for="edge in edgePaths"
            :key="`${edge.source}-${edge.target}`"
            :d="edge.path"
            class="tile-edge"
            :class="{ 'debate-edge': edge.type === 'debate' }"
          />
        </svg>
        
        <!-- Prompt Tiles -->
        <PromptTile
          v-for="tile in promptTiles"
          :key="tile.id"
          :tile="tile"
          :selected="selectedTiles.includes(tile.id)"
          :streaming-models="streamingModels"
          @drag="handleTileDrag"
          @select="handleTileSelect"
          @branch="handleInlineBranch"
          @delete="handleDeleteTile"
        />

        <!-- Debate Tiles -->
        <DebateTile
          v-for="debate in debates"
          :key="debate.id"
          :debate="debate"
          @drag="handleDebateDrag"
          @continue="handleDebateContinue"
          @delete="handleDeleteTile"
        />
      </div>
    </div>

    <!-- Minimap -->
    <div class="minimap" v-if="session && promptTiles.length > 0">
      <div class="minimap-content">
        <div
          v-for="tile in promptTiles"
          :key="'mini-' + tile.id"
          class="minimap-tile"
          :style="minimapTileStyle(tile)"
          @click="panToTile(tile)"
        ></div>
        <div class="minimap-viewport" :style="minimapViewportStyle"></div>
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
      :branch-context="branchContext"
      @submit="handlePromptSubmit"
      @cancel="closeBranchDialog"
    />

    <!-- Loading Overlay -->
    <div class="loading-overlay" v-if="loading">
      <div class="spinner"></div>
      <p>Loading...</p>
    </div>

    <!-- Save Toast -->
    <div
      v-if="saveMessage"
      class="save-toast"
      :class="saveMessage.type"
    >
      {{ saveMessage.text }}
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
const saving = ref(false)
const saveMessage = ref(null)
const branchContext = ref(null)  // { parentTileId, parentModelId, parentContent }

// D3 zoom
let zoom = null

// Computed
const session = computed(() => canvasStore.currentSession)
const promptTiles = computed(() => canvasStore.promptTiles)
const debates = computed(() => canvasStore.debates)
const availableModels = computed(() => canvasStore.availableModels)
const loading = computed(() => canvasStore.loading)
const streamingModels = computed(() => canvasStore.streamingModels)
const tileEdges = computed(() => canvasStore.tileEdges)
const debateEdges = computed(() => canvasStore.debateEdges)

// Compute edge paths for SVG (prompt branches + debate connections)
const edgePaths = computed(() => {
  const edges = []
  
  // Prompt tile edges (parent-child branches)
  if (tileEdges.value.length && promptTiles.value.length) {
    for (const edge of tileEdges.value) {
      const sourceTile = promptTiles.value.find(t => t.id === edge.source_tile_id)
      const targetTile = promptTiles.value.find(t => t.id === edge.target_tile_id)
      
      if (!sourceTile || !targetTile) continue
      
      // Calculate connection points
      const sourceX = sourceTile.position.x + sourceTile.position.width
      const sourceY = sourceTile.position.y + sourceTile.position.height / 2
      const targetX = targetTile.position.x
      const targetY = targetTile.position.y + targetTile.position.height / 2
      
      // Create bezier curve
      const midX = (sourceX + targetX) / 2
      const path = `M ${sourceX} ${sourceY} C ${midX} ${sourceY}, ${midX} ${targetY}, ${targetX} ${targetY}`
      
      edges.push({
        source: edge.source_tile_id,
        target: edge.target_tile_id,
        modelId: edge.source_model_id,
        type: 'prompt',
        path
      })
    }
  }
  
  // Debate edges (from source tiles to debate tiles)
  if (debateEdges.value.length && promptTiles.value.length && debates.value.length) {
    for (const edge of debateEdges.value) {
      const sourceTile = promptTiles.value.find(t => t.id === edge.source_tile_id)
      const targetDebate = debates.value.find(d => d.id === edge.target_id)
      
      if (!sourceTile || !targetDebate) continue
      
      // Calculate connection points
      const sourceX = sourceTile.position.x + sourceTile.position.width
      const sourceY = sourceTile.position.y + sourceTile.position.height / 2
      const targetX = targetDebate.position.x
      const targetY = targetDebate.position.y + targetDebate.position.height / 2
      
      // Create bezier curve
      const midX = (sourceX + targetX) / 2
      const path = `M ${sourceX} ${sourceY} C ${midX} ${sourceY}, ${midX} ${targetY}, ${targetX} ${targetY}`
      
      edges.push({
        source: edge.source_tile_id,
        target: edge.target_id,
        type: 'debate',
        path
      })
    }
  }
  
  return edges
})

const transformStyle = computed(() => ({
  transform: `translate(${viewport.value.x}px, ${viewport.value.y}px) scale(${viewport.value.zoom})`
}))

// Minimap scale (canvas is max ~5000x5000, minimap is 150x100)
const MINIMAP_SCALE = 0.02
const MINIMAP_WIDTH = 150
const MINIMAP_HEIGHT = 100

function minimapTileStyle(tile) {
  return {
    left: `${tile.position.x * MINIMAP_SCALE}px`,
    top: `${tile.position.y * MINIMAP_SCALE}px`,
    width: `${Math.max(4, tile.position.width * MINIMAP_SCALE)}px`,
    height: `${Math.max(3, tile.position.height * MINIMAP_SCALE)}px`
  }
}

const minimapViewportStyle = computed(() => {
  if (!surface.value) return {}
  const w = (window.innerWidth / viewport.value.zoom) * MINIMAP_SCALE
  const h = (window.innerHeight / viewport.value.zoom) * MINIMAP_SCALE
  const x = (-viewport.value.x / viewport.value.zoom) * MINIMAP_SCALE
  const y = (-viewport.value.y / viewport.value.zoom) * MINIMAP_SCALE
  return {
    left: `${x}px`,
    top: `${y}px`,
    width: `${w}px`,
    height: `${h}px`
  }
})

function panToTile(tile) {
  const centerX = tile.position.x + tile.position.width / 2
  const centerY = tile.position.y + tile.position.height / 2
  const newX = -centerX * viewport.value.zoom + window.innerWidth / 2
  const newY = -centerY * viewport.value.zoom + window.innerHeight / 2
  
  viewport.value = { ...viewport.value, x: newX, y: newY }
  
  if (zoom && surface.value) {
    const transform = d3.zoomIdentity
      .translate(newX, newY)
      .scale(viewport.value.zoom)
    d3.select(surface.value)
      .transition()
      .duration(300)
      .call(zoom.transform, transform)
  }
}

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
    .filter((event) => {
      // Allow all non-wheel events
      if (event.type !== 'wheel') return true
      // Block wheel events from inside tiles (let them scroll)
      if (event.target.closest('.prompt-tile') || event.target.closest('.debate-tile')) {
        return false
      }
      return true
    })
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

async function handleDeleteTile(tileId) {
  if (!confirm('Are you sure you want to delete this tile? This action cannot be undone.')) {
    return
  }
  
  try {
    await canvasStore.deleteTile(tileId)
    // Also remove from selection if selected
    const index = selectedTiles.value.indexOf(tileId)
    if (index !== -1) {
      selectedTiles.value.splice(index, 1)
    }
  } catch (err) {
    console.error('Failed to delete tile:', err)
  }
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

async function handlePromptSubmit({ prompt, models, systemPrompt, temperature, maxTokens, contextMode }) {
  showPromptDialog.value = false

  try {
    if (branchContext.value) {
      // Branching from a tile
      await canvasStore.branchFromResponse(
        branchContext.value.parentTileId,
        branchContext.value.parentModelId,
        prompt,
        models,
        systemPrompt,
        temperature,
        maxTokens,
        contextMode || 'full_history'
      )
      branchContext.value = null
    } else {
      // Regular prompt
      await canvasStore.sendPrompt(prompt, models, systemPrompt, temperature, maxTokens)
    }
  } catch (err) {
    console.error('Failed to send prompt:', err)
  }
}

function handleBranch(tileId, modelId) {
  // Get parent context
  const parentInfo = canvasStore.getParentResponseContent(tileId, modelId)
  branchContext.value = {
    parentTileId: tileId,
    parentModelId: modelId,
    parentContent: parentInfo
  }
  showPromptDialog.value = true
}

// Handle inline branch (from tile's built-in input)
async function handleInlineBranch(tileId, modelId, prompt, models, contextMode = 'full_history') {
  try {
    await canvasStore.branchFromResponse(
      tileId,
      modelId,
      prompt,
      models,
      null,  // systemPrompt
      0.7,   // temperature
      2048,  // maxTokens
      contextMode
    )
  } catch (err) {
    console.error('Failed to branch:', err)
  }
}

function closeBranchDialog() {
  showPromptDialog.value = false
  branchContext.value = null
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

async function handleSaveAsNote() {
  if (!session.value || saving.value) return

  saving.value = true
  saveMessage.value = null

  try {
    const result = await canvasStore.saveAsNote()
    const action = result.updated ? 'Updated' : 'Saved as'
    saveMessage.value = {
      type: 'success',
      text: `${action} "${result.title}"`
    }

    // Clear success message after 3 seconds
    setTimeout(() => {
      saveMessage.value = null
    }, 3000)
  } catch (err) {
    console.error('Failed to save as note:', err)
    saveMessage.value = {
      type: 'error',
      text: 'Failed to save as note'
    }
  } finally {
    saving.value = false
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

.linked-note-badge {
  font-size: 0.6875rem;
  padding: 2px 6px;
  background: rgba(74, 222, 128, 0.2);
  color: var(--accent-green, #4ade80);
  border-radius: var(--radius-sm);
}

.toolbar-divider {
  width: 1px;
  height: 20px;
  background: var(--bg-tertiary);
  margin: 0 var(--spacing-xs);
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

.edges-layer {
  position: absolute;
  top: 0;
  left: 0;
  width: 10000px;
  height: 10000px;
  pointer-events: none;
  z-index: 0;
  overflow: visible;
}

.tile-edge {
  fill: none;
  stroke: #7c5cff;
  stroke-width: 3;
  stroke-linecap: round;
  filter: drop-shadow(0 0 3px rgba(124, 92, 255, 0.5));
}

.tile-edge.debate-edge {
  stroke: #22d3ee;
  filter: drop-shadow(0 0 3px rgba(34, 211, 238, 0.5));
}

/* Minimap */
.minimap {
  position: absolute;
  top: 60px;
  right: 16px;
  width: 150px;
  height: 100px;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
  z-index: 50;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
}

.minimap-content {
  position: relative;
  width: 100%;
  height: 100%;
}

.minimap-tile {
  position: absolute;
  background: var(--accent-primary);
  border-radius: 1px;
  opacity: 0.7;
  cursor: pointer;
}

.minimap-tile:hover {
  opacity: 1;
}

.minimap-viewport {
  position: absolute;
  border: 1px solid var(--text-primary);
  background: rgba(255, 255, 255, 0.1);
  pointer-events: none;
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

.save-toast {
  position: absolute;
  top: 60px;
  right: var(--spacing-md);
  padding: var(--spacing-sm) var(--spacing-md);
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  z-index: 200;
  animation: slideIn 0.3s ease;
}

.save-toast.success {
  background: rgba(74, 222, 128, 0.2);
  color: var(--accent-green, #4ade80);
  border: 1px solid rgba(74, 222, 128, 0.3);
}

.save-toast.error {
  background: rgba(248, 113, 113, 0.2);
  color: var(--accent-red, #f87171);
  border: 1px solid rgba(248, 113, 113, 0.3);
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateX(20px);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}
</style>
