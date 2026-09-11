import { mount } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import { describe, expect, it, vi } from 'vitest'
import { RUNTIME_PROFILES } from '@/platform/runtime'
import { useCompanionLayout } from '@/composables/useCompanionLayout'

function mediaQuery(initialMatches = false) {
  let matches = initialMatches
  const listeners = new Set()
  return {
    get matches() {
      return matches
    },
    addEventListener: vi.fn((_name, listener) => listeners.add(listener)),
    removeEventListener: vi.fn((_name, listener) => listeners.delete(listener)),
    setMatches(next) {
      matches = next
      for (const listener of listeners) listener({ matches: next })
    },
  }
}

function mountLayout(profile, query) {
  return mount(defineComponent({
    setup() {
      return useCompanionLayout({
        profile,
        matchMedia: () => query,
      })
    },
    template: '<div :data-compact="String(isCompact)" :data-wide="String(isWide)" />',
  }))
}

describe('useCompanionLayout', () => {
  it('keeps Android compact even when the viewport is wide', () => {
    const query = mediaQuery(false)
    const wrapper = mountLayout({ name: RUNTIME_PROFILES.ANDROID_COMPACT }, query)

    expect(wrapper.attributes('data-compact')).toBe('true')
    expect(wrapper.attributes('data-wide')).toBe('false')
    wrapper.unmount()
  })

  it('reacts to a narrow viewport for a desktop runtime', async () => {
    const query = mediaQuery(false)
    const wrapper = mountLayout({ name: RUNTIME_PROFILES.DESKTOP_WIDE }, query)

    expect(wrapper.attributes('data-compact')).toBe('false')
    query.setMatches(true)
    await nextTick()

    expect(wrapper.attributes('data-compact')).toBe('true')
    wrapper.unmount()
  })

  it('removes its media-query listener when the consumer unmounts', () => {
    const query = mediaQuery(false)
    const wrapper = mountLayout({ name: RUNTIME_PROFILES.DESKTOP_WIDE }, query)

    wrapper.unmount()

    expect(query.removeEventListener).toHaveBeenCalledWith('change', expect.any(Function))
  })
})
