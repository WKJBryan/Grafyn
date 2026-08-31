<template>
  <div class="companion-shell">
    <div class="companion-shell-content">
      <slot />
    </div>

    <CompanionBottomNav />
  </div>
</template>

<script setup>
import CompanionBottomNav from './CompanionBottomNav.vue'
</script>

<style scoped>
.companion-shell {
  display: grid;
  grid-template-rows: minmax(0, 1fr) auto;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  background: var(--bg-primary);
}

.companion-shell-content {
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
}

</style>
