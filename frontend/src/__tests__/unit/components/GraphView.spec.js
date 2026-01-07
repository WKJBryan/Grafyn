import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import GraphView from '@/components/GraphView.vue'
import * as apiClient from '@/api/client'
import * as d3 from 'd3'

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
  remove: vi.fn().mockReturnThis(), // Added remove just in case
}

vi.mock('d3', () => {
  return {
    select: vi.fn(() => mockSelection),
    selectAll: vi.fn(() => mockSelection),
    forceSimulation: vi.fn(() => ({
      force: vi.fn().mockReturnThis(),
      on: vi.fn().mockReturnThis(),
      stop: vi.fn(),
      alpha: vi.fn().mockReturnThis(),
      alphaTarget: vi.fn().mockReturnThis(),
      restart: vi.fn().mockReturnThis(),
      nodes: vi.fn().mockReturnThis(), // Sometimes used
    })),
    forceLink: vi.fn(() => ({
      id: vi.fn().mockReturnThis(),
      distance: vi.fn().mockReturnThis(),
      links: vi.fn().mockReturnThis(), // Sometimes used
    })),
    forceManyBody: vi.fn(() => ({
      strength: vi.fn().mockReturnThis(),
    })),
    forceCenter: vi.fn(),
    forceCollide: vi.fn(() => ({
      strength: vi.fn().mockReturnThis(),
    })),
    zoom: vi.fn(() => {
      const zoomBehavior = vi.fn()
      zoomBehavior.scaleExtent = vi.fn().mockReturnThis()
      zoomBehavior.on = vi.fn().mockReturnThis()
      zoomBehavior.transform = vi.fn()
      return zoomBehavior
    }),
    zoomIdentity: {},
    drag: vi.fn(() => {
      const dragBehavior = vi.fn()
      dragBehavior.on = vi.fn().mockReturnThis()
      return dragBehavior
    }),
  }
})

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

    // Initially loading
    expect(wrapper.find('.loading-overlay').exists()).toBe(true)

    await flushPromises()

    // Loading finished
    expect(wrapper.find('.loading-overlay').exists()).toBe(false)
    expect(apiClient.graph.full).toHaveBeenCalledTimes(1)

    // Stats rendered
    expect(wrapper.find('.toolbar-stats').text()).toContain('2 Notes')
    expect(wrapper.find('.toolbar-stats').text()).toContain('1 Links')
  })

  it('initializes D3 simulation', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(d3.forceSimulation).toHaveBeenCalled()
    // It should strip nodes from data and pass to simulation
    // We can't easily check args due to cloning, but we know it was called
  })

  // ============================================================================
  // Interactions
  // ============================================================================

  it('refreshes graph when refresh button clicked', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    apiClient.graph.full.mockClear()

    // Find refresh button (first button in actions)
    // We added a color picker div 1st, so buttons are 2nd and 3rd children of .toolbar-actions?
    // Actually .btn-secondary
    const refreshBtn = wrapper.findAll('.btn-secondary')[0]
    await refreshBtn.trigger('click')

    expect(wrapper.find('.loading-overlay').exists()).toBe(true)
    await flushPromises()
    expect(apiClient.graph.full).toHaveBeenCalledTimes(1)
  })

  // ============================================================================
  // Color Picker Feature
  // ============================================================================

  it('renders color picker', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    expect(wrapper.find('.color-picker-wrapper').exists()).toBe(true)
    expect(wrapper.find('input[type="color"]').exists()).toBe(true)
  })

  it('updates color state when input changes', async () => {
    wrapper = mount(GraphView)
    await flushPromises()

    const colorInput = wrapper.find('input[type="color"]')
    await colorInput.setValue('#ff0000') // Set to red

    expect(wrapper.vm.userColor).toBe('#ff0000')

    // Verify that d3.selectAll was called to update attributes
    // updateNodeColors calls svg.selectAll('circle').attr('fill', ...)
    // Our mock: selectAll -> mockSelection; attr -> mockSelection
    // Since we cleared mocks in beforeEach, d3 selection calls from initGraph are cleared?
    // No, clearAllMocks clears call history.

    // We need to check if attr was called with 'fill' and '#ff0000'
    // But initGraph also calls attr('fill', ...).
    // So we check the LAST call or if it was called with specific args after setValue.

    expect(mockSelection.selectAll).toHaveBeenCalledWith('circle')
    // mockSelection.attr is called multiple times.

    const attrCalls = mockSelection.attr.mock.calls
    // Look for call ['fill', '#ff0000']
    const colorUpdateCall = attrCalls.find(call => call[0] === 'fill' && call[1] === '#ff0000')
    expect(colorUpdateCall).toBeTruthy()
  })
})
