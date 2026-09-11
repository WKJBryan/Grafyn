<template>
  <details class="capture-context">
    <summary>Context <span>optional</span></summary>
    <div class="capture-context-grid">
      <label
        v-for="field in fields"
        :key="field.name"
        :for="`capture-${field.name}`"
      >
        <span>{{ field.label }}</span>
        <input
          :id="`capture-${field.name}`"
          :name="field.name"
          :value="modelValue[field.name]"
          :placeholder="field.placeholder"
          @input="update(field.name, $event.target.value)"
        >
      </label>
    </div>
  </details>
</template>

<script setup>
const props = defineProps({
  modelValue: {
    type: Object,
    required: true,
  },
})

const emit = defineEmits(['update:modelValue'])

const fields = [
  { name: 'person', label: 'Person', placeholder: 'Who was involved?' },
  { name: 'role', label: 'Role', placeholder: 'What role were they in?' },
  { name: 'relationship', label: 'Relationship', placeholder: 'How are you connected?' },
  { name: 'environment', label: 'Environment', placeholder: 'Where did this happen?' },
  { name: 'activity', label: 'Activity', placeholder: 'What were you doing?' },
  { name: 'goal', label: 'Goal', placeholder: 'What were you trying to do?' },
]

function update(name, value) {
  emit('update:modelValue', { ...props.modelValue, [name]: value })
}
</script>

<style scoped>
.capture-context {
  border-top: 1px solid var(--border-subtle);
  padding-top: var(--spacing-sm);
}

.capture-context summary {
  min-height: 44px;
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 0.875rem;
  font-weight: 600;
}

.capture-context summary span {
  color: var(--text-muted);
  font-size: 0.75rem;
  font-weight: 400;
}

.capture-context-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing-sm);
  padding-top: var(--spacing-sm);
}

.capture-context-grid label {
  display: grid;
  gap: var(--spacing-xs);
  color: var(--text-secondary);
  font-size: 0.75rem;
}

@media (max-width: 480px) {
  .capture-context-grid {
    grid-template-columns: 1fr;
  }
}
</style>
