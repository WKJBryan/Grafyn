<template>
  <aside class="perspective-timeline" aria-label="Perspective history">
    <header>
      <strong>Perspective through time</strong>
      <button type="button" aria-label="Close perspective history" @click="$emit('close')">
        ×
      </button>
    </header>
    <p class="hint">
      Recorded perspectives over the current note map. Earlier note layouts are not reconstructed.
    </p>
    <p v-if="error" role="alert" class="error">{{ error }}</p>

    <label v-if="perspectives.length" class="field">
      Perspective
      <select v-model="selectedPerspectiveId" @change="selectAtTime">
        <option v-for="p in perspectives" :key="p.id" :value="p.id">{{ p.title }}</option>
      </select>
    </label>
    <label v-if="dates.length" class="field">
      As of {{ formatDate(dates[timeIndex]) }}
      <input
        v-model.number="timeIndex"
        aria-label="Time slice"
        type="range"
        min="0"
        :max="dates.length - 1"
        step="1"
        @input="selectAtTime"
      />
    </label>

    <div v-if="lineage.length" class="lineage" aria-label="Backward perspective lineage">
      <button
        v-for="state in lineage"
        :key="state.id"
        type="button"
        class="state"
        :class="{ active: state.id === activeState?.id }"
        @click="selectState(state)"
      >
        <time :datetime="state.effective_at">{{ formatDate(state.effective_at) }}</time>
        <span class="kind">{{ state.change_kind }}</span>
        <strong>{{ state.statement }}</strong>
        <small v-if="state.reason">{{ state.reason }}</small>
        <small
          >{{ state.source_note_ids.length }} source note{{
            state.source_note_ids.length === 1 ? '' : 's'
          }}</small
        >
      </button>
    </div>
    <p v-else class="hint">No recorded state at this time. Record one below.</p>

    <form @submit.prevent="recordState">
      <h3>Record a perspective</h3>
      <label class="field"
        >Source note
        <select v-model="form.sourceNoteId" required>
          <option value="" disabled>Select a note</option>
          <option v-for="note in nodes" :key="note.id" :value="note.id">{{ note.label }}</option>
        </select>
      </label>
      <label class="field"
        >Title <input v-model.trim="form.title" required maxlength="120"
      /></label>
      <label class="field"
        >What did you think?
        <textarea v-model.trim="form.statement" required rows="2" maxlength="2000" />
      </label>
      <label class="field"
        >When did you hold this view? <input v-model="form.effectiveAt" type="date" required
      /></label>
      <label v-if="activeState" class="field"
        >Change
        <select v-model="form.changeKind">
          <option value="revised">Revised</option>
          <option value="qualified">Qualified</option>
          <option value="reversed">Reversed</option>
        </select>
      </label>
      <label class="field"
        >Why? <textarea v-model.trim="form.reason" rows="2" maxlength="1000" />
      </label>
      <button class="save" type="submit" :disabled="saving">
        {{ saving ? 'Recording...' : activeState ? 'Add next state' : 'Start perspective' }}
      </button>
      <button v-if="activeState" class="new" type="button" @click="startNew">
        Start a different perspective
      </button>
    </form>
  </aside>
</template>

<script setup>
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { graph } from '@/api/client'

const props = defineProps({
  nodes: { type: Array, default: () => [] },
  sourceNoteId: { type: String, default: '' },
})
const emit = defineEmits(['close', 'select-state'])
const states = ref([])
const selectedPerspectiveId = ref('')
const selectedStateId = ref('')
const timeIndex = ref(0)
const saving = ref(false)
const error = ref('')
const form = reactive({
  sourceNoteId: props.sourceNoteId || '',
  title: '',
  statement: '',
  effectiveAt: new Date().toISOString().slice(0, 10),
  changeKind: 'revised',
  reason: '',
})
watch(
  () => props.sourceNoteId,
  (id) => {
    if (id) form.sourceNoteId = id
  }
)

const perspectives = computed(() => {
  const found = new Map()
  for (const state of states.value) {
    if (!found.has(state.perspective_id))
      found.set(state.perspective_id, { id: state.perspective_id, title: state.title })
  }
  return [...found.values()]
})
const dates = computed(() =>
  [...new Set(states.value.map((s) => s.effective_at.slice(0, 10)))].sort()
)
const eligible = computed(() =>
  states.value.filter(
    (s) =>
      s.perspective_id === selectedPerspectiveId.value &&
      s.effective_at.slice(0, 10) <= dates.value[timeIndex.value]
  )
)
const activeState = computed(
  () => eligible.value.find((s) => s.id === selectedStateId.value) || null
)
const lineage = computed(() => {
  const byId = new Map(eligible.value.map((s) => [s.id, s]))
  const result = []
  let state = activeState.value
  while (state) {
    result.push(state)
    state = byId.get(state.previous_state_id)
  }
  return result
})

function formatDate(value) {
  return value?.slice(0, 10) || ''
}

function selectAtTime() {
  const latest = eligible.value.at(-1)
  selectedStateId.value = latest?.id || ''
  emit('select-state', latest || null)
  if (latest) form.title = latest.title
}

function selectState(state) {
  selectedStateId.value = state.id
  emit('select-state', state)
}

function startNew() {
  selectedPerspectiveId.value = ''
  selectedStateId.value = ''
  form.title = ''
  form.statement = ''
  form.reason = ''
  emit('select-state', null)
}

async function loadStates() {
  try {
    states.value = await graph.perspectiveStates()
    if (!selectedPerspectiveId.value) selectedPerspectiveId.value = perspectives.value[0]?.id || ''
    timeIndex.value = Math.max(0, dates.value.length - 1)
    selectAtTime()
  } catch (cause) {
    error.value = cause?.message || String(cause)
  }
}

async function recordState() {
  error.value = ''
  saving.value = true
  try {
    const previous = activeState.value
    const recorded = await graph.recordPerspectiveState({
      perspective_id: previous?.perspective_id || null,
      previous_state_id: previous?.id || null,
      title: form.title,
      statement: form.statement,
      change_kind: previous ? form.changeKind : 'initial',
      reason: form.reason,
      source_note_ids: [form.sourceNoteId],
      effective_at: new Date(`${form.effectiveAt}T00:00:00Z`).toISOString(),
    })
    states.value = [...states.value, recorded].sort((a, b) =>
      a.effective_at.localeCompare(b.effective_at)
    )
    selectedPerspectiveId.value = recorded.perspective_id
    timeIndex.value = dates.value.indexOf(recorded.effective_at.slice(0, 10))
    selectState(recorded)
    form.statement = ''
    form.reason = ''
  } catch (cause) {
    error.value = cause?.message || String(cause)
  } finally {
    saving.value = false
  }
}

onMounted(loadStates)
</script>

<style scoped>
.perspective-timeline {
  position: absolute;
  top: 52px;
  left: 12px;
  bottom: 12px;
  width: min(330px, calc(100% - 24px));
  overflow: auto;
  z-index: 20;
  padding: 14px;
  color: var(--text-primary);
  background: var(--bg-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 32px #0005;
}
header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
header button,
.new {
  background: none;
  border: 0;
  color: var(--text-secondary);
  cursor: pointer;
}
.hint {
  font-size: 0.75rem;
  color: var(--text-secondary);
}
.error {
  color: var(--accent-red);
  font-size: 0.8rem;
}
.field {
  display: grid;
  gap: 4px;
  margin: 10px 0;
  font-size: 0.8rem;
}
input,
textarea,
select {
  width: 100%;
  box-sizing: border-box;
  padding: 6px;
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: 4px;
}
input[type='range'] {
  padding: 0;
}
.lineage {
  margin: 12px 0;
  padding-left: 15px;
  border-left: 2px solid var(--accent-primary);
  max-height: 230px;
  overflow: auto;
}
.state {
  position: relative;
  display: grid;
  width: 100%;
  padding: 7px;
  margin: 4px 0;
  gap: 3px;
  text-align: left;
  color: var(--text-primary);
  background: transparent;
  border: 1px solid transparent;
  cursor: pointer;
}
.state::before {
  content: '';
  position: absolute;
  left: -21px;
  top: 12px;
  width: 8px;
  height: 8px;
  background: var(--accent-primary);
  border-radius: 50%;
}
.state.active {
  background: var(--bg-secondary);
  border-color: var(--accent-primary);
  border-radius: 4px;
}
.state time,
.state small,
.kind {
  font-size: 0.7rem;
  color: var(--text-secondary);
}
.state strong {
  font-size: 0.8rem;
}
h3 {
  font-size: 0.85rem;
  margin: 12px 0;
}
.save {
  width: 100%;
  padding: 8px;
  background: var(--accent-primary);
  color: white;
  border: 0;
  border-radius: 4px;
  cursor: pointer;
}
.new {
  width: 100%;
  margin-top: 8px;
}
</style>
