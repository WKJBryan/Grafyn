<template>
  <section class="evidence-section">
    <h2>Decision prediction</h2>
    <p>
      Grounded tentative inferences may inform the answer. Assumptions and conditional branches
      remain visible in normal mode.
    </p>
    <EvidencePredictionLedger />
    <form @submit.prevent="predict">
      <fieldset :disabled="store.busy">
        <label>Domain<select v-model="domain">
          <option value="product_project">Product / project</option>
          <option value="everyday">Everyday life</option>
        </select></label>
        <label>Decision situation<textarea
          v-model="situation"
          rows="3"
          required
        /></label>
        <label>Available options — one per line<textarea
          v-model="options"
          rows="3"
          required
        />
        </label>
        <label class="inline-label"><input
          v-model="validation"
          type="checkbox"
        > Validation: keep the prediction hidden
          until I record my choice</label>
      </fieldset>
      <button
        class="btn btn-primary"
        :disabled="store.busy || !situation.trim() || optionList.length < 2"
      >
        {{ store.predicting ? 'Running comparisons…' : 'Run all three comparisons' }}
      </button>
    </form>
    <p
      v-if="store.predicting"
      role="status"
    >
      The same decision is running without personal evidence, with personal evidence, and with goal
      paths. Local inference can take several minutes.
    </p>
    <section
      v-if="result"
      aria-live="polite"
    >
      <p
        v-if="result.error"
        role="alert"
      >
        {{ result.error }}
      </p>
      <p v-if="result.sealed">
        Prediction sealed. Record your choice below before seeing it.
      </p>
      <template v-else>
        <h3 v-if="result.proposed_action">
          Likely choice
        </h3>
        <p>{{ result.proposed_action }}</p>
        <ul v-if="result.conditional_branches?.length">
          <li
            v-for="(branch, index) in result.conditional_branches"
            :key="index"
          >
            If {{ branch.condition }}: {{ branch.action }}
          </li>
        </ul>
        <details v-if="result.assumptions?.length">
          <summary>Assumptions</summary>
          <ul>
            <li
              v-for="assumption in result.assumptions"
              :key="assumption"
            >
              {{ assumption }}
            </li>
          </ul>
        </details>
        <details v-if="result.evidence_ids?.length">
          <summary>Evidence used</summary>
          <ul>
            <li
              v-for="id in result.evidence_ids"
              :key="id"
            >
              {{ id }}
            </li>
          </ul>
        </details>
      </template>
      <form
        v-if="questions.length && !result.human_choice && !hasClarified"
        @submit.prevent="clarify"
      >
        <label
          v-for="(question, index) in questions"
          :key="question"
        >{{ question }}<input v-model="answers[index]"></label>
        <button
          class="btn btn-secondary"
          :disabled="store.busy"
        >
          Use these clarifications
        </button>
      </form>
      <form
        v-if="!result.human_choice"
        @submit.prevent="recordChoice"
      >
        <label>Your actual choice<input
          v-model="choice"
          required
        ></label>
        <label>Why? Optional<textarea
          v-model="rationale"
          rows="2"
        /></label>
        <button
          class="btn btn-primary"
          :disabled="store.busy || !choice.trim()"
        >
          {{ result.sealed ? 'Record choice and reveal prediction' : 'Record my choice' }}
        </button>
      </form>
      <EvidencePredictionComparisons :record="result" />
    </section>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
import EvidencePredictionLedger from './EvidencePredictionLedger.vue'
import EvidencePredictionComparisons from './EvidencePredictionComparisons.vue'
const store = useEvidenceStore()
const situation = ref(store.predictionRequest?.situation || ''),
  options = ref(store.predictionRequest?.options?.join('\n') || ''),
  domain = ref(store.predictionRequest?.domain || 'product_project'),
  validation = ref(store.predictionRequest?.validation || false)
const choice = ref(''),
  rationale = ref(''),
  answers = ref([])
const result = computed(() => store.prediction)
watch(
  () => store.predictionRequest,
  (request) => {
    if (!request) return
    situation.value = request.situation || ''
    options.value = request.options?.join('\n') || ''
    domain.value = request.domain || 'product_project'
    validation.value = !!request.validation
    answers.value = []
    choice.value = ''
    rationale.value = ''
  }
)
const hasClarified = computed(() => !!store.predictionRequest?.clarifications?.length)
const optionList = computed(() =>
  options.value
    .split('\n')
    .map((v) => v.trim())
    .filter(Boolean)
)
const questions = computed(() => (result.value?.questions || []).slice(0, 2))
async function predict() {
  answers.value = []
  await store.predict({
    batch_id: store.batchId || null,
    domain: domain.value,
    situation: situation.value,
    options: optionList.value,
    clarifications: [],
    validation: validation.value,
  })
}
async function clarify() {
  const clarifications = questions.value.map((question, index) => ({
    question,
    answer: answers.value[index] || '',
  }))
  await store.predict({
    ...store.predictionRequest,
    id: result.value.id,
    clarifications,
    validation: result.value.validation,
  })
}
async function recordChoice() {
  await store.recordChoice({
    prediction_id: result.value.id,
    choice: choice.value,
    rationale: rationale.value || null,
  })
}
</script>
