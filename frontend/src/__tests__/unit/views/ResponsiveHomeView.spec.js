import { flushPromises, mount } from '@vue/test-utils'
import { ref } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import ResponsiveHomeView from '@/views/ResponsiveHomeView.vue'

const state = vi.hoisted(() => ({ compact: false, available: true }))

vi.mock('@/composables/useCompanionLayout', () => ({
  useCompanionLayout: () => ({ isCompact: ref(state.compact) }),
}))

vi.mock('@/platform/capabilities', () => ({
  hasCapability: () => state.available,
}))

vi.mock('@/views/HomeView.vue', () => ({
  default: { name: 'HomeView', template: '<div data-test="wide-home">Wide home</div>' },
}))

vi.mock('@/views/companion/CaptureView.vue', () => ({
  default: { name: 'CaptureView', template: '<div data-test="capture-home">Capture</div>' },
}))

vi.mock('@/components/companion/CapabilityUnavailable.vue', () => ({
  default: { name: 'CapabilityUnavailable', template: '<div data-test="unavailable">Unavailable</div>' },
}))

describe('ResponsiveHomeView', () => {
  beforeEach(() => {
    state.compact = false
    state.available = true
  })

  it('renders the existing HomeView without a compact wrapper when wide', async () => {
    const wrapper = mount(ResponsiveHomeView)
    await flushPromises()
    expect(wrapper.get('[data-test="wide-home"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="capture-home"]').exists()).toBe(false)
  })

  it('renders fast capture on a narrow desktop with note-write capability', async () => {
    state.compact = true
    const wrapper = mount(ResponsiveHomeView)
    await flushPromises()
    expect(wrapper.get('[data-test="capture-home"]').exists()).toBe(true)
  })

  it('renders an honest unavailable state when compact capture is not available', async () => {
    state.compact = true
    state.available = false
    const wrapper = mount(ResponsiveHomeView)
    await flushPromises()
    expect(wrapper.get('[data-test="unavailable"]').exists()).toBe(true)
  })
})
