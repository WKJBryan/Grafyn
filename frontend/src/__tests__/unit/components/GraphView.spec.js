import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import GraphView from '@/components/GraphView.vue'
import * as apiClient from '@/api/client'
import { forceSimulation } from 'd3-force'
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

vi.mock('d3-selection', () => ({
  select: vi.fn(() => mockSelection),
}))

vi.mock('d3-force', () => ({
  forceSimulation: vi.fn(() => ({
    force: vi.fn().mockReturnThis(),
    on: vi.fn().mockReturnThis(),
    stop: vi.fn(),
    alpha: vi.fn().mockReturnThis(),
    alphaTarget: vi.fn().mockReturnThis(),
    restart: vi.fn().mockReturnThis(),
    nodes: vi.fn().mockReturnThis(),
    alphaDecay: vi.fn().mockReturnThis(),
  })),
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
  })),
  forceY: vi.fn(() => ({
    strength: vi.fn().mockReturnThis(),
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
  const mockGraphData = {
    nodes: [
      { id: '1', label: 'Note 1', val: 5 },
      { id: '2', label: 'Note 2', val: 1 },
    ],
    links: [
      { source: '1', target: '2' }
    ]
  }

  beforeEach(() => {
    vi.clearAllMocks()
    vi.spyOn(apiClient.graph, 'full').mockResolvedValue(mockGraphData)

    // Mock ResizeObserver
    global.ResizeObserver = class ResizeObserver {
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
    expect(wrapper.find('.toolbar-stats').text()).toContain('2 Notes')
    expect(wrapper.find('.toolbar-stats').text()).toContain('1 Links')
  })

  it('initializes D3 simulation', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(forceSimulation).toHaveBeenCalled()
  })

  // ============================================================================
  // Interactions
  // ============================================================================

  it('refreshes graph when refresh button clicked', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const callsBefore = apiClient.graph.full.mock.calls.length

    const refreshBtn = wrapper.findAll('.btn-secondary')[0]
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
})
