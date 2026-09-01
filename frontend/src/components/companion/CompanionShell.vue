<template>
  <div class="companion-shell">
    <header class="companion-utility-bar">
      <span class="companion-wordmark">Grafyn</span>
      <button
        type="button"
        class="companion-settings-trigger"
        aria-label="Open companion settings"
        @click="showSettings = true"
      >
        <GIcon
          name="settings"
          :size="20"
          aria-hidden="true"
        />
      </button>
    </header>

    <div class="companion-shell-content">
      <slot />
    </div>

    <CompanionBottomNav />
    <CompanionSettingsSheet
      v-if="showSettings"
      @close="showSettings = false"
    />
  </div>
</template>

<script setup>
import { ref } from 'vue'
import GIcon from '@/components/ui/GIcon.vue'
import CompanionBottomNav from './CompanionBottomNav.vue'
import CompanionSettingsSheet from './CompanionSettingsSheet.vue'

const showSettings = ref(false)
</script>

<style scoped>
.companion-shell {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) auto;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  background: var(--bg-primary);
}

.companion-utility-bar {
  min-width: 0;
  min-height: calc(52px + env(safe-area-inset-top));
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--spacing-md);
  padding: env(safe-area-inset-top) max(var(--spacing-sm), env(safe-area-inset-right)) 4px max(var(--spacing-md), env(safe-area-inset-left));
  background: var(--bg-primary);
  border-bottom: 1px solid var(--border-subtle);
}

.companion-wordmark {
  min-height: 44px;
  display: inline-flex;
  align-items: center;
  color: var(--text-primary);
  font-size: 0.78rem;
  font-weight: 750;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.companion-settings-trigger {
  width: 44px;
  height: 44px;
  display: grid;
  place-items: center;
  flex: none;
  padding: 0;
  color: var(--text-secondary);
  background: transparent;
  border: 0;
  border-radius: var(--radius-md);
}

.companion-settings-trigger:hover,
.companion-settings-trigger:focus-visible {
  color: var(--text-primary);
  background: var(--bg-tertiary);
}

.companion-settings-trigger:focus-visible {
  outline: 2px solid var(--accent-cyan);
  outline-offset: -2px;
}

.companion-shell-content {
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
}

</style>
