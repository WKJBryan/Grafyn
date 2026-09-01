import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import CapabilityUnavailable from '@/components/companion/CapabilityUnavailable.vue'

describe('CapabilityUnavailable', () => {
  it('names unavailable linear Canvas as Canvas', () => {
    const wrapper = mount(CapabilityUnavailable, {
      props: { capability: 'linearCanvas' },
    })

    expect(wrapper.get('h1').text()).toBe('Canvas')
  })
})
