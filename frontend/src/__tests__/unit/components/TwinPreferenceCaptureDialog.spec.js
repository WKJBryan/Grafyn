import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import TwinPreferenceCaptureDialog from '@/components/companion/TwinPreferenceCaptureDialog.vue'

const wrappers = []

function mountDialog(props = {}) {
  const wrapper = mount(TwinPreferenceCaptureDialog, {
    attachTo: document.body,
    props: { visible: true, ...props },
  })
  wrappers.push(wrapper)
  return wrapper
}

afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount())
  document.body.innerHTML = ''
})

describe('TwinPreferenceCaptureDialog', () => {
  it('starts blank, explains the evidence threshold, and requires an explicit claim', async () => {
    const wrapper = mountDialog()
    await nextTick()

    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.attributes('aria-modal')).toBe('true')
    expect(dialog.text()).toContain('This records evidence, not memory.')
    expect(dialog.text()).toContain('three matching captures from three different responses')
    expect(dialog.text()).toContain('review is still required')
    expect(wrapper.get('[aria-label="Twin preference evidence"]').element.value).toBe('')
    expect(document.activeElement).toBe(wrapper.get('[aria-label="Twin preference evidence"]').element)

    await wrapper.get('form').trigger('submit')

    expect(wrapper.emitted('submit')).toBeUndefined()
    expect(wrapper.get('[role="alert"]').text()).toContain('Enter a preference to capture')
  })

  it('submits only trimmed user-authored text and keeps touch-safe controls', async () => {
    const wrapper = mountDialog()
    const textarea = wrapper.get('[aria-label="Twin preference evidence"]')
    await textarea.setValue('  I prefer concrete implementation details.  ')
    await wrapper.get('form').trigger('submit')

    expect(wrapper.emitted('submit')).toEqual([['I prefer concrete implementation details.']])
    expect(wrapper.get('[data-test="cancel-twin-preference"]').classes()).toContain('touch-action')
    expect(wrapper.get('[data-test="submit-twin-preference"]').classes()).toContain('touch-action')
  })

  it('traps focus, closes with Escape, and restores the opener', async () => {
    const opener = document.createElement('button')
    document.body.appendChild(opener)
    opener.focus()
    const wrapper = mountDialog({ visible: false })

    await wrapper.setProps({ visible: true })
    await nextTick()
    const textarea = wrapper.get('[aria-label="Twin preference evidence"]')
    const submit = wrapper.get('[data-test="submit-twin-preference"]')
    submit.element.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }))
    expect(document.activeElement).toBe(textarea.element)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(wrapper.emitted('cancel')).toHaveLength(1)
    await wrapper.setProps({ visible: false })
    await nextTick()
    expect(document.activeElement).toBe(opener)
  })

  it('keeps an honest Close action available while recording', async () => {
    const wrapper = mountDialog({ submitting: true })
    await nextTick()

    const close = wrapper.get('[data-test="cancel-twin-preference"]')
    expect(close.text()).toBe('Close')
    expect(close.attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).toContain('Closing this dialog does not cancel recording.')

    await close.trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(1)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(wrapper.emitted('cancel')).toHaveLength(2)
  })
})
