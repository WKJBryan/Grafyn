import { flushPromises, mount } from '@vue/test-utils'
import { ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

const state = vi.hoisted(() => ({ compact: false }))

vi.mock('@/composables/useCompanionLayout', () => ({
  useCompanionLayout: () => ({ isCompact: ref(state.compact) }),
}))

vi.mock('@/views/CanvasView.vue', () => ({
  default: { template: '<div data-d3-canvas>Spatial Canvas</div>' },
}))
vi.mock('@/views/companion/CanvasCompanionView.vue', () => ({
  default: { template: '<div data-linear-canvas>Linear Canvas</div>' },
}))
vi.mock('@/views/TwinReviewView.vue', () => ({
  default: { template: '<div data-wide-twin>Wide Twin</div>' },
}))
vi.mock('@/views/companion/TwinCompanionView.vue', () => ({
  default: { template: '<div data-compact-twin>Compact Twin</div>' },
}))

import ResponsiveCanvasView from '@/views/ResponsiveCanvasView.vue'
import ResponsiveTwinView from '@/views/ResponsiveTwinView.vue'

describe('responsive Twin and Canvas views', () => {
  afterEach(() => vi.restoreAllMocks())

  it('mounts linear Canvas at compact width without mounting the D3 spatial surface', async () => {
    state.compact = true
    const wrapper = mount(ResponsiveCanvasView)
    await flushPromises()

    expect(wrapper.find('[data-linear-canvas]').exists()).toBe(true)
    expect(wrapper.find('[data-d3-canvas]').exists()).toBe(false)
  })

  it('preserves the existing spatial Canvas and Twin workspace at wide width', async () => {
    state.compact = false
    const canvas = mount(ResponsiveCanvasView)
    const twin = mount(ResponsiveTwinView)
    await flushPromises()

    expect(canvas.find('[data-d3-canvas]').exists()).toBe(true)
    expect(canvas.find('[data-linear-canvas]').exists()).toBe(false)
    expect(twin.find('[data-wide-twin]').exists()).toBe(true)
    expect(twin.find('[data-compact-twin]').exists()).toBe(false)
  })
})
