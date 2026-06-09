<template>
  <div
    class="panel-header-base"
    :class="{ 'panel-header-base--collapsible': collapsible }"
    @click="collapsible && $emit('toggle')"
  >
    <span
      v-if="collapsible"
      class="panel-header-base__icon"
    >{{ expanded ? '▼' : '▶' }}</span>
    <slot name="title">
      <h3 class="panel-header-base__title">
        {{ title }}
      </h3>
    </slot>
    <span
      v-if="count !== null && count !== undefined"
      class="panel-header-base__count"
      :class="`panel-header-base__count--${badgeVariant}`"
    >{{ count }}</span>
    <slot name="action" />
  </div>
</template>

<script setup>
defineProps({
  title: { type: String, default: '' },
  count: { type: Number, default: null },
  collapsible: { type: Boolean, default: false },
  expanded: { type: Boolean, default: true },
  badgeVariant: { type: String, default: 'accent' }, // 'accent' | 'muted'
})
defineEmits(['toggle'])
</script>

<style scoped>
.panel-header-base {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm, 0.5rem);
}

.panel-header-base--collapsible {
  cursor: pointer;
  padding: var(--spacing-sm, 0.5rem) var(--spacing-md, 1rem);
  border-radius: var(--radius-sm, 4px);
  transition: background 0.15s;
}

.panel-header-base--collapsible:hover {
  background: var(--bg-hover);
}

.panel-header-base__icon {
  font-size: 0.75rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.panel-header-base__title {
  flex: 1;
  margin: 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-secondary);
}

/* accent variant — pill with accent-primary background */
.panel-header-base__count--accent {
  background: var(--accent-primary);
  color: white;
  font-size: 0.75rem;
  padding: 2px 6px;
  border-radius: 999px;
  min-width: 20px;
  text-align: center;
  flex-shrink: 0;
}

/* muted variant — small subdued badge */
.panel-header-base__count--muted {
  font-size: 0.75rem;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 8px;
  border-radius: var(--radius-sm, 4px);
  flex-shrink: 0;
}
</style>
