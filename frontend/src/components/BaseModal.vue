<template>
  <div
    class="base-modal-overlay"
    @click.self="handleOverlayClick"
  >
    <div
      class="base-modal"
      :style="containerStyle"
    >
      <div class="base-modal__header">
        <slot name="header">
          <h3>{{ title }}</h3>
          <button
            v-if="showClose"
            class="base-modal__close"
            aria-label="Close"
            @click="$emit('close')"
          >
            &times;
          </button>
        </slot>
      </div>
      <div class="base-modal__body">
        <slot />
      </div>
      <div
        v-if="$slots.footer"
        class="base-modal__footer"
      >
        <slot name="footer" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  title: { type: String, default: '' },
  closeOnOverlay: { type: Boolean, default: true },
  showClose: { type: Boolean, default: true },
  maxWidth: { type: String, default: null },
  width: { type: String, default: null },
})

const emit = defineEmits(['close'])

const containerStyle = computed(() => {
  const style = {}
  if (props.maxWidth) style.maxWidth = props.maxWidth
  if (props.width) style.width = props.width
  return style
})

function handleOverlayClick() {
  if (props.closeOnOverlay) emit('close')
}
</script>

<style scoped>
.base-modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: var(--spacing-lg, 1.5rem);
}

.base-modal {
  background: var(--bg-secondary, #1a1a2e);
  border: 1px solid var(--border-color, rgba(255, 255, 255, 0.1));
  border-radius: var(--radius-lg, 12px);
  display: flex;
  flex-direction: column;
  max-height: 90vh;
  width: 90%;
  max-width: 520px;
  overflow: hidden;
}

.base-modal__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-lg, 1.5rem);
  border-bottom: 1px solid var(--border-color, rgba(255, 255, 255, 0.1));
  flex-shrink: 0;
}

.base-modal__header h2,
.base-modal__header h3 {
  margin: 0;
  font-size: 1.1rem;
  font-weight: 600;
  color: var(--text-primary, #e0e0e0);
}

.base-modal__close {
  background: none;
  border: none;
  color: var(--text-secondary, #888);
  cursor: pointer;
  font-size: 1.4rem;
  line-height: 1;
  padding: 0.2rem 0.4rem;
  border-radius: 4px;
  transition: color 0.15s, background 0.15s;
  flex-shrink: 0;
}

.base-modal__close:hover {
  color: var(--text-primary, #e0e0e0);
  background: rgba(255, 255, 255, 0.08);
}

.base-modal__body {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-lg, 1.5rem);
}

.base-modal__footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--spacing-sm, 0.5rem);
  padding: var(--spacing-md, 1rem) var(--spacing-lg, 1.5rem);
  border-top: 1px solid var(--border-color, rgba(255, 255, 255, 0.1));
  flex-shrink: 0;
}
</style>
