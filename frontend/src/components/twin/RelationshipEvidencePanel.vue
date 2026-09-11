<template>
  <section
    class="evidence-section relationship-panel"
    aria-label="Relationship evidence"
  >
    <div class="evidence-actions">
      <h3>{{ relationship.relation.replaceAll('_', ' ') }}</h3>
      <button
        class="btn btn-secondary"
        @click="$emit('close')"
      >
        Close evidence
      </button>
    </div>
    <p v-if="relationship.explanation">
      {{ relationship.explanation }}
    </p>
    <p>
      {{ relationship.review_status }} · {{ relationship.provenance }} ·
      {{
        relationship.similarity == null
          ? 'Unscored / neutral length'
          : `Candidate relatedness ${relationship.similarity.toFixed(2)}`
      }}
    </p>
    <section
      v-if="relationship.assessment?.result"
      aria-label="Contextual assessment"
    >
      <p>
        Contextual verdict: {{ relationship.assessment.result.verdict.replaceAll('_', ' ') }} ·
        {{ directionLabel(relationship.assessment.result.direction) }}
      </p>
      <p>{{ relationship.assessment.result.explanation }}</p>
      <p>
        Model {{ relationship.assessment.model_version }} · prompt
        {{ relationship.assessment.prompt_version }}
      </p>
      <p v-if="relationship.assessment.result.conditions?.length">
        Assessment conditions: {{ relationship.assessment.result.conditions.join('; ') }}
      </p>
      <figure>
        <figcaption>Assessment LEFT (input order)</figcaption>
        <blockquote>{{ relationship.assessment.result.from_quote }}</blockquote>
      </figure>
      <figure>
        <figcaption>Assessment RIGHT (input order)</figcaption>
        <blockquote>{{ relationship.assessment.result.to_quote }}</blockquote>
      </figure>
    </section>
    <p v-else-if="relationship.assessment?.error">
      Contextual assessment failed: {{ relationship.assessment.error }}
    </p>
    <p v-else-if="relationship.provenance === 'local_semantic_similarity'">
      Awaiting contextual assessment.
    </p>
    <p v-if="relationship.causal_basis">
      Effect basis: {{ relationship.causal_basis.replaceAll('_', ' ') }}. Review does not establish
      a measured causal effect.
    </p>
    <figure
      v-for="(receipt, index) in [relationship.from_receipt, relationship.to_receipt]"
      :key="index"
    >
      <figcaption>
        {{ index === 0 ? 'From' : 'To' }} · {{ receipt.locator || receipt.source_id }} · source
        revision {{ receipt.source_revision }}
      </figcaption>
      <blockquote>{{ receipt.quote }}</blockquote>
      <small>Source {{ receipt.source_id }} · bytes {{ receipt.start }}–{{ receipt.end }} · recorded
        {{ sourceTime(receipt.source_id) }}</small>
    </figure>
    <p v-if="relationship.conditions?.length">
      Conditions: {{ relationship.conditions.join('; ') }}
    </p>
    <p v-if="relationship.causal_basis">
      Delay: {{ relationship.delay || 'unknown' }} · magnitude:
      {{ relationship.magnitude || 'unknown' }} · criterion:
      {{ relationship.goal_criterion_id || 'unresolved' }}
    </p>
    <div class="evidence-actions">
      <button
        class="btn btn-primary"
        :disabled="store.busy"
        @click="review('confirmed')"
      >
        Accept
      </button><button
        class="btn btn-secondary"
        :disabled="store.busy"
        @click="review('rejected')"
      >
        Reject
      </button>
    </div>
    <form @submit.prevent="review('confirmed', correction)">
      <label>Correct relation<select v-model="correction">
        <option
          v-for="relation in relations"
          :key="relation"
          :disabled="correctionDisabled(relation)"
        >
          {{ relation }}
        </option>
      </select></label><button
        class="btn btn-secondary"
        :disabled="store.busy"
      >
        Save correction
      </button>
    </form>
    <p v-if="!hasScopeConditions">
      Equivalent and conflicts corrections require scope conditions in the supporting evidence.
    </p>
    <p v-if="!hasEffectBasis">
      Effect corrections require an existing direction and effect basis.
    </p>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
const props = defineProps({ relationship: { type: Object, required: true } })
const emit = defineEmits(['close'])
const store = useEvidenceStore()
const correction = ref(props.relationship.relation)
watch(
  () => props.relationship,
  (value) => {
    correction.value = value.relation
  }
)
const relations = [
  'related',
  'equivalent',
  'conflicts',
  'supports',
  'contradicts',
  'expands',
  'questions',
  'answers',
  'example',
  'part_of',
  'enables',
  'inhibits',
  'requires',
  'contributes_to',
]
const scopeRequired = ['equivalent', 'conflicts']
const effects = ['enables', 'inhibits', 'requires', 'contributes_to']
const hasScopeConditions = computed(
  () =>
    !!props.relationship.conditions?.length ||
    !!props.relationship.assessment?.result?.conditions?.length
)
const hasEffectBasis = computed(
  () =>
    props.relationship.directed === true &&
    ['target_stated_belief', 'extracted_hypothesis'].includes(props.relationship.causal_basis)
)
const correctionDisabled = (relation) =>
  (scopeRequired.includes(relation) && !hasScopeConditions.value) ||
  (effects.includes(relation) && !hasEffectBasis.value)
const directionLabel = (direction) =>
  ({
    symmetric: 'symmetric',
    left_to_right: 'LEFT → RIGHT (assessment input order)',
    right_to_left: 'RIGHT → LEFT (assessment input order)',
  })[direction] || direction
const sourceTime = (id) =>
  store.snapshot.sources.find((source) => source.id === id)?.recorded_at || 'unknown'
async function review(status, relation = null) {
  await store.reviewRelationship({ id: props.relationship.id, status, relation })
  if (!store.error) emit('close')
}
</script>
