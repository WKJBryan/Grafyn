<template>
  <div
    ref="container"
    class="graph-view"
  >
    <div class="graph-toolbar">
      <div
        v-if="stats"
        class="toolbar-stats"
      >
        <span class="stat-item">{{ stats.nodes }} Nodes</span>
        <span class="stat-divider">•</span>
        <span class="stat-item">{{ stats.edges }} Edges</span>
      </div>
      <div class="toolbar-actions">
        <div class="graph-legend">
          <span class="legend-item"><i class="legend-swatch explicit" />Explicit</span>
          <span class="legend-item"><i class="legend-swatch inferred" />Inferred</span>
          <span class="legend-item"><i class="legend-swatch topic" />Topic</span>
          <span class="legend-item"><i class="legend-swatch auto" />Auto</span>
        </div>
        <button
          class="btn btn-secondary btn-sm"
          title="Refresh Graph"
          @click="refreshGraph"
        >
          <span class="icon">&#8634;</span>
        </button>
        <button
          class="btn btn-secondary btn-sm"
          title="Reset Zoom"
          @click="resetZoom"
        >
          <span class="icon">&#8693;</span>
        </button>
      </div>
    </div>
    <div
      ref="canvas"
      class="graph-canvas"
    />
    
    <!-- Settings Panel -->
    <GraphSettings
      v-if="showSettings"
      @update:filters="handleFiltersUpdate"
      @update:display="handleDisplayUpdate"
      @update:forces="handleForcesUpdate"
      @animate="restartSimulation"
    />
    
    <div
      v-if="loading || waitingForBoot"
      class="loading-overlay"
    >
      <div class="spinner" />
      <p>{{ waitingForBoot ? 'Preparing graph...' : 'Loading graph...' }}</p>
    </div>

    <div
      v-if="!loading && !waitingForBoot && stats && stats.nodes === 0"
      class="graph-empty-state"
    >
      <p>Your knowledge graph is empty.</p>
      <p class="empty-hint">
        Create notes with [[wikilinks]] to build connections.
      </p>
    </div>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { select } from 'd3-selection'
import { zoom as d3Zoom, zoomIdentity } from 'd3-zoom'
import { forceSimulation, forceLink, forceManyBody, forceCenter, forceCollide, forceX, forceY } from 'd3-force'
import { drag as d3Drag } from 'd3-drag'
import 'd3-transition'
import { graph as graphApi } from '../api/client'
import GraphSettings from './GraphSettings.vue'
import { useBootStore } from '@/stores/boot'

const props = defineProps({
  width: {
    type: Number,
    default: 0
  },
  height: {
    type: Number,
    default: 0
  },
  showSettings: {
    type: Boolean,
    default: true
  },
  refreshKey: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits(['node-click'])

const container = ref(null)
const canvas = ref(null)
const loading = ref(false)
const stats = ref(null)
const boot = useBootStore()
const graphReady = computed(() => boot.ready || boot.failed)
const waitingForBoot = computed(() => !graphReady.value)

// D3 variables
let simulation = null
let svg = null
let zoom = null
let zoomGroup = null
let canvasWidth = 800
let canvasHeight = 600
let currentZoomLevel = 1

// Graph data
let nodes = []
let links = []
let allNodes = []  // Unfiltered data
let allLinks = []

// Settings state
const currentFilters = ref({
  showNotes: true,
  showHubs: true,
  search: ''
})

const currentDisplay = ref({
  arrows: true,
  textFade: 50,
  nodeSize: 8,
  linkThickness: 1
})

const currentForces = ref({
  center: 0.5,
  repel: -300,
  link: 1,
  distance: 100
})

onMounted(() => {
  // Initial load
  if (graphReady.value) {
    loadGraph()
  }
  
  // Resize observer
  const resizeObserver = new ResizeObserver(entries => {
    for (const entry of entries) {
      if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
        canvasWidth = entry.contentRect.width
        canvasHeight = entry.contentRect.height
        updateDimensions()
      }
    }
  })
  
  if (container.value) {
    resizeObserver.observe(container.value)
  }
  
  onBeforeUnmount(() => {
    resizeObserver.disconnect()
    if (simulation) simulation.stop()
  })
})

// Re-fetch graph when parent signals data changed
watch(() => props.refreshKey, (newVal, oldVal) => {
  if (newVal !== oldVal && graphReady.value) loadGraph()
})

watch(graphReady, (isReady, wasReady) => {
  if (isReady && !wasReady) {
    loadGraph()
  }
})

async function loadGraph() {
  loading.value = true
  try {
    const data = await graphApi.full()
    allNodes = data.nodes.map(d => ({ ...d })) // Clone to avoid mutation issues
    allLinks = data.links.map(d => ({ ...d }))
    
    stats.value = {
      nodes: allNodes.length,
      edges: allLinks.length
    }
    
    applyFilters()
    initGraph()
  } catch (error) {
    console.error('Failed to load graph:', error)
  } finally {
    loading.value = false
  }
}

function applyFilters() {
  const f = currentFilters.value
  
  // Filter nodes by kind and search
  nodes = allNodes.filter(node => {
    const nodeKind = node.node_kind || (node.note_type === 'hub' ? 'topic_hub' : 'note')
    if (nodeKind === 'topic_hub' && !f.showHubs) return false
    if (nodeKind !== 'topic_hub' && !f.showNotes) return false
    
    // Search filter
    if (f.search && !node.label.toLowerCase().includes(f.search.toLowerCase())) {
      return false
    }
    
    return true
  }).map(d => ({ ...d }))
  
  // Filter links to only include visible nodes
  const visibleIds = new Set(nodes.map(n => n.id))
  links = allLinks.filter(link => {
    const sourceId = typeof link.source === 'object' ? link.source.id : link.source
    const targetId = typeof link.target === 'object' ? link.target.id : link.target
    return visibleIds.has(sourceId) && visibleIds.has(targetId)
  }).map(d => ({ ...d }))
}

function initGraph() {
  // Clear previous
  if (canvas.value) canvas.value.innerHTML = ''
  
  // Create SVG
  svg = select(canvas.value)
    .append('svg')
    .attr('width', '100%')
    .attr('height', '100%')
    .attr('viewBox', [0, 0, canvasWidth, canvasHeight])
  
  // Add arrow marker definition
  const defs = svg.append('defs')
  defs.append('marker')
    .attr('id', 'arrowhead')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 20)
    .attr('refY', 0)
    .attr('markerWidth', 6)
    .attr('markerHeight', 6)
    .attr('orient', 'auto')
    .append('path')
    .attr('d', 'M0,-5L10,0L0,5')
    .attr('fill', '#4a4a4f')
    
  // Add group for zoom
  zoomGroup = svg.append('g')
  
  // Zoom behavior
  zoom = d3Zoom()
    .scaleExtent([0.1, 4])
    .on('zoom', (event) => {
      currentZoomLevel = event.transform.k
      zoomGroup.attr('transform', event.transform)
      updateTextOpacity()
    })
    
  svg.call(zoom)
  
  // Simulation with configurable forces
  simulation = forceSimulation(nodes)
    .force('link', forceLink(links).id(d => d.id).distance(currentForces.value.distance).strength(currentForces.value.link))
    .force('charge', forceManyBody().strength(currentForces.value.repel))
    .force('x', forceX(canvasWidth / 2).strength(currentForces.value.center))
    .force('y', forceY(canvasHeight / 2).strength(currentForces.value.center))
    .force('collide', forceCollide(30).strength(0.7))
    
  // Draw lines with relation-based coloring
  const link = zoomGroup.append('g')
    .attr('class', 'links-group')
    .attr('stroke-opacity', 0.6)
    .selectAll('line')
    .data(links)
    .join('line')
    .attr('stroke', d => getLinkColor(d))
    .attr('stroke-width', currentDisplay.value.linkThickness)
    .attr('marker-end', currentDisplay.value.arrows ? 'url(#arrowhead)' : null)
    
  // Draw nodes
  const node = zoomGroup.append('g')
    .attr('class', 'nodes-group')
    .selectAll('.node-group')
    .data(nodes)
    .join('g')
    .attr('class', 'node-group')
    .call(makeDrag(simulation))
    .on('click', (event, d) => {
      event.stopPropagation()
      emit('node-click', d.id)
    })
    
  // Node circles with hub-based coloring
  node.append('circle')
    .attr('r', d => getNodeRadius(d))
    .attr('fill', d => getNodeColor(d))
    .attr('stroke', '#fff')
    .attr('stroke-width', d => isTopicHub(d) ? 2.5 : 1.5)
    .attr('class', 'node-circle')
    
  // Node labels
  const textColor = getComputedStyle(document.documentElement).getPropertyValue('--text-primary').trim() || '#e8e8ed'
  node.append('text')
    .text(d => d.label)
    .attr('x', 12)
    .attr('y', 4)
    .attr('font-size', '12px')
    .attr('fill', textColor)
    .attr('class', 'graph-node-label')
    .style('pointer-events', 'none')
    
  // Animation loop
  simulation.on('tick', () => {
    link
      .attr('x1', d => d.source.x)
      .attr('y1', d => d.source.y)
      .attr('x2', d => d.target.x)
      .attr('y2', d => d.target.y)
      
    node
      .attr('transform', d => `translate(${d.x},${d.y})`)
  })
  
  // Initial text opacity
  updateTextOpacity()
}

function getNodeRadius(node) {
  const baseSize = currentDisplay.value.nodeSize
  const valFactor = Math.max(1, Math.min(3, (node.val || 1) * 0.5))
  return baseSize * valFactor * (isTopicHub(node) ? 1.15 : 1)
}

function isTopicHub(node) {
  return (node.node_kind || '') === 'topic_hub' || node.note_type === 'hub'
}

function getNodeColor(node) {
  if (isTopicHub(node)) return '#f59e0b'
  return node.group || '#6b7280'
}

function getLinkColor(link) {
  const edgeKind = link.edge_kind || 'note_link'
  const provenance = link.provenance || 'explicit'

  if (edgeKind === 'topic_membership') return '#f59e0b'
  if (edgeKind === 'topic_related') return '#c084fc'
  if (provenance === 'inferred') return '#38bdf8'
  if (provenance === 'auto_inserted') return '#22c55e'

  const relationColors = {
    supports: '#4ade80',
    contradicts: '#f87171',
    expands: '#60a5fa',
    questions: '#fbbf24',
    answers: '#a78bfa',
    example: '#2dd4bf',
    part_of: '#fb923c',
    related: '#6b7280',
    untyped: '#4a4a4f',
  }

  return relationColors[link.relation] || '#4a4a4f'
}

function updateTextOpacity() {
  if (!zoomGroup) return
  
  const threshold = currentDisplay.value.textFade / 100
  const fadeStart = 0.3 + threshold * 0.7  // Range: 0.3 to 1.0
  
  // Fade text based on zoom level
  const opacity = currentZoomLevel < fadeStart 
    ? Math.max(0, (currentZoomLevel - 0.1) / (fadeStart - 0.1))
    : 1
    
  zoomGroup.selectAll('.graph-node-label')
    .style('opacity', opacity)
}

function updateDimensions() {
  if (simulation) {
    simulation.force('center', forceCenter(canvasWidth / 2, canvasHeight / 2).strength(currentForces.value.center))
    simulation.alpha(0.3).restart()
  }
}

function handleFiltersUpdate(filters) {
  currentFilters.value = filters
  applyFilters()
  initGraph()
}

function handleDisplayUpdate(display) {
  currentDisplay.value = display
  
  if (!zoomGroup) return
  
  // Update arrows
  zoomGroup.selectAll('.links-group line')
    .attr('marker-end', display.arrows ? 'url(#arrowhead)' : null)
    .attr('stroke-width', display.linkThickness)
  
  // Update node sizes
  zoomGroup.selectAll('.node-circle')
    .attr('r', d => getNodeRadius(d))
  
  // Update text visibility
  updateTextOpacity()
}

function handleForcesUpdate(forces) {
  currentForces.value = forces
  
  if (!simulation) return
  
  // Update existing force parameters rather than replacing forces
  const linkForce = simulation.force('link')
  if (linkForce) {
    linkForce.distance(forces.distance).strength(forces.link)
  }
  
  const chargeForce = simulation.force('charge')
  if (chargeForce) {
    chargeForce.strength(forces.repel)
  }
  
  // Update X and Y forces for center pulling
  const xForce = simulation.force('x')
  if (xForce) {
    xForce.strength(forces.center)
  }
  
  const yForce = simulation.force('y')
  if (yForce) {
    yForce.strength(forces.center)
  }
  
  // Restart with new forces
  simulation.alpha(0.5).restart()
}

function restartSimulation() {
  if (simulation) {
    // Reset all nodes to center of the graph
    const centerX = canvasWidth / 2
    const centerY = canvasHeight / 2
    
    nodes.forEach(node => {
      // Reset position to center with some random spread
      node.x = centerX + (Math.random() - 0.5) * 100
      node.y = centerY + (Math.random() - 0.5) * 100
      // Clear any fixed position from dragging
      node.fx = null
      node.fy = null
    })
    
    // Restart with slower animation (lower alphaDecay = slower)
    simulation
      .alpha(1)
      .alphaDecay(0.01)  // Default is 0.0228, lower = slower
      .restart()
  }
}

function makeDrag(simulation) {
  function dragstarted(event) {
    if (!event.active) simulation.alphaTarget(0.3).restart()
    event.subject.fx = event.subject.x
    event.subject.fy = event.subject.y
  }

  function dragged(event) {
    event.subject.fx = event.x
    event.subject.fy = event.y
  }

  function dragended(event) {
    if (!event.active) simulation.alphaTarget(0)
    event.subject.fx = null
    event.subject.fy = null
  }

  return d3Drag()
    .on('start', dragstarted)
    .on('drag', dragged)
    .on('end', dragended)
}

function refreshGraph() {
  loadGraph()
}

function resetZoom() {
  if (svg && zoom) {
    svg.transition()
      .duration(750)
      .call(zoom.transform, zoomIdentity)
  }
}
</script>

<style scoped>
.graph-view {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
  overflow: hidden;
  position: relative;
}

.graph-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-xs) var(--spacing-sm);
  border-bottom: 1px solid var(--bg-tertiary);
  background: var(--bg-primary);
  z-index: 10;
}

.graph-legend {
  display: flex;
  gap: 0.75rem;
  margin-right: 0.75rem;
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.legend-item {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
}

.legend-swatch {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  display: inline-block;
}

.legend-swatch.explicit { background: #6b7280; }
.legend-swatch.inferred { background: #38bdf8; }
.legend-swatch.topic { background: #f59e0b; }
.legend-swatch.auto { background: #22c55e; }

.toolbar-stats {
  font-size: 0.75rem;
  color: var(--text-muted);
  display: flex;
  gap: var(--spacing-xs);
}

.toolbar-actions {
  display: flex;
  gap: var(--spacing-xs);
}

.graph-canvas {
  flex: 1;
  width: 100%;
  height: 100%;
  background: radial-gradient(circle at center, var(--bg-secondary) 0%, var(--bg-primary) 100%);
  cursor: grab;
}

.graph-canvas:active {
  cursor: grabbing;
}

.loading-overlay {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  z-index: 200;
}

.spinner {
  width: 24px;
  height: 24px;
  border: 2px solid var(--bg-tertiary);
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

/* Graph node label styling - adapts to light/dark theme */
.graph-canvas :deep(.graph-node-label) {
  fill: var(--text-primary);
  text-shadow: 0 1px 3px var(--bg-primary);
  transition: opacity 0.15s ease;
}

/* In dark mode, use light shadow for contrast */
:root[data-theme="dark"] .graph-canvas :deep(.graph-node-label) {
  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
}

/* In light mode, use subtle shadow for readability */
:root[data-theme="light"] .graph-canvas :deep(.graph-node-label) {
  text-shadow: 0 1px 2px rgba(255, 255, 255, 0.8);
}

.graph-empty-state {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  text-align: center;
  z-index: 5;
}

.graph-empty-state p {
  margin: 0;
  font-size: 0.9rem;
}

.empty-hint {
  margin-top: var(--spacing-sm) !important;
  font-size: 0.8rem !important;
  color: var(--text-muted);
  opacity: 0.7;
}
</style>
