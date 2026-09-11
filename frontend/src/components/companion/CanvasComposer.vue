<template>
  <form
    class="canvas-composer"
    @submit.prevent="submit"
  >
    <div
      v-if="parent"
      class="parent-chip"
    >
      <span>Following {{ parent.modelId }}</span>
      <button
        type="button"
        aria-label="Clear Canvas follow-up"
        @click="$emit('clear-parent')"
      >
        Clear
      </button>
    </div>

    <textarea
      v-model="prompt"
      aria-label="Canvas prompt"
      placeholder="Ask one model…"
      rows="3"
      @keydown.ctrl.enter="submit"
    />

    <div class="composer-options">
      <label>
        <span>Model</span>
        <select
          v-model="modelId"
          aria-label="Canvas model"
        >
          <option
            v-for="model in models"
            :key="model.id"
            :value="model.id"
          >
            {{ model.name || model.id }}
          </option>
        </select>
      </label>
      <label>
        <span>Context</span>
        <select
          v-model="mode"
          aria-label="Canvas context"
        >
          <option value="plain">Plain</option>
          <option value="knowledge">Knowledge</option>
          <option value="twin">Twin</option>
        </select>
      </label>
    </div>

    <button
      class="send-button"
      type="submit"
      :disabled="busy || !prompt.trim() || !selectedModel"
    >
      {{ busy ? 'Sending…' : 'Send' }}
    </button>
  </form>
</template>

<script setup>
import { computed, ref, watch } from 'vue'

const props = defineProps({
  models: { type: Array, default: () => [] },
  parent: { type: Object, default: null },
  busy: { type: Boolean, default: false },
  draft: { type: String, default: undefined },
})

const emit = defineEmits(['submit', 'clear-parent', 'update:draft'])
const localPrompt = ref('')
const modelId = ref('')
const mode = ref('plain')

const prompt = computed({
  get: () => props.draft === undefined ? localPrompt.value : props.draft,
  set: value => {
    if (props.draft === undefined) localPrompt.value = value
    emit('update:draft', value)
  },
})

watch(() => props.models, models => {
  if (!models.some(model => model.id === modelId.value)) {
    modelId.value = models[0]?.id || ''
  }
}, { immediate: true })

const selectedModel = computed(() => props.models.find(model => model.id === modelId.value) || null)

function submit() {
  const value = prompt.value.trim()
  if (!value || !selectedModel.value || props.busy) return
  emit('submit', {
    prompt: value,
    modelId: selectedModel.value.id,
    provider: runtimeProvider(selectedModel.value),
    mode: mode.value,
    parentTileId: props.parent?.tileId || null,
    parentModelId: props.parent?.modelId || null,
  })
}

function runtimeProvider(model) {
  return model.provider?.toLowerCase() === 'ollama' ? 'ollama' : 'openrouter'
}
</script>

<style scoped>
.canvas-composer {
  display: grid;
  gap: var(--spacing-sm);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
}

.canvas-composer textarea,
.canvas-composer select,
.canvas-composer button {
  min-height: 44px;
  box-sizing: border-box;
  color: var(--text-primary);
  border-radius: var(--radius-md);
}

.canvas-composer textarea,
.canvas-composer select {
  width: 100%;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
}

.canvas-composer textarea {
  resize: vertical;
}

.composer-options {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  gap: var(--spacing-sm);
}

.composer-options label {
  display: grid;
  gap: 0.25rem;
}

.composer-options span {
  color: var(--text-muted);
  font-size: 0.7rem;
  font-weight: 700;
  text-transform: uppercase;
}

.parent-chip {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.parent-chip button {
  padding: 0 var(--spacing-sm);
  color: var(--text-secondary);
  background: transparent;
  border: 1px solid var(--border-default);
}

.send-button {
  padding: 0 var(--spacing-md);
  color: var(--bg-primary) !important;
  background: var(--accent-cyan);
  border: 1px solid transparent;
  font-weight: 800;
}

.send-button:disabled {
  opacity: 0.55;
}
</style>
