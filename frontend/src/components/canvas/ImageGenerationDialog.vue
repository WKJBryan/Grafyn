<template>
  <Teleport to="body">
    <div
      class="image-dialog-overlay"
      @click.self="requestClose"
    >
      <section
        ref="dialog"
        class="image-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="image-generation-title"
        tabindex="-1"
      >
        <header class="dialog-header">
          <div>
            <span class="eyebrow">Canvas studio</span>
            <h1 id="image-generation-title">
              Generate an image
            </h1>
          </div>
          <button
            type="button"
            aria-label="Close image generation"
            @click="requestClose"
          >
            Close
          </button>
        </header>
        <QuickImageComposer @dirty-change="dirty = $event" />
      </section>
    </div>
  </Teleport>

  <ConfirmDialog
    :visible="confirmClose"
    title="Discard image draft"
    message="Close and discard this image prompt or current preview?"
    confirm-label="Discard and close"
    cancel-label="Keep working"
    variant="warning"
    @confirm="confirmDiscard"
    @cancel="cancelDiscard"
  />
</template>

<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import QuickImageComposer from '@/components/companion/QuickImageComposer.vue'

const emit = defineEmits(['close'])
const dialog = ref(null)
const dirty = ref(false)
const confirmClose = ref(false)
let opener = null

onMounted(async () => {
  opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
  window.addEventListener('keydown', handleKeydown)
  await nextTick()
  dialog.value?.focus()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeydown)
  if (opener?.isConnected) opener.focus()
})

function handleKeydown(event) {
  if (confirmClose.value) return
  if (event.key === 'Escape') {
    requestClose()
    return
  }
  if (event.key === 'Tab') trapFocus(event)
}

function trapFocus(event) {
  const controls = [...(dialog.value?.querySelectorAll(
    'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  ) || [])]
  if (!controls.length) {
    event.preventDefault()
    dialog.value?.focus()
    return
  }
  const first = controls[0]
  const last = controls.at(-1)
  const focusOutside = !dialog.value?.contains(document.activeElement)
  if (event.shiftKey && (document.activeElement === first || focusOutside)) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && (document.activeElement === last || focusOutside)) {
    event.preventDefault()
    first.focus()
  }
}

function requestClose() {
  if (dirty.value) {
    confirmClose.value = true
    return
  }
  emit('close')
}

function confirmDiscard() {
  confirmClose.value = false
  emit('close')
}

async function cancelDiscard() {
  confirmClose.value = false
  await nextTick()
  dialog.value?.focus()
}
</script>

<style scoped>
.image-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: grid;
  place-items: center;
  padding: var(--spacing-lg);
  background: rgba(4, 7, 10, 0.78);
  backdrop-filter: blur(4px);
}

.image-dialog {
  width: min(58rem, 100%);
  max-height: calc(100vh - 2 * var(--spacing-lg));
  overflow: auto;
  padding: var(--spacing-lg);
  color: var(--text-primary);
  background: var(--bg-primary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  box-shadow: 0 24px 72px rgba(0, 0, 0, 0.45);
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-md);
}

.dialog-header h1 {
  margin: 0;
  font-size: 1.35rem;
}

.eyebrow {
  color: var(--text-muted);
  font-size: 0.7rem;
  font-weight: 700;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.dialog-header button {
  min-height: 44px;
  padding: 0 var(--spacing-md);
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.dialog-header button:focus-visible,
.image-dialog:focus-visible {
  outline: 2px solid var(--accent-cyan);
  outline-offset: 2px;
}

@media (max-width: 42rem) {
  .image-dialog-overlay {
    padding: 0;
  }

  .image-dialog {
    width: 100%;
    height: 100%;
    max-height: none;
    border: 0;
    border-radius: 0;
  }
}
</style>
