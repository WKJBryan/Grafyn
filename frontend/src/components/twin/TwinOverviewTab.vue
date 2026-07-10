<template>
  <section class="tab-panel">
    <div class="overview-stats">
      <button
        class="metric-tile"
        @click="twinStore.activeTab = 'constitution'"
      >
        <span>Constitution</span>
        <strong>{{ twinStore.activeConstitutionCount }}</strong>
        <small>{{ twinStore.constitutionItems.length }} total items</small>
      </button>
      <button
        class="metric-tile"
        @click="twinStore.activeTab = 'action_gaps'"
      >
        <span>Action Gaps</span>
        <strong>{{ twinStore.activeActionGapCount }}</strong>
        <small>{{ twinStore.actionGaps.length }} total gaps</small>
      </button>
      <button
        class="metric-tile"
        @click="twinStore.activeTab = 'decisions'"
      >
        <span>Decisions</span>
        <strong>{{ twinStore.decisions.length }}</strong>
        <small>{{ twinStore.pendingOutcomeCount }} outcomes pending</small>
      </button>
      <button
        class="metric-tile"
        @click="twinStore.activeTab = 'memory'"
      >
        <span>Review</span>
        <strong>{{ twinStore.pendingReviewCount }}</strong>
        <small>digest plus candidate records</small>
      </button>
    </div>

    <section
      v-if="twinStore.showTutorialIntro"
      class="workspace-band tutorial-intro"
    >
      <div class="band-header">
        <h2>How To Use</h2>
        <div class="header-actions compact">
          <button
            class="text-button"
            @click="twinStore.activeTab = 'guide'"
          >
            Open Guide
          </button>
          <button
            class="text-button"
            @click="twinStore.dismissTutorial"
          >
            Dismiss
          </button>
        </div>
      </div>
      <p>
        Start decisions in Canvas, then use Twin Workspace for review, setup, outcomes,
        configuration, and benchmark export.
      </p>
    </section>

    <section class="workspace-band">
      <div class="band-header">
        <h2>Recent Decisions</h2>
        <button
          class="text-button"
          @click="twinStore.activeTab = 'decisions'"
        >
          View all
        </button>
      </div>
      <div
        v-if="twinStore.recentDecisions.length === 0"
        class="empty-panel"
      >
        No decision episodes yet. Start a Decision in Canvas and it will appear here.
      </div>
      <button
        v-for="item in twinStore.recentDecisions"
        :key="item.episode.id"
        class="compact-row"
        :class="{ highlighted: item.episode.id === routeDecisionId }"
        @click="twinStore.activeTab = 'decisions'"
      >
        <span
          class="row-dot"
          :class="decisionState(item)"
        />
        <span class="row-title">{{ item.episode.decision }}</span>
        <span class="row-chips">
          <span
            v-for="chip in decisionChips(item)"
            :key="chip.id"
            class="chip"
            :class="chip.cls"
          >{{ chip.label }}</span>
        </span>
      </button>
    </section>

    <section class="workspace-band">
      <div class="band-header">
        <h2>Current Action Gaps</h2>
        <button
          class="text-button"
          @click="twinStore.activeTab = 'action_gaps'"
        >
          Review
        </button>
      </div>
      <div
        v-if="twinStore.topActionGaps.length === 0"
        class="empty-panel"
      >
        No action gaps found.
      </div>
      <button
        v-for="gap in twinStore.topActionGaps"
        :key="gap.id"
        class="compact-row"
        @click="twinStore.activeTab = 'action_gaps'"
      >
        <span class="row-dot is-pending" />
        <span class="row-title">{{ gap.decision_risk }}</span>
        <span class="row-chips">
          <span class="chip">{{ formatPercent(gap.confidence) }}</span>
        </span>
      </button>
    </section>
  </section>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { useTwinStore } from '@/stores/twin'
import { decisionState, decisionChips, formatPercent } from '@/utils/twinFormat'

const twinStore = useTwinStore()
const route = useRoute() || { query: {} }
const routeDecisionId = computed(() => String(route.query?.decision || ''))
</script>
