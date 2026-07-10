<template>
  <section class="tab-panel config-panel">
    <div class="panel-header">
      <div>
        <h2>Decision Mirror Config</h2>
        <span>Presets change retrieval and scoring weights without hiding raw sub-scores.</span>
      </div>
      <button
        class="btn btn-secondary btn-sm"
        :disabled="twinStore.exportingBenchmark"
        @click="twinStore.exportDecisionBenchmark"
      >
        {{ twinStore.exportingBenchmark ? 'Exporting...' : 'Export Benchmark' }}
      </button>
    </div>

    <section class="workspace-band config-card">
      <label class="config-row">
        <span>Preset</span>
        <select v-model="twinStore.configDraft.preset">
          <option
            v-for="preset in decisionMirrorPresets"
            :key="preset.value"
            :value="preset.value"
          >
            {{ preset.label }}
          </option>
        </select>
      </label>
      <label class="config-toggle">
        <input
          v-model="twinStore.configDraft.advanced_enabled"
          type="checkbox"
        >
        <span>Advanced</span>
      </label>
      <div class="config-actions">
        <button
          class="btn btn-primary"
          :disabled="twinStore.savingConfig"
          @click="twinStore.saveDecisionMirrorConfig"
        >
          {{ twinStore.savingConfig ? 'Saving...' : 'Save Config' }}
        </button>
        <button
          class="btn btn-secondary"
          :disabled="twinStore.savingConfig"
          @click="twinStore.resetDecisionMirrorConfig"
        >
          Reset
        </button>
      </div>
    </section>

    <section
      v-if="twinStore.configDraft.advanced_enabled"
      class="workspace-band config-card"
    >
      <div class="panel-header compact">
        <div>
          <h2>Advanced Weights</h2>
          <span>0 ignores the signal, 3 gives it strong priority.</span>
        </div>
      </div>
      <label
        v-for="weight in configWeightRows"
        :key="weight.key"
        class="weight-row"
      >
        <span>{{ weight.label }}</span>
        <input
          v-model.number="twinStore.configDraft.weights[weight.key]"
          type="range"
          min="0"
          max="3"
          step="0.05"
        >
        <strong>{{ formatWeight(twinStore.configDraft.weights[weight.key]) }}</strong>
      </label>
    </section>
  </section>
</template>

<script setup>
import { useTwinStore } from '@/stores/twin'
import { decisionMirrorPresets, configWeightRows, formatWeight } from '@/utils/twinFormat'

const twinStore = useTwinStore()
</script>
