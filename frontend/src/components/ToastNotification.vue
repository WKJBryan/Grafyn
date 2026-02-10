<template>
  <Teleport to="body">
    <div
      v-if="toasts.length > 0"
      class="toast-container"
    >
      <TransitionGroup name="toast">
        <div
          v-for="toast in toasts"
          :key="toast.id"
          class="toast"
          :class="'toast-' + toast.type"
          @click="remove(toast.id)"
        >
          <span class="toast-icon">{{ icons[toast.type] }}</span>
          <span class="toast-message">{{ toast.message }}</span>
          <button
            class="toast-close"
            @click.stop="remove(toast.id)"
          >
            &times;
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<script setup>
import { useToast } from '@/composables/useToast'

const { toasts, remove } = useToast()

const icons = {
  success: '\u2713',
  error: '\u2717',
  warning: '\u26A0',
  info: '\u2139',
}
</script>

<style scoped>
.toast-container {
  position: fixed;
  bottom: var(--spacing-lg, 24px);
  right: var(--spacing-lg, 24px);
  z-index: 9999;
  display: flex;
  flex-direction: column-reverse;
  gap: var(--spacing-sm, 8px);
  max-width: 380px;
  pointer-events: none;
}

.toast {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm, 8px);
  padding: var(--spacing-sm, 8px) var(--spacing-md, 16px);
  border-radius: var(--radius-md, 8px);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  color: var(--text-primary);
  font-size: 0.875rem;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  cursor: pointer;
  pointer-events: auto;
  min-width: 240px;
}

.toast-success {
  border-left: 3px solid var(--accent-success);
}

.toast-error {
  border-left: 3px solid var(--accent-danger);
}

.toast-warning {
  border-left: 3px solid var(--accent-warning);
}

.toast-info {
  border-left: 3px solid var(--accent-secondary);
}

.toast-icon {
  flex-shrink: 0;
  width: 20px;
  text-align: center;
  font-size: 1rem;
}

.toast-success .toast-icon { color: var(--accent-success); }
.toast-error .toast-icon { color: var(--accent-danger); }
.toast-warning .toast-icon { color: var(--accent-warning); }
.toast-info .toast-icon { color: var(--accent-secondary); }

.toast-message {
  flex: 1;
  line-height: 1.4;
}

.toast-close {
  flex-shrink: 0;
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: 1.25rem;
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
  transition: color var(--transition-fast);
}

.toast-close:hover {
  color: var(--text-primary);
}

/* Transitions */
.toast-enter-active {
  animation: toastIn 0.3s ease-out;
}

.toast-leave-active {
  animation: toastOut 0.2s ease-in forwards;
}

.toast-move {
  transition: transform 0.3s ease;
}

@keyframes toastIn {
  from {
    opacity: 0;
    transform: translateX(100%);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}

@keyframes toastOut {
  from {
    opacity: 1;
    transform: translateX(0);
  }
  to {
    opacity: 0;
    transform: translateX(100%);
  }
}
</style>
