<template>
  <div class="dialog-overlay" @click.self="$emit('cancel')">
    <div class="dialog-content">
      <div class="dialog-header">
        <h3>Add Models</h3>
        <button class="close-btn" @click="$emit('cancel')">&times;</button>
      </div>

      <div class="dialog-body">
        <ModelSelector
          :models="filteredModels"
          v-model="selectedModels"
        />
      </div>

      <div class="dialog-footer">
        <button class="btn btn-secondary" @click="$emit('cancel')">Cancel</button>
        <button
          class="btn btn-primary"
          @click="handleSubmit"
          :disabled="selectedModels.length === 0"
        >
          Add {{ selectedModels.length }} Model{{ selectedModels.length !== 1 ? 's' : '' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import ModelSelector from './ModelSelector.vue'

const props = defineProps({
  models: {
    type: Array,
    default: () => []
  },
  existingModelIds: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['submit', 'cancel'])

const selectedModels = ref([])

// Filter out models already used in this tile
const filteredModels = computed(() => {
  return props.models.filter(m => !props.existingModelIds.includes(m.id))
})

function handleSubmit() {
  if (selectedModels.value.length === 0) return
  emit('submit', selectedModels.value)
}

// Handle escape key
function handleKeydown(e) {
  if (e.key === 'Escape') {
    emit('cancel')
  }
}

onMounted(() => {
  document.addEventListener('keydown', handleKeydown)
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', handleKeydown)
})
</script>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  animation: fadeIn 0.15s ease;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.dialog-content {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  width: 90%;
  max-width: 500px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  animation: slideUp 0.2s ease;
}

@keyframes slideUp {
  from {
    opacity: 0;
    transform: translateY(20px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
}

.dialog-header h3 {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
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

.dialog-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-md);
  min-height: 300px;
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md);
  border-top: 1px solid var(--bg-tertiary);
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
