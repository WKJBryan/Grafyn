<template>
  <section class="tab-panel memory-grid">
    <section class="workspace-band">
      <div class="panel-header compact">
        <div>
          <h2>Adaptive Digest</h2>
          <span>{{ twinStore.memoryDigestItems.length }} clustered items</span>
        </div>
      </div>
      <div
        v-if="twinStore.memoryDigestItems.length === 0"
        class="empty-panel"
      >
        No digest items need review.
      </div>
      <article
        v-for="item in twinStore.memoryDigestItems"
        :key="item.id"
        class="digest-card"
      >
        <div class="card-topline">
          <span class="status-pill">{{ statusLabel(item.state) }}</span>
          <span>{{ item.evidence_count }} evidence</span>
          <span>{{ item.trigger_reason }}</span>
        </div>
        <p>{{ item.pattern }}</p>
        <small v-if="item.latest_evidence?.summary">{{ item.latest_evidence.summary }}</small>
        <ReviewActions
          @review="action => twinStore.reviewMemoryDigestItem(item.id, action)"
        />
      </article>
    </section>

    <section class="workspace-band">
      <div class="panel-header compact">
        <div>
          <h2>User Records</h2>
          <span>{{ twinStore.filteredReviewRecords.length }} shown</span>
        </div>
        <select v-model="twinStore.selectedRecordState">
          <option
            v-for="state in recordStates"
            :key="state"
            :value="state"
          >
            {{ statusLabel(state) }}
          </option>
        </select>
      </div>
      <div
        v-if="twinStore.filteredReviewRecords.length === 0"
        class="empty-panel"
      >
        No records in this state.
      </div>
      <article
        v-for="item in twinStore.filteredReviewRecords"
        :key="item.record.id"
        class="record-card"
      >
        <div class="card-topline">
          <span class="status-pill">{{ kindLabel(item.record.kind) }}</span>
          <span>{{ statusLabel(item.record.promotion_state) }}</span>
          <span>{{ item.evidence_count }} evidence</span>
        </div>
        <p>{{ item.record.content }}</p>
        <div class="record-actions">
          <button @click="twinStore.openEvidence(item.record.id)">
            Evidence
          </button>
          <button @click="twinStore.setPromotion(item.record.id, 'endorsed')">
            Endorse
          </button>
          <button @click="twinStore.setPromotion(item.record.id, 'private')">
            Private
          </button>
          <button @click="twinStore.setPromotion(item.record.id, 'no_train')">
            No Train
          </button>
          <button @click="twinStore.setPromotion(item.record.id, 'rejected')">
            Reject
          </button>
        </div>
      </article>
    </section>
  </section>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import { statusLabel, kindLabel } from '@/utils/twinFormat'
import ReviewActions from './ReviewActions.vue'

const recordStates = ['candidate', 'auto_promoted', 'endorsed', 'private', 'no_train', 'rejected']
const twinStore = useTwinStore()
</script>
