<template>
  <section class="tab-panel">
    <div class="panel-header">
      <div>
        <h2>Decisions</h2>
        <span>{{ twinStore.decisions.length }} episodes — record what you actually chose to reveal each sealed twin prediction</span>
      </div>
    </div>
    <div
      v-if="twinStore.decisions.length === 0"
      class="empty-panel"
    >
      No decision episodes yet. Start a Decision in Canvas and it will appear here.
    </div>
    <DecisionRow
      v-for="item in twinStore.decisions"
      :key="item.episode.id"
      :item="item"
      :highlighted="item.episode.id === routeDecisionId"
      :trace-open="item.episode.id === routeDecisionId && routeTraceRequested"
      @update-outcome="twinStore.updateDecisionOutcome"
    />
  </section>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { useTwinStore } from '@/stores/twin'
import DecisionRow from './DecisionRow.vue'

const twinStore = useTwinStore()
const route = useRoute() || { query: {} }
const routeDecisionId = computed(() => String(route.query?.decision || ''))
const routeTraceRequested = computed(() => route.query?.trace === '1')
</script>
