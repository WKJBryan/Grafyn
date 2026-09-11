import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import ImageGenerationDialog from '@/components/canvas/ImageGenerationDialog.vue'
import QuickImageComposer from '@/components/companion/QuickImageComposer.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'

function mountDialog() {
  return mount(ImageGenerationDialog, {
    attachTo: document.body,
    global: {
      stubs: {
        QuickImageComposer: {
          template: '<div class="quick-image-stub"><button class="first-image-control">First</button><button class="last-image-control">Last</button></div>',
          emits: ['dirty-change'],
        },
      },
    },
  })
}

describe('ImageGenerationDialog', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('uses an accessible focused dialog and closes a clean draft from the keyboard', async () => {
    const wrapper = mountDialog()
    await wrapper.vm.$nextTick()

    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.attributes('aria-modal')).toBe('true')
    expect(dialog.attributes('aria-labelledby')).toBe('image-generation-title')
    expect(document.activeElement).toBe(dialog.element)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('warns before closing a dirty prompt or preview and respects cancel or discard', async () => {
    const wrapper = mountDialog()
    wrapper.getComponent(QuickImageComposer).vm.$emit('dirty-change', true)
    await wrapper.vm.$nextTick()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeUndefined()
    expect(wrapper.getComponent(ConfirmDialog).props('visible')).toBe(true)

    wrapper.getComponent(ConfirmDialog).vm.$emit('cancel')
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeUndefined()

    await wrapper.get('[aria-label="Close image generation"]').trigger('click')
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('moves focus into the dirty-close confirmation and restores it after Escape', async () => {
    const wrapper = mountDialog()
    await wrapper.vm.$nextTick()
    wrapper.getComponent(QuickImageComposer).vm.$emit('dirty-change', true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    const confirmation = document.querySelector('[role="alertdialog"]')
    expect(confirmation?.contains(document.activeElement)).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
      cancelable: true,
    }))
    await wrapper.vm.$nextTick()
    expect(document.querySelector('[role="alertdialog"]')).toBeNull()
    expect(document.activeElement).toBe(document.querySelector('[role="dialog"]'))
    wrapper.unmount()
  })

  it('traps forward and reverse Tab navigation inside the image dialog', async () => {
    const wrapper = mountDialog()
    await wrapper.vm.$nextTick()
    const dialog = wrapper.get('[role="dialog"]')
    const close = wrapper.get('[aria-label="Close image generation"]')
    const last = wrapper.get('.last-image-control')

    last.element.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab',
      bubbles: true,
      cancelable: true,
    }))
    expect(document.activeElement).toBe(close.element)

    close.element.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab',
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    }))
    expect(document.activeElement).toBe(last.element)
    expect(dialog.element.contains(document.activeElement)).toBe(true)
    wrapper.unmount()
  })

  it('restores focus to the opener after the dialog unmounts', async () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const wrapper = mountDialog()
    await wrapper.vm.$nextTick()

    wrapper.unmount()

    expect(document.activeElement).toBe(opener)
  })
})
