<template>
  <section>
    <h3>Saved prediction batches</h3>
    <p>
      Each decision runs three comparisons with the same local model and settings. A batch freezes
      its development evidence; start a new batch after changing that evidence.
    </p>
    <div class="evidence-filters">
      <label>Batch for new decisions<select
        v-model="store.batchId"
        :disabled="store.busy"
      >
        <option value="">Create on first prediction</option>
        <option
          v-for="batch in store.predictionHistory.batches"
          :key="batch.id"
          :value="batch.id"
        >
          {{ batch.created_at }} · {{ batch.model }} · {{ batch.id.slice(0, 8) }}
        </option>
        <option
          v-if="
            store.batchId &&
              !store.predictionHistory.batches.some((batch) => batch.id === store.batchId)
          "
          :value="store.batchId"
        >
          New batch · {{ store.batchId.slice(0, 8) }}
        </option>
      </select></label>
      <button
        class="btn btn-secondary"
        :disabled="store.busy"
        @click="store.batchId = crypto.randomUUID()"
      >
        New batch
      </button>
      <button
        class="btn btn-secondary"
        :disabled="store.busy"
        @click="store.loadPredictions"
      >
        Refresh saved predictions
      </button>
      <button
        class="btn btn-secondary"
        :disabled="store.busy || !store.predictionHistory.records.length"
        @click="download"
      >
        Export prediction ledger JSON
      </button>
    </div>
    <label>Resume a saved decision<select
      :value="store.prediction?.id || ''"
      :disabled="store.busy"
      @change="resume($event.target.value)"
    >
      <option value="">Select a saved decision</option>
      <option
        v-for="record in store.predictionHistory.records"
        :key="record.id"
        :value="record.id"
      >
        {{ record.request?.domain === 'everyday' ? 'Everyday' : 'Product / project' }} ·
        {{ record.request?.situation || record.id }} ·
        {{ record.sealed ? 'sealed' : record.status }}
      </option>
    </select></label>
    <button
      v-if="store.prediction?.status === 'pending' && !store.prediction.human_choice"
      class="btn btn-primary"
      :disabled="store.busy"
      @click="resumePending"
    >
      Resume pending comparisons
    </button>
    <p v-if="!store.predictionHistory.records.length">
      No predictions saved yet.
    </p>
    <details v-if="store.predictionHistory.records.length">
      <summary>Capture and review counts by domain</summary>
      <p>
        Comparison counts include revealed records only. Judgements are human assessments; failed
        and abstained comparisons remain separate. Sealed predictions are excluded from judgement
        counts.
      </p>
      <div class="evidence-table-scroll">
        <table>
          <thead>
            <tr>
              <th>Domain</th>
              <th>Decisions</th>
              <th>Choices recorded</th>
              <th>Sealed</th>
              <th>Revealed comparisons</th>
              <th>Human judged</th>
              <th>Agree</th>
              <th>Disagree</th>
              <th>Ambiguous</th>
              <th>Abstained</th>
              <th>Failed</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in summary"
              :key="row.domain"
            >
              <th>{{ row.domain === 'everyday' ? 'Everyday' : 'Product / project' }}</th>
              <td>{{ row.records }}</td>
              <td>{{ row.choices }}</td>
              <td>{{ row.sealed }}</td>
              <td>{{ row.comparisons }}</td>
              <td>{{ row.judged }}</td>
              <td>{{ row.agree }}</td>
              <td>{{ row.disagree }}</td>
              <td>{{ row.ambiguous }}</td>
              <td>{{ row.abstained }}</td>
              <td>{{ row.failed }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </details>
  </section>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
import { publicPredictionLedger, predictionCaptureSummary } from '@/utils/evidencePredictions'
const store = useEvidenceStore()
const crypto = globalThis.crypto
const summary = computed(() => predictionCaptureSummary(store.predictionHistory.records))
onMounted(store.loadPredictions)
function resume(id) {
  const record = store.predictionHistory.records.find((item) => item.id === id)
  if (record) store.selectPrediction(record)
}
async function resumePending() {
  const record = store.prediction
  await store.predict({ ...record.request, id: record.id, batch_id: record.batch_id })
}
async function download() {
  store.busy = true
  store.error = ''
  try {
    const filename = `grafyn-predictions-${new Date().toISOString().slice(0, 10)}.json`
    const blob = new Blob(
      [JSON.stringify(publicPredictionLedger(store.predictionHistory), null, 2)],
      { type: 'application/json' }
    )
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    link.click()
    URL.revokeObjectURL(url)
    store.notice = `Prediction ledger downloaded as ${filename}`
  } catch (error) {
    store.error = error?.message || String(error)
  } finally {
    store.busy = false
  }
}
</script>
