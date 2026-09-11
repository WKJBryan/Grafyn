<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div
        v-if="visible"
        class="confirm-overlay"
        @click.self="handleCancel"
      >
        <div
          ref="dialog"
          class="confirm-dialog"
          role="alertdialog"
          aria-modal="true"
          :aria-labelledby="titleId"
          :aria-describedby="messageId"
          tabindex="-1"
        >
          <div class="confirm-header">
            <span class="confirm-icon">{{ icon }}</span>
            <h3 :id="titleId">
              {{ title }}
            </h3>
          </div>
          <p
            :id="messageId"
            class="confirm-message"
          >
            {{ message }}
          </p>
          <div class="confirm-actions">
            <button
              ref="cancelButton"
              type="button"
              class="btn btn-secondary"
              @click="handleCancel"
            >
              {{ cancelLabel }}
            </button>
            <button
              type="button"
              class="btn"
              :class="confirmClass"
              @click="handleConfirm"
            >
              {{ confirmLabel }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { computed, getCurrentInstance, nextTick, onBeforeUnmount, ref, watch } from 'vue'

const props = defineProps({
  visible: { type: Boolean, default: false },
  title: { type: String, default: 'Confirm' },
  message: { type: String, default: 'Are you sure?' },
  confirmLabel: { type: String, default: 'Confirm' },
  cancelLabel: { type: String, default: 'Cancel' },
  variant: { type: String, default: 'danger' },
})

const emit = defineEmits(['confirm', 'cancel'])
const instanceId = getCurrentInstance()?.uid ?? 'fallback'
const titleId = `confirm-dialog-title-${instanceId}`
const messageId = `confirm-dialog-message-${instanceId}`
const dialog = ref(null)
const cancelButton = ref(null)
let opener = null
let listening = false

const icon = computed(() => {
  const icons = { danger: '\u26A0', warning: '\u26A0', info: '\u2139' }
  return icons[props.variant] || icons.danger
})

const confirmClass = computed(() => {
  const classes = {
    danger: 'btn-danger',
    warning: 'btn-warning',
    info: 'btn-primary',
  }
  return classes[props.variant] || 'btn-danger'
})

watch(() => props.visible, async visible => {
  if (visible) {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    startListening()
    await nextTick()
    cancelButton.value?.focus()
    return
  }
  stopListening()
  await nextTick()
  restoreFocus()
}, { immediate: true })

onBeforeUnmount(() => {
  stopListening()
  restoreFocus()
})

function startListening() {
  if (listening) return
  window.addEventListener('keydown', handleKeydown, true)
  listening = true
}

function stopListening() {
  if (!listening) return
  window.removeEventListener('keydown', handleKeydown, true)
  listening = false
}

function restoreFocus() {
  if (opener?.isConnected) opener.focus()
  opener = null
}

function handleKeydown(event) {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    handleCancel()
    return
  }
  if (event.key === 'Tab') trapFocus(event)
}

function trapFocus(event) {
  const controls = [...(dialog.value?.querySelectorAll(
    'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  ) || [])]
  event.stopImmediatePropagation()
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

function handleConfirm() {
  emit('confirm')
}

function handleCancel() {
  emit('cancel')
}
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10000;
  backdrop-filter: blur(2px);
}

.confirm-dialog {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  padding: var(--spacing-lg);
  max-width: 400px;
  width: 90%;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
}

.confirm-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-md);
}

.confirm-icon {
  font-size: 1.25rem;
}

.confirm-header h3 {
  margin: 0;
  font-size: 1.1rem;
  color: var(--text-primary);
}

.confirm-message {
  color: var(--text-secondary);
  font-size: 0.9rem;
  line-height: 1.5;
  margin-bottom: var(--spacing-lg);
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
}

.btn {
  min-height: 44px;
  padding: var(--spacing-sm) var(--spacing-md);
  border: none;
  border-radius: var(--radius-md);
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.btn-secondary:hover {
  background: var(--bg-hover);
}

.btn-danger {
  background: var(--accent-danger);
  color: white;
}

.btn-danger:hover {
  opacity: 0.9;
}

.btn-warning {
  background: var(--accent-warning);
  color: white;
}

.btn-primary {
  background: var(--accent-primary);
  color: white;
}

/* Transitions */
.confirm-fade-enter-active {
  transition: opacity 0.15s ease;
}
.confirm-fade-enter-active .confirm-dialog {
  animation: confirmSlideIn 0.2s ease-out;
}

.confirm-fade-leave-active {
  transition: opacity 0.1s ease;
}

.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}

@keyframes confirmSlideIn {
  from {
    transform: scale(0.95) translateY(-10px);
    opacity: 0;
  }
  to {
    transform: scale(1) translateY(0);
    opacity: 1;
  }
}
</style>
