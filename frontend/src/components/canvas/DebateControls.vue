<template>
  <div class="debate-controls">
    <div class="mode-toggle">
      <button
        class="mode-btn"
        :class="{ active: localMode === 'auto' }"
        @click="localMode = 'auto'"
      >
        Auto
      </button>
      <button
        class="mode-btn"
        :class="{ active: localMode === 'mediated' }"
        @click="localMode = 'mediated'"
      >
        Mediated
      </button>
    </div>

    <div
      v-if="localMode === 'mediated'"
      class="mediated-input"
    >
      <textarea
        v-model="customPrompt"
        placeholder="Enter your debate instruction..."
        rows="2"
        class="prompt-input"
        @keydown.ctrl.enter="handleContinue"
      />
      <button
        class="btn btn-sm btn-primary"
        :disabled="!customPrompt.trim()"
        @click="handleContinue"
      >
        Send
      </button>
    </div>

    <div class="control-actions">
      <button
        v-if="debate.status === 'active'"
        class="btn btn-sm btn-secondary"
        @click="$emit('pause')"
      >
        Pause
      </button>
      <button
        v-if="debate.status === 'paused'"
        class="btn btn-sm btn-secondary"
        @click="$emit('resume')"
      >
        Resume
      </button>
      <button
        v-if="localMode === 'auto' && debate.status === 'active'"
        class="btn btn-sm btn-primary"
        @click="handleAutoRound"
      >
        Next Round
      </button>
      <button
        class="btn btn-sm btn-ghost"
        @click="$emit('end')"
      >
        End Debate
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'

const props = defineProps({
  debate: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['continue', 'pause', 'resume', 'end'])

// Local state
const localMode = ref(props.debate.debate_mode || 'auto')
const customPrompt = ref('')

// Methods
function handleContinue() {
  if (!customPrompt.value.trim()) return
  emit('continue', customPrompt.value.trim())
  customPrompt.value = ''
}

function handleAutoRound() {
  emit('continue', 'Continue the debate. Respond to the previous arguments and provide your updated perspective.')
}
</script>

<style scoped>
.debate-controls {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-primary);
  border-top: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
}

.mode-toggle {
  display: flex;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  padding: 2px;
}

.mode-btn {
  flex: 1;
  padding: var(--spacing-xs) var(--spacing-sm);
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.75rem;
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition: all 0.15s;
}

.mode-btn:hover {
  color: var(--text-primary);
}

.mode-btn.active {
  background: var(--accent-primary);
  color: white;
}

.mediated-input {
  display: flex;
  gap: var(--spacing-xs);
  align-items: flex-end;
}

.prompt-input {
  flex: 1;
  padding: var(--spacing-xs);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8125rem;
  font-family: inherit;
  resize: none;
}

.prompt-input:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.control-actions {
  display: flex;
  gap: var(--spacing-xs);
  justify-content: flex-end;
}

.btn-ghost {
  background: transparent;
  color: var(--text-muted);
  border: 1px solid var(--border-subtle);
}

.btn-ghost:hover {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
}
</style>
