<template>
  <main class="capability-unavailable">
    <GIcon
      name="alert-circle"
      :size="32"
      aria-hidden="true"
    />
    <div>
      <p class="capability-kicker">
        NOT AVAILABLE HERE
      </p>
      <h1>{{ title }}</h1>
      <p>This part of Grafyn is not available in the current runtime yet.</p>
    </div>
  </main>
</template>

<script setup>
import { computed } from 'vue'
import GIcon from '@/components/ui/GIcon.vue'

const props = defineProps({
  capability: {
    type: String,
    default: '',
  },
})

const labels = {
  notesWrite: 'Capture',
  recall: 'Recall',
  twinReview: 'Twin',
  spatialCanvas: 'Canvas',
  importByPath: 'Path import',
}

const title = computed(() => labels[props.capability] || 'This capability')
</script>

<style scoped>
.capability-unavailable {
  min-height: 100%;
  display: flex;
  align-items: flex-start;
  gap: var(--spacing-md);
  padding: max(var(--spacing-xl), env(safe-area-inset-top)) max(var(--spacing-md), env(safe-area-inset-right)) var(--spacing-xl) max(var(--spacing-md), env(safe-area-inset-left));
  color: var(--text-secondary);
}

.capability-unavailable h1 {
  margin: var(--spacing-xs) 0 var(--spacing-sm);
}

.capability-unavailable p {
  margin: 0;
}

.capability-kicker {
  color: var(--accent-warning);
  font-family: 'Fira Code', monospace;
  font-size: 0.7rem;
  letter-spacing: 0.12em;
}
</style>
