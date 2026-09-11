<template>
  <section class="evidence-section">
    <h2>A real decision, in your words</h2>
    <p>
      Start with 10–15 minutes. Save at any step and return later. Product and project choices and
      everyday choices both count. Leave anything you do not know blank.
    </p>
    <p aria-live="polite">Step {{ draft.step + 1 }} of 4 — {{ steps[draft.step] }}</p>
    <form @submit.prevent="advance">
      <fieldset :disabled="store.busy">
        <template v-if="draft.step === 0">
          <label
            >Domain<select v-model="draft.domain">
              <option value="product_project">Product / project</option>
              <option value="everyday">Everyday life</option>
            </select></label
          >
          <label>What was the situation?<textarea v-model="draft.situation" rows="4" /></label>
          <label
            >What options did you consider? One per line.<textarea v-model="options" rows="3" />
          </label>
        </template>
        <template v-if="draft.step === 1">
          <label>What did you want to happen?<textarea v-model="draft.wanted" rows="3" /></label>
          <label
            >What did you expect would actually happen?<textarea
              v-model="draft.expected"
              rows="3"
            />
          </label>
          <label
            >How did that expected result affect what you wanted? Optional<select
              v-model="draft.expected_goal_relation"
            >
              <option :value="null">Unsure / not supplied</option>
              <option value="contributes_to">Helped</option>
              <option value="inhibits">Hindered</option>
            </select></label
          >
          <label
            >What constraints mattered? One per line.<textarea v-model="constraints" rows="3" />
          </label>
          <label v-for="goal in currentGoals" :key="goal.id" class="inline-label"
            ><input v-model="draft.goal_ids" type="checkbox" :value="goal.id" />
            {{ goal.label }}</label
          >
        </template>
        <template v-if="draft.step === 2">
          <label
            >What did you choose? Leave blank if you have not chosen.<textarea
              v-model="draft.chosen"
              rows="3"
            />
          </label>
          <label
            >Which options did you reject? One per line.<textarea v-model="rejected" rows="3" />
          </label>
          <label
            >Why did you make that choice?<textarea v-model="draft.rationale" rows="3" />
          </label>
        </template>
        <template v-if="draft.step === 3">
          <label
            >What actually happened? Optional follow-up.<textarea v-model="draft.actual" rows="4" />
          </label>
          <p>
            Wanted, expected, chosen, and actual outcomes stay separate. Saving an unanswered
            situation does not turn it into an observed choice.
          </p>
          <details>
            <summary>Review your interview</summary>
            <dl>
              <template
                v-for="key in ['situation', 'wanted', 'expected', 'chosen', 'rationale', 'actual']"
                :key="key"
                ><dt>{{ key }}</dt>
                <dd>{{ draft[key] || 'Unknown / not supplied' }}</dd></template
              >
            </dl>
          </details>
        </template>
      </fieldset>
      <div class="evidence-actions">
        <button
          v-if="draft.step > 0"
          class="btn btn-secondary"
          type="button"
          :disabled="store.busy"
          @click="draft.step--"
        >
          Back
        </button>
        <button class="btn btn-secondary" type="button" :disabled="store.busy" @click="save(false)">
          Save and pause
        </button>
        <button
          class="btn btn-primary"
          :disabled="store.busy || (draft.step === 0 && !draft.situation.trim())"
        >
          {{ draft.step === 3 ? 'Finish this interview' : 'Save and continue' }}
        </button>
      </div>
    </form>
    <button v-if="completed" class="btn btn-secondary" :disabled="store.busy" @click="startAnother">
      Start another decision
    </button>
  </section>
</template>

<script setup>
import { computed, reactive, ref } from 'vue'
import { useEvidenceStore } from '@/stores/evidence'
const store = useEvidenceStore()
const steps = [
  'Situation and options',
  'Intent and expectations',
  'Choice and tradeoffs',
  'Outcome and review',
]
const draft = reactive({
  id: '',
  subject_id: store.snapshot.subject_id,
  subject_name: store.snapshot.subject_name,
  domain: 'product_project',
  step: 0,
  situation: '',
  options: [],
  wanted: '',
  expected: '',
  chosen: '',
  rejected: [],
  actual: '',
  rationale: '',
  constraints: [],
  goal_ids: [],
  updated_at: '',
  expected_goal_relation: null,
  ...JSON.parse(JSON.stringify(store.snapshot.interview_draft || {})),
})
draft.step = Math.min(3, Math.max(0, draft.step))
const options = ref(draft.options.join('\n'))
const rejected = ref(draft.rejected.join('\n'))
const constraints = ref(draft.constraints.join('\n'))
const completed = ref(false)
const currentGoals = computed(() =>
  Object.values(
    Object.fromEntries(store.snapshot.goals.filter((g) => !g.invalidated).map((g) => [g.id, g]))
  )
)
const lines = (value) =>
  value
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
async function save(submit) {
  const result = await store.saveInterview({
    draft: {
      ...draft,
      options: lines(options.value),
      rejected: lines(rejected.value),
      constraints: lines(constraints.value),
    },
    submit,
  })
  if (!store.error && store.snapshot.interview_draft)
    Object.assign(draft, store.snapshot.interview_draft)
  if (!store.error && submit) completed.value = true
  return !store.error && result !== null
}
async function advance() {
  if (draft.step === 3) return save(true)
  draft.step++
  if (!(await save(false))) draft.step--
}
function startAnother() {
  Object.assign(draft, {
    id: '',
    step: 0,
    situation: '',
    options: [],
    wanted: '',
    expected: '',
    chosen: '',
    rejected: [],
    actual: '',
    rationale: '',
    constraints: [],
    goal_ids: [],
    updated_at: '',
    expected_goal_relation: null,
  })
  options.value = ''
  rejected.value = ''
  constraints.value = ''
  completed.value = false
}
</script>
