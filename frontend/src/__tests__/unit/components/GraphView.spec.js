import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import GraphView from '@/components/GraphView.vue'
import * as apiClient from '@/api/client'
import { forceSimulation, forceCenter, forceLink } from 'd3-force'
import { zoom as d3Zoom } from 'd3-zoom'
import { useBootStore } from '@/stores/boot'
// Mock GraphSettings child component
vi.mock('@/components/GraphSettings.vue', () => ({
  default: {
    name: 'GraphSettings',
    template: '<div class="graph-settings-stub"></div>',
    emits: ['update:filters', 'update:display', 'update:forces', 'animate'],
  },
}))

// Full D3 Mock
const mockSelection = {
  append: vi.fn().mockReturnThis(),
  attr: vi.fn().mockReturnThis(),
  style: vi.fn().mockReturnThis(),
  call: vi.fn().mockReturnThis(),
  selectAll: vi.fn().mockReturnThis(),
  data: vi.fn().mockReturnThis(),
  join: vi.fn().mockReturnThis(),
  on: vi.fn().mockReturnThis(),
  transition: vi.fn().mockReturnThis(),
  duration: vi.fn().mockReturnThis(),
  text: vi.fn().mockReturnThis(),
  remove: vi.fn().mockReturnThis(),
}
const labelSelection = { style: vi.fn().mockReturnThis() }
const linkSelection = { style: vi.fn().mockReturnThis(), attr: vi.fn().mockReturnThis() }
const circleSelection = { style: vi.fn().mockReturnThis(), attr: vi.fn().mockReturnThis() }
mockSelection.selectAll.mockImplementation(selector => {
  if (selector === '.graph-node-label') return labelSelection
  if (selector === '.links-group line') return linkSelection
  if (selector === '.node-circle') return circleSelection
  return mockSelection
})

vi.mock('d3-selection', () => ({
  select: vi.fn(() => mockSelection),
}))

vi.mock('d3-force', () => ({
  // A faithful-enough simulation mock: `.force(name)` returns the previously
  // registered force object (like real d3-force), rather than always returning
  // the simulation itself — needed so tests can assert on force.x()/force.y()
  // calls made through simulation.force('x')/simulation.force('y').
  forceSimulation: vi.fn(() => {
    const registeredForces = {}
    const sim = {
      force: vi.fn((name, force) => {
        if (force === undefined) return registeredForces[name]
        registeredForces[name] = force
        return sim
      }),
      on: vi.fn().mockReturnThis(),
      stop: vi.fn(),
      alpha: vi.fn().mockReturnThis(),
      alphaTarget: vi.fn().mockReturnThis(),
      restart: vi.fn().mockReturnThis(),
      nodes: vi.fn().mockReturnThis(),
      alphaDecay: vi.fn().mockReturnThis(),
    }
    return sim
  }),
  forceLink: vi.fn(() => ({
    id: vi.fn().mockReturnThis(),
    distance: vi.fn().mockReturnThis(),
    strength: vi.fn().mockReturnThis(),
    links: vi.fn().mockReturnThis(),
  })),
  forceManyBody: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
  })),
  forceCenter: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
  })),
  forceX: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
    x: vi.fn().mockReturnThis(),
  })),
  forceY: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
    y: vi.fn().mockReturnThis(),
  })),
  forceCollide: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
  })),
}))

vi.mock('d3-zoom', () => ({
  zoom: vi.fn(() => {
    const zoomBehavior = vi.fn()
    zoomBehavior.scaleExtent = vi.fn().mockReturnThis()
    zoomBehavior.on = vi.fn().mockReturnThis()
    zoomBehavior.transform = vi.fn()
    return zoomBehavior
  }),
  zoomIdentity: {},
}))

vi.mock('d3-drag', () => ({
  drag: vi.fn(() => {
    const dragBehavior = vi.fn()
    dragBehavior.on = vi.fn().mockReturnThis()
    return dragBehavior
  }),
}))

vi.mock('d3-transition', () => ({}))

describe('GraphView', () => {
  let wrapper
  let resizeCallback
  const mockGraphData = {
    nodes: [
      { id: '1', label: 'Note 1', val: 5, node_kind: 'note' },
      { id: '2', label: 'Hub: Topic', val: 1, node_kind: 'topic_hub', note_type: 'hub' },
    ],
    links: [
      { source: '1', target: '2', edge_kind: 'topic_membership' }
    ]
  }

  beforeEach(() => {
    vi.clearAllMocks()
    setActivePinia(createPinia())
    const bootStore = useBootStore()
    bootStore.setStatus({
      phase: 'ready',
      message: 'Grafyn is ready',
      ready: true,
      error: null,
    })
    vi.spyOn(apiClient.graph, 'full').mockResolvedValue(mockGraphData)

    // Mock ResizeObserver, capturing the callback so tests can simulate a resize
    resizeCallback = null
    global.ResizeObserver = class ResizeObserver {
      constructor(cb) {
        resizeCallback = cb
      }
      observe = vi.fn()
      unobserve = vi.fn()
      disconnect = vi.fn()
    }
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
  })

  // ============================================================================
  // Rendering & Data Loading
  // ============================================================================

  it('renders and loads graph data on mount', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    // Loading finished, data loaded
    expect(apiClient.graph.full).toHaveBeenCalledTimes(1)

    // Stats rendered
    expect(wrapper.find('.toolbar-stats').text()).toContain('2 Nodes')
    expect(wrapper.find('.toolbar-stats').text()).toContain('1 Edges')
  })

  it('waits for boot readiness before loading graph data', async () => {
    const bootStore = useBootStore()
    bootStore.setStatus({
      phase: 'building_search_index',
      message: 'Building search index',
      ready: false,
      error: null,
    })

    wrapper = mount(GraphView)
    await flushPromises()

    expect(apiClient.graph.full).not.toHaveBeenCalled()
    expect(wrapper.find('.loading-overlay').text()).toContain('Preparing graph...')

    bootStore.setStatus({
      phase: 'ready',
      message: 'Grafyn is ready',
      ready: true,
      error: null,
    })
    await flushPromises()

    expect(apiClient.graph.full).toHaveBeenCalledTimes(1)
  })

  it('initializes D3 simulation', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(forceSimulation).toHaveBeenCalled()
  })

  it('stops the previous simulation before creating a new one on refresh', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const firstSim = forceSimulation.mock.results[0].value

    const refreshBtn = wrapper.find('[title="Refresh Graph"]')
    await refreshBtn.trigger('click')
    await flushPromises()

    expect(forceSimulation).toHaveBeenCalledTimes(2)
    expect(firstSim.stop).toHaveBeenCalled()
  })

  it('stops the simulation on unmount', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const sim = forceSimulation.mock.results[0].value
    wrapper.unmount()
    wrapper = null

    expect(sim.stop).toHaveBeenCalled()
  })

  it('updates existing forceX/forceY centers on resize instead of injecting forceCenter', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const sim = forceSimulation.mock.results[0].value
    const xForce = sim.force('x')
    const yForce = sim.force('y')
    expect(xForce).toBeTruthy()
    expect(yForce).toBeTruthy()

    resizeCallback([{ contentRect: { width: 1000, height: 700 } }])
    await flushPromises()

    expect(forceCenter).not.toHaveBeenCalled()
    expect(xForce.x).toHaveBeenCalledWith(500)
    expect(yForce.y).toHaveBeenCalledWith(350)
    expect(sim.restart).toHaveBeenCalled()
  })

  // ============================================================================
  // Error handling
  // ============================================================================

  it('shows an inline error state when the graph fails to load', async () => {
    apiClient.graph.full.mockRejectedValueOnce(new Error('vault read failed'))

    wrapper = mount(GraphView)
    await flushPromises()

    expect(wrapper.find('.graph-error-state').exists()).toBe(true)
    expect(wrapper.text()).toContain('vault read failed')
    // Should not simultaneously claim the vault is merely empty
    expect(wrapper.find('.graph-empty-state').exists()).toBe(false)
  })

  it('clears the error state after a successful retry', async () => {
    apiClient.graph.full.mockRejectedValueOnce(new Error('vault read failed'))

    wrapper = mount(GraphView)
    await flushPromises()
    expect(wrapper.find('.graph-error-state').exists()).toBe(true)

    apiClient.graph.full.mockResolvedValueOnce(mockGraphData)
    const retryBtn = wrapper.find('.graph-error-state button')
    await retryBtn.trigger('click')
    await flushPromises()

    expect(wrapper.find('.graph-error-state').exists()).toBe(false)
  })

  // ============================================================================
  // Interactions
  // ============================================================================

  it('refreshes graph when refresh button clicked', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const callsBefore = apiClient.graph.full.mock.calls.length

    const refreshBtn = wrapper.find('[title="Refresh Graph"]')
    await refreshBtn.trigger('click')
    await flushPromises()

    expect(apiClient.graph.full).toHaveBeenCalledTimes(callsBefore + 1)
  })

  it('renders graph settings panel', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(wrapper.find('.graph-settings-stub').exists()).toBe(true)
  })

  it('renders toolbar with stats', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(wrapper.find('.graph-toolbar').exists()).toBe(true)
    expect(wrapper.find('.toolbar-actions').exists()).toBe(true)
  })

  it('uses bounded structural distances instead of treating every relationship alike', async () => {
    wrapper = mount(GraphView)
    await flushPromises()
    const distance = forceLink.mock.results[0].value.distance.mock.calls[0][0]
    expect(distance({ edge_kind: 'topic_membership' })).toBe(70)
    expect(distance({ edge_kind: 'note_link' })).toBe(100)
    expect(distance({ edge_kind: 'topic_related' })).toBe(160)
  })

  it('reveals a clicked note and its local edge at overview zoom', async () => {
    wrapper = mount(GraphView)
    await flushPromises()
    const click = mockSelection.on.mock.calls.find(([name]) => name === 'click')[1]
    click({ stopPropagation: vi.fn() }, { id: '1', label: 'Note 1', node_kind: 'note' })
    const labelVisibility = labelSelection.style.mock.calls.filter(([name]) => name === 'visibility').at(-1)[1]
    const linkVisibility = linkSelection.style.mock.calls.filter(([name]) => name === 'visibility').at(-1)[1]
    expect(labelVisibility({ id: '1', node_kind: 'note' })).toBe('visible')
    expect(linkVisibility({ source: '1', target: '2', edge_kind: 'note_link' })).toBe('visible')
    expect(wrapper.emitted('node-click')?.[0]).toEqual(['1'])
  })

  it('keeps hub labels and note circles at overview, then reveals note labels and links', async () => {
    wrapper = mount(GraphView)
    await flushPromises()
    const labelVisibility = () => labelSelection.style.mock.calls
      .filter(([key]) => key === 'visibility').at(-1)[1]
    const zoomHandler = d3Zoom.mock.results[0].value.on.mock.calls.find(([kind]) => kind === 'zoom')[1]
    const overview = labelVisibility()
    expect(overview({ id: '2', node_kind: 'topic_hub' })).toBe('visible')
    expect(overview({ id: '1', node_kind: 'note' })).toBe('hidden')
    expect(circleSelection.style).toHaveBeenCalled()
    const overviewEdges = linkSelection.style.mock.calls.filter(([key]) => key === 'visibility').at(-1)[1]
    expect(overviewEdges({ source: '1', target: '2', edge_kind: 'note_link' })).toBe('hidden')

    zoomHandler({ transform: { k: 2 } })
    expect(labelVisibility()({ id: '1', node_kind: 'note' })).toBe('visible')
    const detailedEdges = linkSelection.style.mock.calls.filter(([key]) => key === 'visibility').at(-1)[1]
    expect(detailedEdges({ source: '1', target: '2', edge_kind: 'note_link' })).toBe('visible')
  })

  it('shows the sourced perspective lineage at the selected time slice', async () => {
    const initial = { id: 's1', perspective_id: 'p1', title: 'Design', statement: 'Optimise first',
      change_kind: 'initial', reason: 'Early view', source_note_ids: ['1'], previous_state_id: null,
      effective_at: '2025-01-01T12:00:00Z' }
    const revised = { ...initial, id: 's2', statement: 'Explore first', change_kind: 'revised',
      previous_state_id: 's1', effective_at: '2026-01-01T12:00:00Z' }
    vi.spyOn(apiClient.graph, 'perspectiveStates').mockResolvedValue([initial, revised])
    wrapper = mount(GraphView)
    await flushPromises()
    await wrapper.find('button[aria-pressed="false"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('.lineage').text()).toContain('Explore first')
    expect(wrapper.find('.lineage').text()).toContain('Optimise first')
    expect(wrapper.find('input[aria-label="Time slice"]').exists()).toBe(true)
    await wrapper.find('input[aria-label="Time slice"]').setValue('0')
    expect(wrapper.find('.lineage').text()).toContain('Optimise first')
    expect(wrapper.find('.lineage').text()).not.toContain('Explore first')
    const labelVisibility = labelSelection.style.mock.calls.filter(([key]) => key === 'visibility').at(-1)[1]
    expect(labelVisibility({ id: '1', node_kind: 'note' })).toBe('visible')
  })

  it('records an explicit next state linked to its parent and selected source note', async () => {
    const initial = { id: 's1', perspective_id: 'p1', title: 'Design', statement: 'Optimise first',
      change_kind: 'initial', reason: 'Early view', source_note_ids: ['1'], previous_state_id: null,
      effective_at: '2025-01-01T12:00:00Z' }
    vi.spyOn(apiClient.graph, 'perspectiveStates').mockResolvedValue([initial])
    const save = vi.spyOn(apiClient.graph, 'recordPerspectiveState').mockResolvedValue({
      ...initial, id: 's2', statement: 'Explore first', previous_state_id: 's1', change_kind: 'revised',
      effective_at: '2026-09-24T12:00:00Z'
    })
    wrapper = mount(GraphView)
    await flushPromises()
    await wrapper.find('button[aria-pressed="false"]').trigger('click')
    await flushPromises()
    await wrapper.find('select[required]').setValue('1')
    await wrapper.find('textarea[required]').setValue('Explore first')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(save).toHaveBeenCalledWith(expect.objectContaining({
      perspective_id: 'p1', previous_state_id: 's1', source_note_ids: ['1'],
      statement: 'Explore first', change_kind: 'revised'
    }))
    expect(wrapper.find('.lineage').text()).toContain('Explore first')
  })
})
