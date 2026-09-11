import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import ConfirmDialog from '@/components/ConfirmDialog.vue'

const wrappers = []

async function mountDialog(props = {}) {
  const wrapper = mount(ConfirmDialog, {
    attachTo: document.body,
    props: {
      visible: true,
      title: 'Discard preview',
      message: 'Discard this unsaved preview?',
      ...props,
    },
  })
  wrappers.push(wrapper)
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('ConfirmDialog', () => {
  afterEach(() => {
    wrappers.splice(0).forEach(wrapper => wrapper.unmount())
    document.body.innerHTML = ''
  })

  it('announces itself as a modal alert and places focus on the safe action', async () => {
    const wrapper = await mountDialog()
    const dialog = document.querySelector('[role="alertdialog"]')
    const cancel = document.querySelector('.confirm-actions .btn-secondary')

    expect(dialog?.getAttribute('aria-modal')).toBe('true')
    expect(document.getElementById(dialog?.getAttribute('aria-labelledby'))?.textContent).toContain('Discard preview')
    expect(document.getElementById(dialog?.getAttribute('aria-describedby'))?.textContent).toContain('Discard this unsaved preview?')
    expect(document.activeElement).toBe(cancel)
    expect(wrapper.emitted('cancel')).toBeUndefined()
  })

  it('traps forward and reverse Tab navigation inside the confirmation', async () => {
    await mountDialog()
    const cancel = document.querySelector('.confirm-actions .btn-secondary')
    const confirm = document.querySelector('.confirm-actions .btn-danger')

    confirm.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab',
      bubbles: true,
      cancelable: true,
    }))
    expect(document.activeElement).toBe(cancel)

    cancel.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab',
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    }))
    expect(document.activeElement).toBe(confirm)
  })

  it('cancels on Escape and restores the opener when hidden', async () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const wrapper = await mountDialog()

    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
      cancelable: true,
    }))
    expect(wrapper.emitted('cancel')).toHaveLength(1)

    await wrapper.setProps({ visible: false })
    await wrapper.vm.$nextTick()
    expect(document.activeElement).toBe(opener)
  })
})
