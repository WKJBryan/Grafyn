<template>
  <div class="canvas-container" ref="container">
    <div class="canvas-toolbar">
      <div class="toolbar-left">
        <span class="session-title" v-if="session">{{ session.title }}</span>
        <span class="toolbar-stats" v-if="session">
          <span class="stat-item">{{ promptTiles.length }} Prompts</span>
          <span class="stat-divider">|</span>
          <span class="stat-item">{{ llmNodes.length }} Responses</span>
          <span class="stat-divider">|</span>
          <span class="stat-item">{{ debates.length }} Debates</span>
        </span>
        <span v-if="session?.linked_note_id" class="linked-note-badge" title="Saved as note">
          Linked
        </span>
      </div>
      <div class="toolbar-actions">
        <div class="arrange-dropdown" ref="arrangeDropdown">
          <button
            class="btn btn-secondary btn-sm"
            @click="toggleArrangeMenu"
            :disabled="!session || promptTiles.length === 0"
            title="Auto-arrange nodes"
          >
            <span class="icon">⊞</span> Arrange
            <span class="dropdown-arrow">▼</span>
          </button>
          <div class="dropdown-menu" v-if="showArrangeMenu">
            <div class="dropdown-header">Layout Algorithm</div>
            <button
              class="dropdown-item"
              :class="{ active: layoutAlgorithm === 'hierarchical' }"
              @click="handleAutoArrange('hierarchical')"
            >
              <span class="layout-icon">🌳</span>
              <div class="layout-info">
                <span class="layout-name">Hierarchical</span>
                <span class="layout-desc">Tree layout from left to right</span>
              </div>
            </button>
            <button
              class="dropdown-item"
              :class="{ active: layoutAlgorithm === 'force-directed' }"
              @click="handleAutoArrange('force-directed')"
            >
              <span class="layout-icon">🔮</span>
              <div class="layout-info">
                <span class="layout-name">Force-Directed</span>
                <span class="layout-desc">Physics-based node positioning</span>
              </div>
            </button>
            <button
              class="dropdown-item"
              :class="{ active: layoutAlgorithm === 'circular' }"
              @click="handleAutoArrange('circular')"
            >
              <span class="layout-icon">⭕</span>
              <div class="layout-info">
                <span class="layout-name">Circular</span>
                <span class="layout-desc">Nodes arranged in a circle</span>
              </div>
            </button>
          </div>
        </div>
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
        <!-- SVG layer for edges -->
        <svg class="edges-layer">
          <!-- Prompt → LLM edges -->
          <path
            v-for="edge in promptToLLMEdges"
            :key="`p2l-${edge.source}-${edge.target}`"
            :d="edge.path"
            class="node-edge prompt-to-llm"
            :style="{ stroke: edge.color }"
          />
          <!-- LLM → Prompt branch edges -->
          <path
            v-for="edge in branchEdges"
            :key="`br-${edge.source}-${edge.target}`"
            :d="edge.path"
            class="node-edge branch-edge"
          />
          <!-- Debate edges -->
          <path
            v-for="edge in debateEdgePaths"
            :key="`db-${edge.source}-${edge.target}`"
            :d="edge.path"
            class="node-edge debate-edge"
          />
        </svg>

        <!-- Prompt Nodes -->
        <PromptNode
          v-for="tile in promptTiles"
          :key="`prompt-${tile.id}`"
          :tile="tile"
          :selected="selectedNodes.includes(`prompt:${tile.id}`)"
          @drag="handlePromptDrag"
          @delete="handleDeletePrompt"
        />

        <!-- LLM Response Nodes -->
        <LLMNode
          v-for="node in llmNodes"
          :key="`llm-${node.tileId}-${node.modelId}`"
          :tile-id="node.tileId"
          :model-id="node.modelId"
          :response="node.response"
          :is-streaming="streamingModels.has(node.modelId)"
          :selected="selectedNodes.includes(`llm:${node.tileId}:${node.modelId}`)"
          :available-models="availableModels"
          @drag="handleLLMDrag"
          @branch="handleLLMBranch"
          @select="handleNodeSelect"
          @delete="handleDeleteLLMNode"
          @regenerate="handleRegenerate"
          @show-add-model-dialog="handleShowAddModelDialog"
        />

        <!-- Debate Nodes -->
        <DebateNode
          v-for="debate in debates"
          :key="`debate-${debate.id}`"
          :debate="debate"
          :is-expanded="expandedDebates.includes(debate.id)"
          @drag="handleDebateDrag"
          @delete="handleDeleteDebate"
          @expand="expandDebate"
          @collapse="collapseDebate"
          @continue="handleDebateContinue"
        />
      </div>
    </div>

    <!-- Minimap -->
    <div class="minimap" v-if="session && (promptTiles.length > 0 || llmNodes.length > 0)">
      <div class="minimap-content">
        <!-- Prompt nodes in minimap -->
        <div
          v-for="tile in promptTiles"
          :key="'mini-prompt-' + tile.id"
          class="minimap-node minimap-prompt"
          :style="minimapPromptStyle(tile)"
          @click="panToNode(tile.position)"
        ></div>
        <!-- LLM nodes in minimap -->
        <div
          v-for="node in llmNodes"
          :key="'mini-llm-' + node.tileId + '-' + node.modelId"
          class="minimap-node minimap-llm"
          :style="minimapLLMStyle(node)"
          @click="panToNode(node.response.position)"
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
        :disabled="selectedLLMNodes.length < 2"
        @click="handleStartDebate"
      >
        Debate ({{ selectedLLMNodes.length }})
      </button>
      <button
        v-if="selectedNodes.length > 0"
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

    <!-- Add Model Dialog -->
    <AddModelDialog
      v-if="showAddModelDialog"
      :models="availableModels"
      :existing-model-ids="addModelContext?.existingModelIds || []"
      @submit="handleAddModelDialogSubmit"
      @cancel="handleAddModelDialogCancel"
    />

    <!-- Loading Overlay -->
    <div class="loading-overlay" v-if="loading">
      <div class="spinner"></div>
      <p>Loading...</p>
    </div>

    <!-- Arranging Overlay -->
    <div class="arranging-overlay" v-if="arranging">
      <div class="spinner"></div>
      <p>Arranging nodes...</p>
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
import PromptNode from './PromptNode.vue'
import LLMNode from './LLMNode.vue'
import DebateNode from './DebateNode.vue'
import PromptDialog from './PromptDialog.vue'
import AddModelDialog from './AddModelDialog.vue'

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
const selectedNodes = ref([])  // Format: "prompt:{id}", "llm:{tileId}:{modelId}", "debate:{id}"
const showPromptDialog = ref(false)
const saving = ref(false)
const saveMessage = ref(null)
const branchContext = ref(null)  // { parentTileId, parentModelId, parentContent }
const expandedDebates = ref([])  // IDs of expanded debate nodes
const showAddModelDialog = ref(false)
const addModelContext = ref(null)  // { tileId, existingModelIds }
const showArrangeMenu = ref(false)
const layoutAlgorithm = ref('hierarchical')  // 'hierarchical', 'force-directed', 'circular'
const arranging = ref(false)  // Loading state during arrangement
const arrangeDropdown = ref(null)  // Ref for dropdown element


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

// Flatten all LLM responses into individual node objects
const llmNodes = computed(() => {
  const nodes = []
  for (const tile of promptTiles.value) {
    for (const [modelId, response] of Object.entries(tile.responses)) {
      nodes.push({
        tileId: tile.id,
        modelId,
        response
      })
    }
  }
  return nodes
})

// Get selected LLM nodes for debate functionality
const selectedLLMNodes = computed(() => {
  return selectedNodes.value
    .filter(id => id.startsWith('llm:'))
    .map(id => {
      const parts = id.split(':')
      return { tileId: parts[1], modelId: parts.slice(2).join(':') }
    })
})

// Compute Prompt → LLM edge paths
const promptToLLMEdges = computed(() => {
  const edges = []
  
  for (const tile of promptTiles.value) {
    const promptPos = tile.position
    
    for (const [modelId, response] of Object.entries(tile.responses)) {
      const llmPos = response.position
      
      // Source: right side of prompt node
      const sourceX = promptPos.x + (promptPos.width || 200)
      const sourceY = promptPos.y + (promptPos.height || 120) / 2
      
      // Target: left side of LLM node
      const targetX = llmPos.x
      const targetY = llmPos.y + (llmPos.height || 200) / 2
      
      // Create bezier curve
      const midX = (sourceX + targetX) / 2
      const path = `M ${sourceX} ${sourceY} C ${midX} ${sourceY}, ${midX} ${targetY}, ${targetX} ${targetY}`
      
      edges.push({
        source: `prompt:${tile.id}`,
        target: `llm:${tile.id}:${modelId}`,
        color: response.color || '#7c5cff',
        path
      })
    }
  }
  
  return edges
})

// Compute LLM → Prompt branch edge paths
const branchEdges = computed(() => {
  const edges = []
  
  for (const tile of promptTiles.value) {
    if (!tile.parent_tile_id || !tile.parent_model_id) continue
    
    // Find parent tile
    const parentTile = promptTiles.value.find(t => t.id === tile.parent_tile_id)
    if (!parentTile || !parentTile.responses[tile.parent_model_id]) continue
    
    const parentResponse = parentTile.responses[tile.parent_model_id]
    const parentPos = parentResponse.position
    const childPos = tile.position
    
    // Source: right side of parent LLM node
    const sourceX = parentPos.x + (parentPos.width || 280)
    const sourceY = parentPos.y + (parentPos.height || 200) / 2
    
    // Target: left side of child prompt node
    const targetX = childPos.x
    const targetY = childPos.y + (childPos.height || 120) / 2
    
    // Create bezier curve
    const midX = (sourceX + targetX) / 2
    const path = `M ${sourceX} ${sourceY} C ${midX} ${sourceY}, ${midX} ${targetY}, ${targetX} ${targetY}`
    
    edges.push({
      source: `llm:${tile.parent_tile_id}:${tile.parent_model_id}`,
      target: `prompt:${tile.id}`,
      path
    })
  }
  
  return edges
})

// Compute Debate edge paths (LLM → Debate)
const debateEdgePaths = computed(() => {
  const edges = []
  
  for (const debate of debates.value) {
    const debatePos = debate.position
    
    for (const sourceTileId of debate.source_tile_ids || []) {
      const sourceTile = promptTiles.value.find(t => t.id === sourceTileId)
      if (!sourceTile) continue
      
      for (const modelId of debate.participating_models || []) {
        if (!sourceTile.responses[modelId]) continue
        
        const llmPos = sourceTile.responses[modelId].position
        
        // Source: right side of LLM node
        const sourceX = llmPos.x + (llmPos.width || 280)
        const sourceY = llmPos.y + (llmPos.height || 200) / 2
        
        // Target: left side of debate node
        const targetX = debatePos.x
        const targetY = debatePos.y + (debatePos.height || 150) / 2
        
        // Create bezier curve
        const midX = (sourceX + targetX) / 2
        const path = `M ${sourceX} ${sourceY} C ${midX} ${sourceY}, ${midX} ${targetY}, ${targetX} ${targetY}`
        
        edges.push({
          source: `llm:${sourceTileId}:${modelId}`,
          target: `debate:${debate.id}`,
          path
        })
      }
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

function minimapPromptStyle(tile) {
  return {
    left: `${tile.position.x * MINIMAP_SCALE}px`,
    top: `${tile.position.y * MINIMAP_SCALE}px`,
    width: `${Math.max(4, (tile.position.width || 200) * MINIMAP_SCALE)}px`,
    height: `${Math.max(3, (tile.position.height || 120) * MINIMAP_SCALE)}px`,
    background: 'var(--accent-primary)'
  }
}

function minimapLLMStyle(node) {
  return {
    left: `${node.response.position.x * MINIMAP_SCALE}px`,
    top: `${node.response.position.y * MINIMAP_SCALE}px`,
    width: `${Math.max(4, (node.response.position.width || 280) * MINIMAP_SCALE)}px`,
    height: `${Math.max(3, (node.response.position.height || 200) * MINIMAP_SCALE)}px`,
    background: node.response.color || '#7c5cff'
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

function panToNode(position) {
  const centerX = position.x + (position.width || 200) / 2
  const centerY = position.y + (position.height || 200) / 2
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
  
  // Add click outside listener for dropdown
  document.addEventListener('click', handleClickOutside)
})

onBeforeUnmount(() => {
  // Save viewport state before unmounting
  if (session.value) {
    canvasStore.updateViewport(viewport.value)
  }
  
  // Remove click outside listener
  document.removeEventListener('click', handleClickOutside)
})

// Methods
function initZoom() {
  zoom = d3.zoom()
    .scaleExtent([0.1, 3])
    .filter((event) => {
      // Allow all non-wheel events
      if (event.type !== 'wheel') return true
      // Block wheel events from inside nodes (let them scroll)
      if (event.target.closest('.prompt-node') || 
          event.target.closest('.llm-node') || 
          event.target.closest('.debate-node') ||
          event.target.closest('.prompt-tile') || 
          event.target.closest('.debate-tile')) {
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

// Node drag handlers
function handlePromptDrag(tileId, position) {
  canvasStore.updateTilePosition(tileId, position)
}

function handleLLMDrag(tileId, modelId, position) {
  canvasStore.updateLLMNodePosition(tileId, modelId, position)
}

function handleDebateDrag(debateId, position) {
  canvasStore.updateTilePosition(debateId, position)
}

// Node delete handlers
async function handleDeletePrompt(tileId) {
  if (!confirm('Delete this prompt and all its responses? This action cannot be undone.')) {
    return
  }
  
  try {
    await canvasStore.deleteTile(tileId)
    // Remove related nodes from selection
    selectedNodes.value = selectedNodes.value.filter(id => !id.includes(tileId))
  } catch (err) {
    console.error('Failed to delete prompt:', err)
  }
}

async function handleDeleteLLMNode(info) {
  // TODO: Implement individual LLM node deletion
  // For now, we just deselect it
  const nodeId = `llm:${info.tileId}:${info.modelId}`
  selectedNodes.value = selectedNodes.value.filter(id => id !== nodeId)
}

// Handle regenerate response
async function handleRegenerate({ tileId, modelId }) {
  try {
    await canvasStore.regenerateResponse(tileId, modelId)
  } catch (err) {
    console.error('Failed to regenerate response:', err)
  }
}

// Handle showing the add model dialog
function handleShowAddModelDialog({ tileId, modelId }) {
  // Find the tile and get all existing model IDs
  const tile = promptTiles.value.find(t => t.id === tileId)
  if (!tile) return

  const existingModelIds = Object.keys(tile.responses || {})

  addModelContext.value = {
    tileId,
    existingModelIds
  }
  showAddModelDialog.value = true
}

// Handle add model dialog submit
async function handleAddModelDialogSubmit(newModelIds) {
  if (!addModelContext.value || newModelIds.length === 0) return

  try {
    await canvasStore.addModelToTile(addModelContext.value.tileId, newModelIds)
  } catch (err) {
    console.error('Failed to add models to tile:', err)
  } finally {
    showAddModelDialog.value = false
    addModelContext.value = null
  }
}

// Handle add model dialog cancel
function handleAddModelDialogCancel() {
  showAddModelDialog.value = false
  addModelContext.value = null
}

async function handleDeleteDebate(debateId) {
  if (!confirm('Delete this debate? This action cannot be undone.')) {
    return
  }
  
  try {
    await canvasStore.deleteTile(debateId)
    selectedNodes.value = selectedNodes.value.filter(id => id !== `debate:${debateId}`)
    expandedDebates.value = expandedDebates.value.filter(id => id !== debateId)
  } catch (err) {
    console.error('Failed to delete debate:', err)
  }
}

// Node selection handler
function handleNodeSelect({ tileId, modelId }) {
  const nodeId = `llm:${tileId}:${modelId}`
  const index = selectedNodes.value.indexOf(nodeId)
  if (index === -1) {
    selectedNodes.value.push(nodeId)
  } else {
    selectedNodes.value.splice(index, 1)
  }
}

function clearSelection() {
  selectedNodes.value = []
}

// Debate expand/collapse
function expandDebate(debateId) {
  if (!expandedDebates.value.includes(debateId)) {
    expandedDebates.value.push(debateId)
  }
}

function collapseDebate(debateId) {
  expandedDebates.value = expandedDebates.value.filter(id => id !== debateId)
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

// Handle branch from LLM node
function handleLLMBranch(tileId, modelId, prompt, contextMode = 'full_history', selectedModels = null) {
  // Get parent context
  const parentInfo = canvasStore.getParentResponseContent(tileId, modelId)
  branchContext.value = {
    parentTileId: tileId,
    parentModelId: modelId,
    parentContent: parentInfo
  }
  
  // If prompt is provided directly (inline branch), submit immediately
  if (prompt) {
    // Use selected models if provided, otherwise fall back to parent model
    const models = selectedModels && selectedModels.length > 0 ? selectedModels : [modelId]
    handlePromptSubmit({
      prompt,
      models,
      systemPrompt: null,
      temperature: 0.7,
      maxTokens: 2048,
      contextMode
    })
  } else {
    showPromptDialog.value = true
  }
}

function closeBranchDialog() {
  showPromptDialog.value = false
  branchContext.value = null
}

async function handleStartDebate() {
  if (selectedLLMNodes.value.length < 2) return

  // Extract unique models and tile IDs from selected LLM nodes
  const models = new Set()
  const tileIds = new Set()
  
  for (const node of selectedLLMNodes.value) {
    models.add(node.modelId)
    tileIds.add(node.tileId)
  }

  try {
    await canvasStore.startDebate(Array.from(tileIds), Array.from(models), 'auto', 3)
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

// Toggle arrange menu dropdown
function toggleArrangeMenu() {
  showArrangeMenu.value = !showArrangeMenu.value
}

// Close arrange menu when clicking outside
function handleClickOutside(event) {
  if (arrangeDropdown.value && !arrangeDropdown.value.contains(event.target)) {
    showArrangeMenu.value = false
  }
}

// Auto-arrange nodes with different layout algorithms
async function handleAutoArrange(algorithm = 'hierarchical') {
  if (!session.value || promptTiles.value.length === 0) return
  
  arranging.value = true
  showArrangeMenu.value = false
  layoutAlgorithm.value = algorithm
  
  const positions = {}
  const NODE_GAP = 80
  const PROMPT_WIDTH = 200
  const PROMPT_HEIGHT = 120
  const LLM_WIDTH = 280
  const LLM_HEIGHT = 220
  const DEBATE_WIDTH = 280
  const DEBATE_HEIGHT = 200
  
  try {
    if (algorithm === 'hierarchical') {
      // Hierarchical tree layout
      let globalY = 50
      
      const rootTiles = promptTiles.value.filter(t => !t.parent_tile_id)
      
      function layoutPromptTree(tile, startX, startY) {
        let treeHeight = 0
        let currentY = startY
        
        positions[`prompt:${tile.id}`] = {
          x: startX,
          y: currentY,
          width: PROMPT_WIDTH,
          height: PROMPT_HEIGHT
        }
        
        const responses = Object.entries(tile.responses || {})
        const llmX = startX + PROMPT_WIDTH + NODE_GAP
        
        if (responses.length === 0) {
          treeHeight = PROMPT_HEIGHT
        } else {
          for (const [modelId, response] of responses) {
            positions[`llm:${tile.id}:${modelId}`] = {
              x: llmX,
              y: currentY,
              width: LLM_WIDTH,
              height: LLM_HEIGHT
            }
            
            const branches = promptTiles.value.filter(
              t => t.parent_tile_id === tile.id && t.parent_model_id === modelId
            )
            
            let llmSubtreeHeight = LLM_HEIGHT
            
            if (branches.length > 0) {
              const branchX = llmX + LLM_WIDTH + NODE_GAP
              let branchY = currentY
              
              for (const branch of branches) {
                const branchHeight = layoutPromptTree(branch, branchX, branchY)
                branchY += branchHeight + NODE_GAP
                llmSubtreeHeight = Math.max(llmSubtreeHeight, branchY - currentY - NODE_GAP)
              }
            }
            
            currentY += llmSubtreeHeight + NODE_GAP
          }
          
          treeHeight = currentY - startY - NODE_GAP
        }
        
        if (responses.length > 0) {
          const promptCenterY = startY + (treeHeight - PROMPT_HEIGHT) / 2
          positions[`prompt:${tile.id}`].y = promptCenterY
        }
        
        return Math.max(treeHeight, PROMPT_HEIGHT)
      }
      
      for (const rootTile of rootTiles) {
        const treeHeight = layoutPromptTree(rootTile, 50, globalY)
        globalY += treeHeight + NODE_GAP * 2
      }
      
      // Position debates
      let maxX = 0
      for (const pos of Object.values(positions)) {
        maxX = Math.max(maxX, pos.x + (pos.width || LLM_WIDTH))
      }
      
      const debateX = maxX + NODE_GAP * 2
      let debateY = 50
      
      for (const debate of debates.value) {
        positions[`debate:${debate.id}`] = {
          x: debateX,
          y: debateY,
          width: DEBATE_WIDTH,
          height: DEBATE_HEIGHT
        }
        debateY += DEBATE_HEIGHT + NODE_GAP
      }
      
    } else if (algorithm === 'force-directed') {
      // Force-directed layout using D3
      const nodes = []
      const links = []
      
      // Create nodes
      for (const tile of promptTiles.value) {
        nodes.push({
          id: `prompt:${tile.id}`,
          type: 'prompt',
          width: PROMPT_WIDTH,
          height: PROMPT_HEIGHT,
          x: tile.position?.x || Math.random() * 800,
          y: tile.position?.y || Math.random() * 600
        })
        
        for (const [modelId, response] of Object.entries(tile.responses || {})) {
          const llmId = `llm:${tile.id}:${modelId}`
          nodes.push({
            id: llmId,
            type: 'llm',
            width: LLM_WIDTH,
            height: LLM_HEIGHT,
            x: response.position?.x || Math.random() * 800,
            y: response.position?.y || Math.random() * 600
          })
          
          // Link prompt to LLM
          links.push({
            source: `prompt:${tile.id}`,
            target: llmId
          })
          
          // Link LLM to branches
          for (const branch of promptTiles.value) {
            if (branch.parent_tile_id === tile.id && branch.parent_model_id === modelId) {
              links.push({
                source: llmId,
                target: `prompt:${branch.id}`
              })
            }
          }
        }
      }
      
      for (const debate of debates.value) {
        nodes.push({
          id: `debate:${debate.id}`,
          type: 'debate',
          width: DEBATE_WIDTH,
          height: DEBATE_HEIGHT,
          x: debate.position?.x || Math.random() * 800,
          y: debate.position?.y || Math.random() * 600
        })
        
        // Link source LLMs to debate
        for (const sourceTileId of debate.source_tile_ids || []) {
          const sourceTile = promptTiles.value.find(t => t.id === sourceTileId)
          if (sourceTile) {
            for (const modelId of debate.participating_models || []) {
              if (sourceTile.responses[modelId]) {
                links.push({
                  source: `llm:${sourceTileId}:${modelId}`,
                  target: `debate:${debate.id}`
                })
              }
            }
          }
        }
      }
      
      // Run force simulation
      const simulation = d3.forceSimulation(nodes)
        .force('link', d3.forceLink(links).id(d => d.id).distance(150))
        .force('charge', d3.forceManyBody().strength(-300))
        .force('center', d3.forceCenter(400, 300))
        .force('collision', d3.forceCollide().radius(d => (d.width + d.height) / 4))
        .stop()
      
      // Run simulation for 300 iterations
      for (let i = 0; i < 300; i++) {
        simulation.tick()
      }
      
      // Extract final positions
      for (const node of nodes) {
        positions[node.id] = {
          x: Math.max(50, node.x - node.width / 2),
          y: Math.max(50, node.y - node.height / 2),
          width: node.width,
          height: node.height
        }
      }
      
    } else if (algorithm === 'circular') {
      // Circular layout
      const nodes = []
      
      // Collect all nodes
      for (const tile of promptTiles.value) {
        nodes.push({
          id: `prompt:${tile.id}`,
          width: PROMPT_WIDTH,
          height: PROMPT_HEIGHT
        })
        
        for (const [modelId, response] of Object.entries(tile.responses || {})) {
          nodes.push({
            id: `llm:${tile.id}:${modelId}`,
            width: LLM_WIDTH,
            height: LLM_HEIGHT
          })
        }
      }
      
      for (const debate of debates.value) {
        nodes.push({
          id: `debate:${debate.id}`,
          width: DEBATE_WIDTH,
          height: DEBATE_HEIGHT
        })
      }
      
      // Arrange in a circle
      const centerX = 400
      const centerY = 300
      const radius = Math.min(centerX, centerY) - 100
      const angleStep = (2 * Math.PI) / nodes.length
      
      nodes.forEach((node, i) => {
        const angle = i * angleStep
        positions[node.id] = {
          x: centerX + radius * Math.cos(angle) - node.width / 2,
          y: centerY + radius * Math.sin(angle) - node.height / 2,
          width: node.width,
          height: node.height
        }
      })
    }
    
    // Send batch update to backend
    await canvasStore.autoArrange(positions)
  } catch (err) {
    console.error('Failed to auto-arrange:', err)
  } finally {
    arranging.value = false
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

/* Arrange Dropdown */
.arrange-dropdown {
  position: relative;
  display: inline-block;
}

.dropdown-arrow {
  font-size: 0.7rem;
  margin-left: 4px;
  opacity: 0.7;
}

.dropdown-menu {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  min-width: 280px;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  z-index: 100;
  overflow: hidden;
}

.dropdown-header {
  padding: var(--spacing-sm) var(--spacing-md);
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--bg-tertiary);
}

.dropdown-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  background: transparent;
  border: none;
  cursor: pointer;
  transition: background 0.15s;
}

.dropdown-item:hover {
  background: var(--bg-tertiary);
}

.dropdown-item.active {
  background: color-mix(in srgb, var(--accent-primary) 15%, transparent);
  border-left: 3px solid var(--accent-primary);
}

.layout-icon {
  font-size: 1.25rem;
  flex-shrink: 0;
}

.layout-info {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}

.layout-name {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
}

.layout-desc {
  font-size: 0.75rem;
  color: var(--text-muted);
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

/* Node edge styles */
.node-edge {
  fill: none;
  stroke-width: 2.5;
  stroke-linecap: round;
  transition: stroke 0.15s;
}

.node-edge.prompt-to-llm {
  /* Stroke color set dynamically via style attribute, fallback to theme color */
  stroke: var(--accent-primary);
  filter: drop-shadow(0 0 2px color-mix(in srgb, var(--accent-primary) 40%, transparent));
}

.node-edge.branch-edge {
  stroke: var(--accent-primary);
  stroke-width: 2;
  stroke-dasharray: 8 4;
  filter: drop-shadow(0 0 2px color-mix(in srgb, var(--accent-primary) 40%, transparent));
}

.node-edge.debate-edge {
  stroke: var(--accent-cyan);
  stroke-width: 2;
  filter: drop-shadow(0 0 3px color-mix(in srgb, var(--accent-cyan) 50%, transparent));
}

/* Legacy tile-edge for backwards compatibility */
.tile-edge {
  fill: none;
  stroke: var(--accent-primary);
  stroke-width: 3;
  stroke-linecap: round;
  filter: drop-shadow(0 0 3px color-mix(in srgb, var(--accent-primary) 50%, transparent));
}

.tile-edge.debate-edge {
  stroke: var(--accent-cyan);
  filter: drop-shadow(0 0 3px color-mix(in srgb, var(--accent-cyan) 50%, transparent));
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

/* New node-based minimap styles */
.minimap-node {
  position: absolute;
  border-radius: 1px;
  opacity: 0.7;
  cursor: pointer;
  transition: opacity 0.15s;
}

.minimap-node:hover {
  opacity: 1;
}

.minimap-prompt {
  background: var(--accent-primary);
}



/* Legacy minimap-tile for backwards compatibility */
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

.loading-overlay,
.arranging-overlay {
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

.arranging-overlay {
  z-index: 150;
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
