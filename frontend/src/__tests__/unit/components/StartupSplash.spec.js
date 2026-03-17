import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import StartupSplash from '@/components/StartupSplash.vue'

describe('StartupSplash', () => {
  it('renders startup message and animated progress for non-error states', () => {
    const wrapper = mount(StartupSplash, {
      props: {
        status: {
          phase: 'building_indices',
          message: 'Building graph and search index',
          ready: false,
          error: null,
        },
      },
    })

    expect(wrapper.text()).toContain('Loading your knowledge workspace')
    expect(wrapper.text()).toContain('Building graph and search index')
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
})
