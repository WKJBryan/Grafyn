<template>
  <nav
    class="companion-nav"
    aria-label="Primary"
  >
    <RouterLink
      v-for="item in destinations"
      :key="item.to"
      :to="item.to"
      class="companion-nav-link"
      :aria-current="isCurrent(item) ? 'page' : undefined"
    >
      <GIcon
        :name="item.icon"
        :size="20"
        aria-hidden="true"
      />
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>

<script setup>
import { useRoute } from 'vue-router'
import GIcon from '@/components/ui/GIcon.vue'

const destinations = [
  { label: 'Capture', to: '/', icon: 'plus', capability: 'notesWrite' },
  { label: 'Recall', to: '/recall', icon: 'search', capability: 'recall' },
  { label: 'Twin', to: '/twin', icon: 'orbit', capability: 'twinReview' },
  { label: 'Canvas', to: '/canvas', icon: 'layout-grid', capability: 'linearCanvas' },
]

const route = useRoute()

function isCurrent(item) {
  if (route.path === item.to) return true
  if (item.to === '/canvas' && route.path.startsWith('/canvas/')) return true
  return route.name === 'capability-unavailable'
    && route.query.capability === item.capability
}
</script>

<style scoped>
.companion-nav {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  padding: var(--spacing-xs) max(var(--spacing-xs), env(safe-area-inset-right)) max(var(--spacing-xs), env(safe-area-inset-bottom)) max(var(--spacing-xs), env(safe-area-inset-left));
  background: var(--bg-secondary);
  border-top: 1px solid var(--border-default);
}

.companion-nav-link {
  min-width: 0;
  min-height: 52px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  color: var(--text-muted);
  border-radius: var(--radius-md);
  font-size: 0.7rem;
  font-weight: 600;
  text-decoration: none;
}

.companion-nav-link:hover,
.companion-nav-link:focus-visible {
  color: var(--text-primary);
  background: var(--bg-tertiary);
}

.companion-nav-link:focus-visible {
  outline: 2px solid var(--accent-secondary);
  outline-offset: -2px;
}

.companion-nav-link[aria-current='page'] {
  color: var(--accent-cyan);
  background: color-mix(in srgb, var(--accent-cyan) 12%, transparent);
}
</style>
