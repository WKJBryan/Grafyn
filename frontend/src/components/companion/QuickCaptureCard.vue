<template>
  <form
    class="quick-capture card"
    @submit.prevent="submit"
  >
    <div class="quick-capture-heading">
      <div>
        <p class="quick-capture-kicker">
          INBOX
        </p>
        <h1>Capture what is happening</h1>
      </div>
      <GIcon
        name="grafyn"
        :size="28"
        aria-hidden="true"
      />
    </div>

    <label
      class="sr-only"
      for="companion-capture-content"
    >Capture what is happening</label>
    <textarea
      id="companion-capture-content"
      v-model="content"
      rows="7"
      placeholder="A thought, encounter, decision, or observation…"
      enterkeyhint="done"
    />

    <CaptureContextFields v-model="context" />

    <label class="local-only-control">
      <input
        v-model="localOnly"
        name="local-only"
        type="checkbox"
      >
      <span>
        <strong>Keep only on this device</strong>
        <small>Overrides your vault sync preference for this capture.</small>
      </span>
    </label>

    <p
      v-if="error"
      class="capture-message capture-error"
      role="alert"
    >
      {{ error }}
    </p>
    <p
      v-else-if="success"
      class="capture-message capture-success"
      role="status"
    >
      Captured as {{ success }}.
    </p>

    <button
      type="submit"
      class="btn quick-capture-submit"
      :disabled="!content.trim() || submitting || uncertain"
    >
      <GIcon
        :name="submitting ? 'loader' : 'plus'"
        :icon-class="submitting ? 'spinning' : ''"
        aria-hidden="true"
      />
      {{ submitting ? 'Capturing…' : 'Capture' }}
    </button>
  </form>
</template>

<script setup>
import { ref } from 'vue'
import { twin } from '@/api/client'
import GIcon from '@/components/ui/GIcon.vue'
import CaptureContextFields from './CaptureContextFields.vue'

const props = defineProps({
  attachmentDigests: {
    type: Array,
    default: () => [],
  },
})

const emit = defineEmits(['captured', 'capture-uncertain'])

const emptyContext = () => ({
  person: '',
  role: '',
  relationship: '',
  environment: '',
  activity: '',
  goal: '',
})

const content = ref('')
const context = ref(emptyContext())
const localOnly = ref(false)
const submitting = ref(false)
const uncertain = ref(false)
const error = ref('')
const success = ref('')

function isUncertainCaptureError(rejection) {
  const message = String(rejection?.message ?? rejection).toLowerCase()
  return message.includes('do not retry')
    || message.includes('before deciding whether to retry')
}

async function submit() {
  if (!content.value.trim() || submitting.value || uncertain.value) return

  submitting.value = true
  error.value = ''
  success.value = ''

  try {
    const response = await twin.createCompanionCapture({
      content: content.value,
      captureKind: 'text',
      context: { ...context.value },
      attachmentDigests: [...props.attachmentDigests],
      grafynSync: localOnly.value ? 'local_only' : 'inherit',
    })
    content.value = ''
    context.value = emptyContext()
    success.value = response.note?.title || 'a new note'
    emit('captured', response)
  } catch (rejection) {
    if (isUncertainCaptureError(rejection)) {
      uncertain.value = true
      error.value = 'This capture may already be saved. Do not submit it again. Check Recent captures before continuing.'
      emit('capture-uncertain')
    } else {
      error.value = 'Capture could not be saved. Your draft is still here.'
    }
  } finally {
    submitting.value = false
  }
}
</script>

<style scoped>
.quick-capture {
  display: grid;
  gap: var(--spacing-md);
  border-color: var(--border-default);
}

.quick-capture-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--spacing-md);
}

.quick-capture-heading h1 {
  margin: 0;
  color: var(--text-secondary);
  font-size: clamp(1.35rem, 6vw, 1.75rem);
}

.quick-capture-kicker {
  margin: 0 0 var(--spacing-xs);
  color: var(--accent-cyan);
  font-family: 'Fira Code', monospace;
  font-size: 0.7rem;
  font-weight: 600;
  letter-spacing: 0.12em;
}

.quick-capture textarea {
  min-height: 148px;
  resize: vertical;
  line-height: 1.55;
}

.local-only-control {
  min-height: 44px;
  display: flex;
  align-items: flex-start;
  gap: var(--spacing-sm);
  color: var(--text-secondary);
  cursor: pointer;
}

.local-only-control input {
  width: 20px;
  height: 20px;
  margin-top: 2px;
  accent-color: var(--accent-primary);
}

.local-only-control span {
  display: grid;
}

.local-only-control small {
  color: var(--text-muted);
}

.capture-message {
  margin: 0;
  font-size: 0.875rem;
}

.capture-success {
  color: var(--accent-success);
}

.capture-error {
  color: var(--accent-danger);
}

.quick-capture-submit {
  min-height: 48px;
  background: #5a5fad;
  color: white;
}

.quick-capture-submit:hover:not(:disabled) {
  background: #4c5095;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
</style>
