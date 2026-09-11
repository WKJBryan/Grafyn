<template>
  <section v-if="!record.sealed && record.comparisons?.length">
    <h3>Comparison records</h3>
    <p v-if="record.human_choice">
      Recorded choice: {{ record.human_choice }}. Judge whether each prediction is equivalent to
      your choice; combined and outside options are allowed.
    </p>
    <article v-for="comparison in record.comparisons" :key="keyOf(comparison)" class="criterion">
      <h4>
        {{ conditionLabels[comparison.condition] || comparison.condition }} ·
        {{
          comparison.stage === 'before_clarification'
            ? 'Before clarification'
            : 'After clarification'
        }}
      </h4>
      <p>Status: {{ comparison.status }}</p>
      <p v-if="comparison.error" role="alert">{{ comparison.error }}</p>
      <p v-if="comparison.forecast?.insufficient_evidence">Insufficient evidence — abstained.</p>
      <p>{{ comparison.forecast?.proposed_action }}</p>
      <ul v-if="comparison.forecast?.conditional_branches?.length">
        <li v-for="(branch, index) in comparison.forecast.conditional_branches" :key="index">
          If {{ branch.condition }}: {{ branch.action }}
        </li>
      </ul>
      <details v-if="comparison.forecast">
        <summary>Assumptions and evidence</summary>
        <ul>
          <li v-for="assumption in comparison.forecast.assumptions" :key="assumption">
            {{ assumption }}
          </li>
        </ul>
        <p>Evidence IDs: {{ comparison.forecast.evidence_ids?.join(', ') || 'None supplied' }}</p>
        <p v-for="goal in comparison.context?.goals || []" :key="`${goal.id}-${goal.revision}`">
          Goal {{ goal.label }} · revision {{ goal.revision }}
        </p>
      </details>
      <label v-if="record.human_choice"
        >Your equivalence judgement<select
          v-model="judgements[keyOf(comparison)]"
          :disabled="store.busy || comparison.status !== 'completed'"
        >
          <option value="">Not reviewed</option>
          <option value="agree">Agree — equivalent choice</option>
          <option value="disagree">Disagree — different choice</option>
          <option value="ambiguous">Ambiguous — cannot determine</option>
        </select></label
      >
    </article>
    <button v-if="record.human_choice" class="btn btn-primary" :disabled="store.busy" @click="save">
      Save my judgements
    </button>
  </section>
</template>

<script setup>
import { ref, watch } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
import { conditionLabels } from '@/utils/evidencePredictions'
const props = defineProps({ record: { type: Object, required: true } })
const store = useEvidenceStore()
const judgements = ref({ ...props.record.adjudications })
watch(
  () => props.record,
  (record) => {
    judgements.value = { ...record.adjudications }
  }
)
const keyOf = (comparison) => `${comparison.condition}:${comparison.stage}`
async function save() {
  const adjudications = Object.fromEntries(
    Object.entries(judgements.value).filter(([, value]) => value)
  )
  await store.recordChoice({
    prediction_id: props.record.id,
    choice: props.record.human_choice,
    rationale: props.record.human_rationale || null,
    adjudications,
  })
}
</script>
