<template>
  <div class="twin-workspace">
    <header class="workspace-header">
      <div class="header-links">
        <router-link to="/">
          Notes
        </router-link>
        <router-link to="/canvas">
          Canvas
        </router-link>
      </div>
      <div class="header-title">
        <h1>Twin Workspace</h1>
        <span>{{ twinStore.healthSummary }}</span>
      </div>
      <div class="header-actions">
        <button
          class="btn btn-secondary"
          :disabled="twinStore.runningTwinInference"
          @click="twinStore.runTwinInference"
        >
          {{ twinStore.runningTwinInference ? 'Running...' : 'Run Records' }}
        </button>
        <button
          class="btn btn-primary"
          :disabled="twinStore.runningConstitutionInference"
          @click="twinStore.runConstitutionInference"
        >
          {{ twinStore.runningConstitutionInference ? 'Running...' : 'Run Constitution' }}
        </button>
      </div>
    </header>

    <div class="workspace-body">
      <nav class="workspace-rail">
        <template
          v-for="group in navGroups"
          :key="group.id"
        >
          <span
            v-if="group.label"
            class="rail-label"
          >{{ group.label }}</span>
          <button
            v-for="tab in group.tabs"
            :key="tab.id"
            class="tab-button"
            :class="{ active: twinStore.activeTab === tab.id }"
            @click="twinStore.activeTab = tab.id"
          >
            <span>{{ tab.label }}</span>
            <strong v-if="tab.count !== null">{{ tab.count }}</strong>
          </button>
        </template>
      </nav>

      <main class="workspace-main">
        <section
          v-if="twinStore.activeTab === 'overview'"
          class="tab-panel"
        >
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

        <TwinConstitutionTab v-else-if="twinStore.activeTab === 'constitution'" />

        <TwinActionGapsTab v-else-if="twinStore.activeTab === 'action_gaps'" />

        <TwinDecisionsTab v-else-if="twinStore.activeTab === 'decisions'" />

        <section
          v-else-if="twinStore.activeTab === 'memory'"
          class="tab-panel memory-grid"
        >
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

        <TwinSetupTab v-else-if="twinStore.activeTab === 'setup'" />

        <TwinGuideTab v-else-if="twinStore.activeTab === 'guide'" />

        <TwinConfigTab v-else-if="twinStore.activeTab === 'config'" />
      </main>
    </div>

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

    <div
      v-if="twinStore.message"
      class="save-toast"
      :class="twinStore.message.type"
    >
      {{ twinStore.message.text }}
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { useTwinStore } from '@/stores/twin'
import {
  decisionState,
  decisionChips,
  statusLabel,
  kindLabel,
  formatPercent,
  formatDate,
  eventLabel
} from '@/utils/twinFormat'
import ReviewActions from '@/components/twin/ReviewActions.vue'
import TwinActionGapsTab from '@/components/twin/TwinActionGapsTab.vue'
import TwinConstitutionTab from '@/components/twin/TwinConstitutionTab.vue'
import TwinDecisionsTab from '@/components/twin/TwinDecisionsTab.vue'
import TwinGuideTab from '@/components/twin/TwinGuideTab.vue'
import TwinSetupTab from '@/components/twin/TwinSetupTab.vue'
import TwinConfigTab from '@/components/twin/TwinConfigTab.vue'
import '@/components/twin/twin-workspace.css'

const twinStore = useTwinStore()

const navGroups = computed(() => [
  {
    id: 'home',
    label: null,
    tabs: [{ id: 'overview', label: 'Overview', count: null }]
  },
  {
    id: 'work',
    label: 'Work',
    tabs: [{ id: 'decisions', label: 'Decisions', count: twinStore.decisions.length }]
  },
  {
    id: 'review',
    label: 'Review',
    tabs: [
      { id: 'memory', label: 'Memory Review', count: twinStore.pendingReviewCount },
      { id: 'constitution', label: 'Constitution', count: twinStore.constitutionItems.length },
      { id: 'action_gaps', label: 'Action Gaps', count: twinStore.actionGaps.length }
    ]
  },
  {
    id: 'tune',
    label: 'Configure',
    tabs: [
      { id: 'setup', label: 'Setup', count: null },
      { id: 'config', label: 'Config', count: null }
    ]
  },
  {
    id: 'help',
    label: 'Help',
    tabs: [{ id: 'guide', label: 'Guide', count: null }]
  }
])

const recordStates = ['candidate', 'auto_promoted', 'endorsed', 'private', 'no_train', 'rejected']
const route = useRoute() || { query: {} }
// Matches the original per-mount ref initialization: every time this view
// mounts, the active tab is (re)computed from the current route query.
twinStore.activeTab = route.query?.decision ? 'decisions' : 'overview'

const routeDecisionId = computed(() => String(route.query?.decision || ''))

onMounted(twinStore.loadWorkspace)
</script>

