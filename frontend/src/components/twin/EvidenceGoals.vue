<template>
  <section class="evidence-section">
    <h2>Goals and changing priorities</h2>
    <p>
      Keep several goals at once. Unknown quantities, deadlines, and observations remain unknown.
      Editing creates a revision.
    </p>
    <div class="evidence-actions">
      <button class="btn btn-secondary" @click="edit()">New goal</button>
    </div>
    <ul class="evidence-list">
      <li v-for="goal in currentGoals" :key="goal.id">
        <button class="btn btn-secondary" @click="edit(goal)">
          {{ goal.label }} · {{ goal.status }} · revision {{ goal.revision }}
        </button>
        <p>{{ goal.definition }}</p>
        <span v-for="criterion in goal.criteria" :key="criterion.id"
          >{{ criterion.metric || 'Metric unresolved' }}:
          {{ criterion.target ?? 'Target unknown' }} ·
          {{ criterion.deadline || criterion.duration || 'Deadline unknown' }} · observed
          {{ criterion.observed_progress ?? 'unknown' }}</span
        >
      </li>
    </ul>
    <p v-if="!currentGoals.length">
      No goals recorded yet. You can begin qualitatively and add grounded criteria later.
    </p>
    <form v-if="draft" @submit.prevent="save">
      <fieldset :disabled="store.busy">
        <label>Goal name<input v-model="draft.label" required /></label>
        <label
          >What does success mean in your words?<textarea
            v-model="draft.definition"
            rows="3"
            required
          />
        </label>
        <label>Who benefits?<input v-model="draft.beneficiary" /></label>
        <label>Role / context / scope<input v-model="draft.scope" /></label>
        <label
          >Status<select v-model="draft.status">
            <option v-for="state in ['active', 'paused', 'achieved', 'abandoned']" :key="state">
              {{ state }}
            </option>
          </select></label
        >
        <label
          >Context-specific priority<textarea v-model="draft.contextual_priority" rows="2" />
        </label>
        <label
          >Constraints and unacceptable costs — one per line<textarea
            v-model="constraints"
            rows="2"
          />
        </label>
        <fieldset>
          <legend>Competing goals</legend>
          <label
            v-for="goal in currentGoals.filter((g) => g.id !== draft.id)"
            :key="goal.id"
            class="inline-label"
            ><input v-model="draft.competing_goal_ids" type="checkbox" :value="goal.id" />
            {{ goal.label }}</label
          >
        </fieldset>
        <section v-for="(criterion, index) in draft.criteria" :key="index" class="criterion">
          <h3>Criterion {{ index + 1 }}</h3>
          <label v-for="field in textFields" :key="field.key"
            >{{ field.label
            }}<input v-model="criterion[field.key]" placeholder="Unknown / not supplied"
          /></label>
          <label v-for="field in numericFields" :key="field.key"
            >{{ field.label
            }}<input v-model="criterion[field.key]" type="number" step="any" placeholder="Unknown"
          /></label>
          <label class="inline-label"
            ><input v-model="criterion.is_proxy" type="checkbox" /> This is an agreed proxy</label
          >
          <button class="btn btn-secondary" type="button" @click="draft.criteria.splice(index, 1)">
            Remove criterion from this revision
          </button>
        </section>
        <button
          class="btn btn-secondary"
          type="button"
          @click="draft.criteria.push({ field_provenance: {}, receipts: [], is_proxy: false })"
        >
          Add measurable criterion (optional)
        </button>
        <label
          >When did this change take effect? Optional date/time with timezone<input
            v-model="draft.effective_at"
            type="text"
            placeholder="For example: 2026-09-05T12:00:00+08:00"
        /></label>
        <label>Reason for this revision<textarea v-model="draft.reason" rows="2" /></label>
      </fieldset>
      <div class="evidence-actions">
        <button class="btn btn-primary" :disabled="store.busy">Save goal revision</button
        ><button class="btn btn-secondary" type="button" @click="draft = null">Cancel</button>
      </div>
    </form>
    <details v-if="store.snapshot.goals.length">
      <summary>Revision history</summary>
      <ul>
        <li v-for="goal in store.snapshot.goals" :key="`${goal.id}-${goal.revision}`">
          {{ goal.label }} · revision {{ goal.revision }} · {{ goal.status }} · effective
          {{ goal.effective_at || 'unknown' }} · recorded {{ goal.recorded_at }}
          <p>{{ goal.reason || 'No reason supplied' }}</p>
        </li>
      </ul>
    </details>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
const store = useEvidenceStore()
const draft = ref(null),
  constraints = ref('')
const currentGoals = computed(() =>
  Object.values(
    Object.fromEntries(
      [...store.snapshot.goals]
        .sort((a, b) => a.revision - b.revision)
        .filter((g) => !g.invalidated)
        .map((g) => [g.id, g])
    )
  )
)
const textFields = [
  ['metric', 'Metric'],
  ['counting_rule', 'What counts, including deduplication'],
  ['unit', 'Unit'],
  ['population', 'Population'],
  ['denominator', 'Denominator for a rate'],
  ['comparator', 'Comparator (at least / at most / range)'],
  ['baseline_observed_at', 'Baseline observation date'],
  ['start_anchor', 'Start date or event'],
  ['deadline', 'Deadline'],
  ['duration', 'Duration'],
  ['sustained_period', 'Sustained success period'],
  ['measurement_source', 'Measurement source'],
  ['observation_schedule', 'Observation schedule'],
].map(([key, label]) => ({ key, label }))
const numericFields = [
  ['baseline', 'Observed baseline'],
  ['target', 'Target'],
  ['upper_target', 'Upper target for a range'],
  ['observed_progress', 'Observed progress'],
].map(([key, label]) => ({ key, label }))
function edit(goal) {
  draft.value = goal
    ? JSON.parse(JSON.stringify(goal))
    : {
        id: '',
        subject_id: store.snapshot.subject_id,
        label: '',
        definition: '',
        beneficiary: '',
        scope: '',
        status: 'active',
        criteria: [],
        constraints: [],
        competing_goal_ids: [],
        contextual_priority: null,
        effective_at: null,
        reason: '',
        receipts: [],
        review_status: 'confirmed',
      }
  constraints.value = draft.value.constraints.join('\n')
}
async function save() {
  const request = JSON.parse(JSON.stringify(draft.value))
  request.constraints = constraints.value
    .split('\n')
    .map((v) => v.trim())
    .filter(Boolean)
  request.effective_at = request.effective_at || null
  request.contextual_priority = request.contextual_priority || null
  request.review_status = 'confirmed'
  for (const criterion of request.criteria) {
    for (const { key } of numericFields)
      criterion[key] =
        criterion[key] === '' || criterion[key] == null ? null : Number(criterion[key])
    for (const { key } of textFields) criterion[key] = criterion[key] || null
    for (const { key } of [...numericFields, ...textFields])
      criterion.field_provenance[key] = criterion[key] == null ? 'unknown' : 'confirmed'
  }
  await store.saveGoal(request)
  if (!store.error) draft.value = null
}
</script>
