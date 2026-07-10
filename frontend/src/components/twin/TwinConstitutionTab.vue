<template>
  <section class="tab-panel">
    <div class="panel-header">
      <div>
        <h2>Constitution</h2>
        <span>The value rules the twin reasons from — {{ twinStore.groupedConstitution.length }} dimensions</span>
      </div>
      <button
        class="btn btn-secondary btn-sm"
        @click="twinStore.activeTab = 'setup'"
      >
        Setup
      </button>
    </div>

    <div
      v-if="twinStore.constitutionItems.length === 0"
      class="empty-panel"
    >
      No constitution items yet. Run Constitution to infer them from records and decisions.
    </div>
    <section
      v-for="group in twinStore.groupedConstitution"
      :key="group.dimension"
      class="dimension-section"
    >
      <h3>{{ dimensionLabel(group.dimension) }}</h3>
      <article
        v-for="item in group.items"
        :key="item.id"
        class="constitution-card"
      >
        <div class="card-topline">
          <span class="status-pill">{{ statusLabel(item.status) }}</span>
          <span>{{ formatPercent(item.confidence) }} confidence</span>
          <span>{{ item.evidence_refs?.length || 0 }} evidence</span>
          <span>{{ constitutionSourceLabel(item) }}</span>
        </div>
        <p>{{ item.claim }}</p>
        <div
          v-if="constitutionEvidenceLabels(item).length"
          class="source-row"
        >
          <span
            v-for="label in constitutionEvidenceLabels(item)"
            :key="`${item.id}-${label}`"
          >{{ label }}</span>
        </div>
        <div
          v-if="item.scope?.length"
          class="tag-row"
        >
          <span
            v-for="scope in item.scope"
            :key="scope"
          >{{ scope }}</span>
        </div>
        <ReviewActions
          @review="action => twinStore.reviewConstitutionItem(item.id, action)"
        />
      </article>
    </section>
  </section>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import {
  dimensionLabel,
  statusLabel,
  formatPercent,
  constitutionSourceLabel,
  constitutionEvidenceLabels
} from '@/utils/twinFormat'
import ReviewActions from './ReviewActions.vue'

const twinStore = useTwinStore()
</script>
