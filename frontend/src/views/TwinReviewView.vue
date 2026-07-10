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
        <TwinOverviewTab v-if="twinStore.activeTab === 'overview'" />

        <TwinConstitutionTab v-else-if="twinStore.activeTab === 'constitution'" />

        <TwinActionGapsTab v-else-if="twinStore.activeTab === 'action_gaps'" />

        <TwinDecisionsTab v-else-if="twinStore.activeTab === 'decisions'" />

        <TwinMemoryTab v-else-if="twinStore.activeTab === 'memory'" />

        <TwinSetupTab v-else-if="twinStore.activeTab === 'setup'" />

        <TwinGuideTab v-else-if="twinStore.activeTab === 'guide'" />

        <TwinConfigTab v-else-if="twinStore.activeTab === 'config'" />
      </main>
    </div>

    <EvidenceDrawer />

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
import EvidenceDrawer from '@/components/twin/EvidenceDrawer.vue'
import TwinOverviewTab from '@/components/twin/TwinOverviewTab.vue'
import TwinActionGapsTab from '@/components/twin/TwinActionGapsTab.vue'
import TwinConstitutionTab from '@/components/twin/TwinConstitutionTab.vue'
import TwinDecisionsTab from '@/components/twin/TwinDecisionsTab.vue'
import TwinMemoryTab from '@/components/twin/TwinMemoryTab.vue'
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

const route = useRoute() || { query: {} }
// Matches the original per-mount ref initialization: every time this view
// mounts, the active tab is (re)computed from the current route query.
twinStore.activeTab = route.query?.decision ? 'decisions' : 'overview'

onMounted(twinStore.loadWorkspace)
</script>

