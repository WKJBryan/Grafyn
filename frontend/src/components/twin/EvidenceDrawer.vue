<template>
  <aside
    v-if="twinStore.selectedRecordId"
    class="evidence-drawer"
  >
    <div class="drawer-header">
      <h2>Evidence</h2>
      <button @click="twinStore.selectedRecordId = null">
        x
      </button>
    </div>
    <div
      v-if="twinStore.evidenceLoading"
      class="empty-panel"
    >
      Loading evidence...
    </div>
    <div
      v-else-if="twinStore.selectedEvidence.length === 0"
      class="empty-panel"
    >
      No evidence events found.
    </div>
    <article
      v-for="item in twinStore.selectedEvidence"
      :key="item.event_id"
      class="evidence-item"
    >
      <div class="card-topline">
        <span>{{ eventLabel(item.event_type) }}</span>
        <span>{{ formatDate(item.created_at) }}</span>
      </div>
      <p v-if="item.prompt_excerpt">
        {{ item.prompt_excerpt }}
      </p>
      <p v-if="item.response_excerpt">
        {{ item.response_excerpt }}
      </p>
      <small>{{ item.session_id }} <span v-if="item.model_id">/ {{ item.model_id }}</span></small>
    </article>
  </aside>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import { eventLabel, formatDate } from '@/utils/twinFormat'

const twinStore = useTwinStore()
</script>
