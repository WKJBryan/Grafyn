<template>
  <section class="tab-panel">
    <div class="panel-header">
      <div>
        <h2>Action Gaps</h2>
        <span>{{ twinStore.actionGaps.length }} stated versus revealed patterns</span>
      </div>
    </div>
    <div
      v-if="twinStore.actionGaps.length === 0"
      class="empty-panel"
    >
      No action gaps yet.
    </div>
    <ActionGapRow
      v-for="gap in twinStore.actionGaps"
      :key="gap.id"
      :gap="gap"
      @review="twinStore.reviewActionGap"
    />
  </section>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import ActionGapRow from './ActionGapRow.vue'

const twinStore = useTwinStore()
</script>
