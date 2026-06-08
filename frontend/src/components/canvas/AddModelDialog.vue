<template>
  <BaseModal
    max-width="500px"
    @close="$emit('cancel')"
  >
    <template #header>
      <h3>Add Models</h3>
      <button
        class="close-btn"
        @click="$emit('cancel')"
      >
        <GIcon
          name="x"
          :size="14"
        />
      </button>
    </template>

    <ModelSelector
      v-if="filteredModels.length > 0"
      v-model="selectedModels"
      :models="filteredModels"
      :presets="presets"
      @create-preset="emit('create-preset', $event)"
      @update-preset="emit('update-preset', $event)"
      @delete-preset="emit('delete-preset', $event)"
    />
    <div
      v-else
      class="empty-state"
    >
      No additional models are available for this prompt.
    </div>

    <template #footer>
      <button
        class="btn btn-secondary"
        @click="$emit('cancel')"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        :disabled="selectedModels.length === 0"
        @click="handleSubmit"
      >
        Add {{ selectedModels.length }} Model{{ selectedModels.length !== 1 ? 's' : '' }}
      </button>
    </template>
  </BaseModal>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import BaseModal from '@/components/BaseModal.vue'
import ModelSelector from './ModelSelector.vue'
import GIcon from '@/components/ui/GIcon.vue'

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  presets: {
    type: Array,
    default: () => []
  },
  existingModelIds: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['submit', 'cancel', 'create-preset', 'update-preset', 'delete-preset'])

const selectedModels = ref([])

const filteredModels = computed(() => {
  return props.models.filter(m => !props.existingModelIds.includes(m.id))
})

function handleSubmit() {
  if (selectedModels.value.length === 0) return
  emit('submit', selectedModels.value)
}

function handleKeydown(e) {
  if (e.key === 'Escape') emit('cancel')
}

onMounted(() => document.addEventListener('keydown', handleKeydown))
onBeforeUnmount(() => document.removeEventListener('keydown', handleKeydown))
</script>

<style scoped>
/* Animation overrides — BaseModal provides the overlay/modal structure */
:deep(.base-modal-overlay) {
  animation: fadeIn 0.15s ease;
  backdrop-filter: blur(8px);
}

:deep(.base-modal) {
  animation: slideUp 0.2s ease;
  max-height: 80vh;
}

:deep(.base-modal__header),
:deep(.base-modal__body),
:deep(.base-modal__footer) {
  padding: var(--spacing-md);
}

:deep(.base-modal__body) {
  min-height: 300px;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes slideUp {
  from { opacity: 0; transform: translateY(20px); }
  to { opacity: 1; transform: translateY(0); }
}

.close-btn {
  padding: 4px 8px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 1.25rem;
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition: all 0.15s;
}

.close-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.empty-state {
  min-height: 220px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  text-align: center;
  font-size: 0.9rem;
}

.btn {
  padding: 8px 16px;
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all 0.15s;
  border: none;
}

.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
}

.btn-secondary:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.btn-primary {
  background: var(--accent-primary);
  color: white;
}

.btn-primary:hover:not(:disabled) {
  background: #6b4fd9;
}

.btn-primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
