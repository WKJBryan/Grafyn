import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import StartupSplash from '@/components/StartupSplash.vue'

describe('StartupSplash', () => {
  it('renders startup message and animated progress for non-error states', () => {
    const wrapper = mount(StartupSplash, {
      props: {
        status: {
          phase: 'building_search_index',
          message: 'Building search index',
          ready: false,
          error: null,
        },
      },
    })

    expect(wrapper.text()).toContain('Loading your knowledge workspace')
    expect(wrapper.text()).toContain('Building search index')
    expect(wrapper.find('.startup-progress-track').exists()).toBe(true)
  })

  it('shows a continue button when startup fails', async () => {
    const wrapper = mount(StartupSplash, {
      props: {
        status: {
          phase: 'failed',
          message: 'Startup failed',
          ready: false,
          error: 'disk error',
        },
      },
    })

    await wrapper.find('button').trigger('click')

    expect(wrapper.text()).toContain('Startup hit a problem')
    expect(wrapper.emitted('dismiss')).toBeTruthy()
  })

  it('shows timeout recovery copy when startup takes too long', () => {
    const wrapper = mount(StartupSplash, {
      props: {
        status: {
          phase: 'building_chunk_index',
          message: 'Building chunk index',
          ready: false,
          error: 'Startup is taking longer than expected while building the chunk index. You can continue to the app, but indexing may not be fully ready yet.',
        },
      },
    })

    expect(wrapper.text()).toContain('Continue to app')
    expect(wrapper.text()).toContain('taking longer than expected')
    expect(wrapper.text()).toContain('indexing may not be fully ready yet')
  })
})
