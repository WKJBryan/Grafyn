<template>
  <div
    v-if="visible"
    class="capture-dialog-overlay"
    @click.self="handleCancel"
  >
    <section
      ref="dialog"
      class="capture-dialog"
      role="dialog"
      aria-modal="true"
      :aria-labelledby="titleId"
      :aria-describedby="descriptionId"
      tabindex="-1"
    >
      <header>
        <span class="eyebrow">Twin evidence</span>
        <h2 :id="titleId">
          Capture Twin preference
        </h2>
      </header>

      <p
        :id="descriptionId"
        class="explanation"
      >
        <strong>This records evidence, not memory.</strong>
        Grafyn needs three matching captures from three different responses before it may propose a Twin memory, and review is still required before a proposal becomes memory.
      </p>

      <form @submit.prevent="handleSubmit">
        <label for="twin-preference-evidence">What preference should Grafyn record as evidence?</label>
        <textarea
          id="twin-preference-evidence"
          ref="textarea"
          v-model="draft"
          aria-label="Twin preference evidence"
          rows="4"
          placeholder="Example: I prefer concrete implementation details."
          :disabled="submitting"
          @input="validationError = ''"
        />

        <p
          v-if="displayError"
          class="dialog-error"
          role="alert"
        >
          {{ displayError }}
        </p>

        <p
          v-if="submitting"
          class="recording-note"
        >
          Closing this dialog does not cancel recording.
        </p>

        <div class="dialog-actions">
          <button
            type="button"
            class="secondary touch-action"
            data-test="cancel-twin-preference"
            @click="handleCancel"
          >
            {{ submitting ? 'Close' : 'Cancel' }}
          </button>
          <button
            type="submit"
            class="primary touch-action"
            data-test="submit-twin-preference"
            :disabled="submitting"
          >
            {{ submitting ? 'Recording…' : 'Record evidence' }}
          </button>
        </div>
      </form>
    </section>
  </div>
</template>

<script setup>
import { computed, getCurrentInstance, nextTick, onBeforeUnmount, ref, watch } from 'vue'

const props = defineProps({
  visible: { type: Boolean, default: false },
  submitting: { type: Boolean, default: false },
  error: { type: String, default: '' },
})

const emit = defineEmits(['submit', 'cancel'])
const instanceId = getCurrentInstance()?.uid ?? 'fallback'
const titleId = `twin-preference-title-${instanceId}`
const descriptionId = `twin-preference-description-${instanceId}`
const dialog = ref(null)
const textarea = ref(null)
const draft = ref('')
const validationError = ref('')
const displayError = computed(() => validationError.value || props.error)
let opener = null
let listening = false

watch(() => props.visible, async visible => {
  if (visible) {
    draft.value = ''
    validationError.value = ''
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    startListening()
    await nextTick()
    textarea.value?.focus()
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
    'button:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
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

function handleSubmit() {
  const content = draft.value.trim()
  if (!content) {
    validationError.value = 'Enter a preference to capture.'
    nextTick(() => textarea.value?.focus())
    return
  }
  validationError.value = ''
  emit('submit', content)
}

function handleCancel() {
  emit('cancel')
}
</script>

<style scoped>
.capture-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding: var(--spacing-md);
  padding-bottom: max(var(--spacing-md), env(safe-area-inset-bottom));
  background: rgba(0, 0, 0, 0.64);
  backdrop-filter: blur(2px);
}

.capture-dialog {
  width: min(100%, 34rem);
  max-height: min(88dvh, 42rem);
  padding: var(--spacing-lg);
  overflow-y: auto;
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  box-shadow: 0 18px 56px rgba(0, 0, 0, 0.45);
}

.eyebrow {
  color: var(--text-muted);
  font-size: 0.72rem;
  text-transform: uppercase;
}

h2 {
  margin: 0.2rem 0 0;
  font-size: 1.2rem;
}

.explanation {
  margin: var(--spacing-md) 0;
  color: var(--text-secondary);
  line-height: 1.5;
}

.explanation strong {
  color: var(--accent-cyan);
}

form {
  display: grid;
  gap: var(--spacing-sm);
}

label {
  color: var(--text-secondary);
  font-size: 0.82rem;
  font-weight: 700;
}

textarea {
  width: 100%;
  min-height: 7rem;
  box-sizing: border-box;
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-primary);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  resize: vertical;
}

textarea:focus-visible,
button:focus-visible {
  outline: 2px solid var(--accent-cyan);
  outline-offset: 2px;
}

.dialog-error,
.recording-note {
  margin: 0;
  font-size: 0.82rem;
}

.dialog-error {
  color: var(--accent-red);
}

.recording-note {
  color: var(--text-secondary);
}

.dialog-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--spacing-sm);
  margin-top: var(--spacing-sm);
}

.touch-action {
  min-height: 44px;
  padding: 0 var(--spacing-md);
  border-radius: var(--radius-md);
  font-weight: 700;
}

.secondary {
  color: var(--text-primary);
  background: transparent;
  border: 1px solid var(--border-default);
}

.primary {
  color: var(--bg-primary);
  background: var(--accent-cyan);
  border: 1px solid transparent;
}

button:disabled,
textarea:disabled {
  cursor: not-allowed;
  opacity: 0.58;
}

@media (min-width: 768px) {
  .capture-dialog-overlay {
    align-items: center;
  }

  .capture-dialog {
    border-radius: var(--radius-lg);
  }
}
</style>
