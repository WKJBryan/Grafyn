<template>
  <section class="tab-panel setup-grid">
    <section class="identity-section">
      <div>
        <h2>Twin Identity</h2>
        <span>Name and role are required before Twin Simulation can run.</span>
      </div>
      <label class="identity-field">
        <span>Name</span>
        <input
          v-model="twinStore.setupDraft.twin_name"
          aria-label="Twin name"
          type="text"
          placeholder="Who is this twin?"
        >
      </label>
      <label class="identity-field">
        <span>Role / context</span>
        <input
          v-model="twinStore.setupDraft.twin_role"
          aria-label="Twin role"
          type="text"
          placeholder="What role should this twin reason from?"
        >
      </label>
      <SetupField
        v-model="twinStore.setupDraft.source_boundaries"
        title="Source Boundaries"
      />
    </section>
    <div class="setup-section-heading">
      <h2>Operating Priors</h2>
      <span>Values, taste, constraints, somatic cues, and action tendencies.</span>
    </div>
    <SetupField
      v-model="twinStore.setupDraft.values"
      title="Values"
    />
    <SetupField
      v-model="twinStore.setupDraft.tastes"
      title="Taste"
    />
    <SetupField
      v-model="twinStore.setupDraft.constraints"
      title="Constraints"
    />
    <SetupField
      v-model="twinStore.setupDraft.somatic_cues"
      title="Somatic Cues"
    />
    <SetupField
      v-model="twinStore.setupDraft.action_tendencies"
      title="Action Tendencies"
    />
    <div class="setup-actions">
      <button
        class="btn btn-primary"
        :disabled="twinStore.savingSetup"
        @click="twinStore.saveSetup"
      >
        {{ twinStore.savingSetup ? 'Saving...' : 'Save Setup' }}
      </button>
    </div>
  </section>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import SetupField from './SetupField.vue'

const twinStore = useTwinStore()
</script>
