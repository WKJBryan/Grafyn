<template>
  <section class="tab-panel evidence-workspace">
    <header class="evidence-section">
      <h2>{{ store.scope === 'pilot' ? 'Bryan’s isolated pilot' : 'Current vault evidence' }}</h2>
      <label>Evidence scope<select
        :value="store.scope"
        :disabled="store.busy"
        @change="store.switchScope($event.target.value)"
      >
        <option value="pilot">Bryan pilot</option>
        <option value="current">Current vault</option>
      </select></label>
      <p v-if="store.scope === 'pilot'">
        Guided interviews across product / project decisions and everyday life. This pilot has its
        own evidence and does not change the current twin or its vault.
      </p>
      <p v-else>
        Inspect evidence from the current app vault. This selector does not switch the app’s vault.
        Target:
        {{
          targetConfigured
            ? store.snapshot.subject_name || store.snapshot.subject_id
            : 'not mapped'
        }}.
      </p>
      <div class="evidence-actions">
        <button
          v-if="store.scope === 'pilot'"
          class="btn btn-secondary"
          :disabled="store.busy"
          @click="store.createPilot"
        >
          Set up / open pilot
        </button><button
          class="btn btn-secondary"
          :disabled="store.busy"
          @click="store.load"
        >
          Refresh
        </button><button
          v-if="store.snapshot?.subject_id"
          class="btn btn-primary"
          :disabled="store.busy"
          @click="store.processJobs"
        >
          Process evidence
        </button>
        <button
          v-if="embeddingModelMissing"
          class="btn btn-secondary"
          :disabled="store.busy"
          @click="store.installEmbeddings"
        >
          {{
            store.installingEmbeddings
              ? 'Installing embeddinggemma…'
              : 'Install embeddinggemma locally'
          }}
        </button>
      </div>
      <p
        v-if="store.installingEmbeddings"
        role="status"
      >
        Downloading the embedding model into local Ollama. This can take several minutes.
      </p>
      <p
        v-if="store.busy"
        role="status"
      >
        Working…
      </p>
      <p
        v-if="store.error"
        role="alert"
      >
        {{ store.error }}
      </p>
      <p
        v-if="store.notice"
        role="status"
      >
        {{ store.notice }}
      </p>
      <details v-if="store.snapshot?.jobs.length">
        <summary>Processing status</summary>
        <ul>
          <li
            v-for="job in store.snapshot.jobs"
            :key="job.id"
          >
            {{ job.source_id }} · {{ job.status }}<span v-if="job.error"> — {{ job.error }}</span>
          </li>
        </ul>
      </details>
    </header>
    <template v-if="store.snapshot && (targetConfigured || tab === 'relationships')">
      <EvidenceInterview
        v-if="tab === 'interview'"
        :key="store.snapshot.subject_id"
      />
      <EvidenceGoals v-else-if="tab === 'goals'" />
      <EvidenceRelationships v-else-if="tab === 'relationships'" />
      <EvidencePrediction v-else-if="tab === 'prediction'" />
    </template>
    <p v-else-if="store.scope === 'current' && !store.busy">
      Map the target person explicitly during import before recording interviews, goals, or
      predictions. You can inspect document relationships in Evidence Graph.
      <router-link to="/import">
        Import and map a target person
      </router-link>
    </p>
    <p v-else-if="!store.busy">
      Set up the pilot to begin. No sample decisions or goals have been added.
    </p>
  </section>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
import EvidenceInterview from './EvidenceInterview.vue'
import EvidenceGoals from './EvidenceGoals.vue'
import EvidenceRelationships from './EvidenceRelationships.vue'
import EvidencePrediction from './EvidencePrediction.vue'
import './evidence-workspace.css'
defineProps({ tab: { type: String, required: true } })
const store = useEvidenceStore()
const targetConfigured = computed(
  () => !!store.snapshot?.subject_id && store.snapshot.subject_id !== 'unconfigured'
)
const embeddingModelMissing = computed(() =>
  /^pending:.*embeddinggemma.*(?:not installed|missing model)/i.test(
    store.snapshot?.embedding_status || ''
  )
)
onMounted(() => {
  if (!store.snapshot) store.load()
})
</script>
