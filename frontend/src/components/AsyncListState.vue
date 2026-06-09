<template>
  <slot
    v-if="loading"
    name="loading"
  >
    <div class="async-list-state__loading">
      <p class="text-muted">
        Loading...
      </p>
    </div>
  </slot>
  <slot
    v-else-if="empty"
    name="empty"
  >
    <div class="async-list-state__empty">
      <p class="text-muted">
        No items found
      </p>
    </div>
  </slot>
  <slot v-else />
</template>

<script setup>
defineProps({
  loading: { type: Boolean, default: false },
  empty: { type: Boolean, default: false },
})
</script>

<style scoped>
.async-list-state__loading,
.async-list-state__empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
}

.async-list-state__loading p,
.async-list-state__empty p {
  margin: 0;
  font-size: 0.875rem;
}
</style>
