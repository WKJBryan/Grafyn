<template>
  <div class="graph-view" ref="container">
    <div class="graph-toolbar">
      <div class="toolbar-stats" v-if="stats">
        <span class="stat-item">{{ stats.nodes }} Notes</span>
        <span class="stat-divider">•</span>
        <span class="stat-item">{{ stats.edges }} Links</span>
      </div>
      <div class="toolbar-actions">
        <div class="color-picker-wrapper" title="Change Node Color">
          <input type="color" v-model="userColor" @input="updateNodeColors" class="color-input">
        </div>
        <button class="btn btn-secondary btn-sm" @click="refreshGraph" title="Refresh Graph">
          <span class="icon">&#8634;</span>
        </button>
        <button class="btn btn-secondary btn-sm" @click="resetZoom" title="Reset Zoom">
            <span class="icon">&#8693;</span>
        </button>
      </div>
    </div>
    <div class="graph-canvas" ref="canvas"></div>
    <div class="loading-overlay" v-if="loading">
      <div class="spinner"></div>
      <p>Loading graph...</p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onBeforeUnmount, watch } from 'vue'
import * as d3 from 'd3'
import { graph as graphApi } from '../api/client'

const props = defineProps({
  width: {
    type: Number,
    default: 0
  },
  height: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits(['node-click'])

const container = ref(null)
const canvas = ref(null)
const loading = ref(false)
const stats = ref(null)
const userColor = ref('#34d399') // Default green

// D3 variables
let simulation = null
let svg = null
let zoom = null
let width = 800
let height = 600

// Graph data
let nodes = []
let links = []

onMounted(() => {
  // Initial load
  loadGraph()
  
  // Resize observer
  const resizeObserver = new ResizeObserver(entries => {
    for (const entry of entries) {
      if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
        width = entry.contentRect.width
        height = entry.contentRect.height
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

async function loadGraph() {
  loading.value = true
  try {
    const data = await graphApi.full()
    nodes = data.nodes.map(d => ({ ...d })) // Clone to avoid mutation issues
    links = data.links.map(d => ({ ...d }))
    
    stats.value = {
      nodes: nodes.length,
      edges: links.length
    }
    
    initGraph()
  } catch (error) {
    console.error('Failed to load graph:', error)
  } finally {
    loading.value = false
  }
}

function initGraph() {
  // Clear previous
  if (canvas.value) canvas.value.innerHTML = ''
  
  // Create SVG
  svg = d3.select(canvas.value)
    .append('svg')
    .attr('width', '100%')
    .attr('height', '100%')
    .attr('viewBox', [0, 0, width, height])
    
  // Add group for zoom
  const g = svg.append('g')
  
  // Zoom behavior
  zoom = d3.zoom()
    .scaleExtent([0.1, 4])
    .on('zoom', (event) => {
      g.attr('transform', event.transform)
    })
    
  svg.call(zoom)
  
  // Simulation
  simulation = d3.forceSimulation(nodes)
    .force('link', d3.forceLink(links).id(d => d.id).distance(100))
    .force('charge', d3.forceManyBody().strength(-300))
    .force('center', d3.forceCenter(width / 2, height / 2))
    .force('collide', d3.forceCollide(30).strength(0.7))
    
  // Draw lines
  const link = g.append('g')
    .attr('stroke', '#4a4a4f')
    .attr('stroke-opacity', 0.6)
    .selectAll('line')
    .data(links)
    .join('line')
    .attr('stroke-width', 1)
    
  // Draw nodes
  const node = g.append('g')
    .selectAll('.node-group')
    .data(nodes)
    .join('g')
    .attr('class', 'node-group')
    .call(drag(simulation))
    .on('click', (event, d) => {
      event.stopPropagation()
      emit('node-click', d.id)
    })
    
  // Node circles
  node.append('circle')
    .attr('r', d => Math.max(5, Math.min(20, 5 + (d.val || 1) * 2)))
    .attr('fill', d => getNodeColor(d.val))
    .attr('stroke', '#fff')
    .attr('stroke-width', 1.5)
    
  // Node labels
  node.append('text')
    .text(d => d.label)
    .attr('x', 12)
    .attr('y', 4)
    .attr('font-size', '12px')
    .attr('fill', '#e8e8ed')
    .style('pointer-events', 'none')
    .style('text-shadow', '0 1px 3px rgba(0,0,0,0.8)')
    
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
}

function updateDimensions() {
  if (simulation) {
    simulation.force('center', d3.forceCenter(width / 2, height / 2))
    simulation.alpha(0.3).restart()
  }
}

function updateNodeColors() {
  if (svg) {
    svg.selectAll('circle')
       .attr('fill', userColor.value)
  }
}

function drag(simulation) {
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
  
  return d3.drag()
    .on('start', dragstarted)
    .on('drag', dragged)
    .on('end', dragended)
}

function getNodeColor(val) {
  // Use user color if set, otherwise default logic (which we are overriding now with the implementation)
  return userColor.value
}

function refreshGraph() {
  loadGraph()
}

function resetZoom() {
  if (svg && zoom) {
    svg.transition()
      .duration(750)
      .call(zoom.transform, d3.zoomIdentity)
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
  background: rgba(15, 15, 16, 0.8);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  z-index: 20;
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

.color-picker-wrapper {
  display: flex;
  align-items: center;
}

.color-input {
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  background: transparent;
}

.color-input::-webkit-color-swatch-wrapper {
  padding: 0;
}

.color-input::-webkit-color-swatch {
  border: 1px solid var(--bg-tertiary);
  border-radius: 4px;
}

.btn-sm {
  padding: 4px 8px;
  font-size: 0.75rem;
}
</style>
