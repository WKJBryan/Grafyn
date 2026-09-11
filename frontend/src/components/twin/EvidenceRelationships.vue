<template>
  <section class="evidence-section">
    <h2>Evidence relationships</h2>
    <p>
      Nearer means more closely related within this view. It does not mean more certain, agreeable,
      or causally stronger. Node size shows visible connectivity.
    </p>
    <div class="evidence-filters">
      <label
        >View<select v-model="layer">
          <option value="meaning">Meaning</option>
          <option value="goal_path" :disabled="store.snapshot.subject_id === 'unconfigured'">Goal paths</option>
        </select></label
      >
      <label
        >Review<select v-model="review">
          <option value="">All usable evidence</option>
          <option value="tentative">Tentative</option>
          <option value="confirmed">Confirmed</option>
        </select></label
      >
      <label
        >Relation<select v-model="relation">
          <option value="">All relations</option>
          <option v-for="kind in relationKinds" :key="kind">{{ kind }}</option>
        </select></label
      >
      <label
        >Source<select v-model="source">
          <option value="">All sources</option>
          <option v-for="item in store.snapshot.sources" :key="item.id" :value="item.id">
            {{ item.title || item.id }}
          </option>
        </select></label
      >
      <label>Known by date<input v-model="asOf" type="date" /></label>
      <label
        >Focus<select v-model="focus">
          <option value="">All visible content</option>
          <option v-for="node in availableNodes" :key="node.id" :value="node.id">
            {{ node.label }}
          </option>
        </select></label
      >
    </div>
    <fieldset v-if="layer === 'goal_path'">
      <legend>Concurrent goals</legend>
      <label v-for="goal in goals" :key="goal.id" class="inline-label"
        ><input v-model="selectedGoals" type="checkbox" :value="goal.id" /> {{ goal.label }} ·
        {{ goal.status }} · r{{ goal.revision }}</label
      ><label
        >Path depth<select v-model.number="depth">
          <option :value="1">One step</option>
          <option :value="2">Two steps</option>
          <option :value="3">Three steps</option>
          <option :value="4">Four steps</option>
        </select></label
      >
      <p>
        Steps are relative to the selected node. Hop count does not describe elapsed time or effect
        size. Missing connecting evidence stays missing.
      </p>
    </fieldset>
    <p>
      {{ store.snapshot.subject_id === 'unconfigured' ? 'Document relationships; no target person mapped' : `Target: ${store.snapshot.subject_name || store.snapshot.subject_id}` }} · Dashed = tentative,
      solid = reviewed · arrows = direction · unscored = neutral length.
    </p>
    <p v-if="layer === 'goal_path'">
      Blue: action · purple: consequence · red: constraint · amber: goal. Labels identify each kind
      without relying on color.
    </p>
    <p>Embedding discovery: {{ store.snapshot.embedding_status || 'Unknown' }}</p>
    <p>Contextual assessment: {{ store.snapshot.assessment_status || 'Unknown' }}</p>
    <details v-if="groups.length" open>
      <summary>Passage groups ({{ groups.length }})</summary>
      <p>
        Each group contains an anchor and its strongest scored semantic neighbors. Passages may
        appear in several groups. A group adds no evidence and does not make all its members
        equivalent.
      </p>
      <ul class="evidence-list">
        <li v-for="group in groups" :key="group.id">
          <button
            class="btn btn-secondary"
            :aria-expanded="expandedGroupId === group.id"
            @click="toggleGroup(group.id)"
          >
            Related to {{ nodeLabel(group.anchor) }} · {{ group.members.length }} members
          </button>
          <section
            v-if="expandedGroupId === group.id"
            :aria-label="`Members related to ${nodeLabel(group.anchor)}`"
          >
            <ul>
              <li v-for="id in group.members" :key="id">
                {{ nodeLabel(id) }}
                <small v-if="membershipCount(id) > 1"
                  >· appears in {{ membershipCount(id) }} groups</small
                >
              </li>
            </ul>
            <p v-if="group.hiddenNeighbors">
              {{ group.hiddenNeighbors }} additional neighbors are outside this strongest-neighbor
              group. Collapse the group and focus its anchor to expand weaker connections.
            </p>
          </section>
        </li>
      </ul>
    </details>
    <details v-if="statements.length">
      <summary>Personal statements and procedures ({{ statements.length }})</summary>
      <p>
        These are source-backed statements, preferences, or procedures. They are not recorded
        choices.
      </p>
      <article v-for="statement in statements" :key="statement.id" class="criterion">
        <strong>{{ statement.kind.replaceAll('_', ' ') }} · {{ statement.review_status }}</strong>
        <p>{{ statement.statement }}</p>
        <div class="evidence-actions">
          <button
            class="btn btn-primary"
            :disabled="store.busy"
            @click="store.reviewStatement({ id: statement.id, status: 'confirmed' })"
          >
            Confirm statement</button
          ><button
            class="btn btn-secondary"
            :disabled="store.busy"
            @click="store.reviewStatement({ id: statement.id, status: 'rejected' })"
          >
            Reject statement
          </button>
        </div>
        <p>
          To correct this interpretation, edit its source and re-import. Original quotes and
          revisions are retained.
        </p>
        <figure v-for="(receipt, index) in statement.receipts" :key="index">
          <figcaption>
            {{ receipt.locator || receipt.source_id }} · source revision
            {{ receipt.source_revision }}
          </figcaption>
          <blockquote>{{ receipt.quote }}</blockquote>
        </figure>
      </article>
    </details>
    <p v-if="focused.hidden">
      {{ focused.hidden }} weaker or unscored connections hidden.
      <button class="btn btn-secondary" @click="expanded = true">Expand weaker connections</button>
    </p>
    <div class="evidence-graph">
      <GraphView
        :evidence-data="graphData"
        :show-settings="true"
        @node-click="focusNode"
        @edge-click="selectedId = $event.id"
      />
    </div>
    <p v-if="!focused.visible.length">
      No grounded relationships match these filters. Save an interview and process its evidence, or
      widen the filters.
    </p>
    <details open>
      <summary>Accessible relationship list ({{ focused.visible.length }})</summary>
      <ul class="evidence-list">
        <li v-for="edge in focused.visible" :key="edge.id">
          <button class="btn btn-secondary" @click="selectedId = edge.id">
            {{ nodeLabel(edge.source) }} — {{ edge.relation.replaceAll('_', ' ') }}
            {{ edge.directed ? '→' : '—' }} {{ nodeLabel(edge.target) }} ·
            {{ edge.review_status }} · {{ edge.score == null ? 'unscored' : 'scored' }}
          </button>
        </li>
      </ul>
    </details>
    <details v-if="withheld.length">
      <summary>Withheld candidates ({{ withheld.length }})</summary>
      <p>
        These candidate pairs are available for review but are not established graph relationships.
        Machine withholding records no human rejection.
      </p>
      <ul class="evidence-list">
        <li v-for="edge in withheld" :key="edge.id">
          <button class="btn btn-secondary" @click="selectedId = edge.id">
            {{ nodeLabel(edge.from_id) }} — {{ candidateStateLabel(edge) }} —
            {{ nodeLabel(edge.to_id) }} ·
            {{ candidateExplanation(edge) }}
          </button>
        </li>
      </ul>
    </details>
    <RelationshipEvidencePanel v-if="selected" :relationship="selected" @close="selectedId = ''" />
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import GraphView from '@/components/GraphView.vue'
import {
  focusRelationships,
  semanticNeighborhoodGroups,
  applicableGoalRevisions,
  isEstablishedRelationship,
  relationshipAssessmentState,
} from '@/utils/evidenceGraph'
import RelationshipEvidencePanel from './RelationshipEvidencePanel.vue'
import { useEvidenceStore } from '@/stores/evidence'
const store = useEvidenceStore()
const layer = ref('meaning'),
  review = ref(''),
  relation = ref(''),
  source = ref(''),
  asOf = ref(''),
  focus = ref(''),
  expanded = ref(false),
  selectedId = ref(''),
  selectedGoals = ref([]),
  expandedGroupId = ref(''),
  depth = ref(2)
const effects = ['enables', 'inhibits', 'requires', 'contributes_to']
const cutoff = computed(() => (asOf.value ? `${asOf.value}T23:59:59.999Z` : null))
const knownByCutoff = (value) => !!value && Date.parse(value) <= Date.parse(cutoff.value)
const statements = computed(() =>
  (store.snapshot.statements || []).filter(
    (item) =>
      !item.invalidated &&
      item.review_status !== 'rejected' &&
      item.subject_id === store.snapshot.subject_id &&
      (!review.value || item.review_status === review.value) &&
      (!cutoff.value || knownByCutoff(item.recorded_at)) &&
      (!source.value || item.receipts.some((receipt) => receipt.source_id === source.value))
  )
)
const goals = computed(() =>
  applicableGoalRevisions(
    store.snapshot.goals.filter((goal) => goal.subject_id === store.snapshot.subject_id),
    cutoff.value ? Date.parse(cutoff.value) : Date.now()
  )
)
const usable = computed(() =>
  store.snapshot.relationships.filter((edge) => {
    if (
      edge.invalidated ||
      edge.review_status === 'rejected' ||
      edge.subject_id !== store.snapshot.subject_id
    )
      return false
    if (!isEstablishedRelationship(edge)) return false
    if ((layer.value === 'goal_path') !== effects.includes(edge.relation)) return false
    if (
      [edge.from_id, edge.to_id].some(
        (id) =>
          store.snapshot.goals.some((goal) => goal.id === id) &&
          !goals.value.some((goal) => goal.id === id)
      )
    )
      return false
    if (review.value && edge.review_status !== review.value) return false
    if (cutoff.value && !knownByCutoff(edge.recorded_at)) return false
    if (
      source.value &&
      ![edge.from_receipt.source_id, edge.to_receipt.source_id].includes(source.value)
    )
      return false
    if (
      cutoff.value &&
      [edge.from_receipt, edge.to_receipt].some((receipt) => {
        const item = store.snapshot.sources.find((s) => s.id === receipt.source_id)
        return !item || !knownByCutoff(item.recorded_at)
      })
    )
      return false
    return true
  })
)
const relationKinds = computed(() => [...new Set(usable.value.map((edge) => edge.relation))])
const withheld = computed(() =>
  store.snapshot.relationships.filter((edge) => {
    if (
      edge.invalidated ||
      edge.review_status === 'rejected' ||
      edge.subject_id !== store.snapshot.subject_id ||
      isEstablishedRelationship(edge)
    )
      return false
    if ((layer.value === 'goal_path') !== effects.includes(edge.relation)) return false
    if (review.value && edge.review_status !== review.value) return false
    if (relation.value && edge.relation !== relation.value) return false
    if (cutoff.value && !knownByCutoff(edge.recorded_at)) return false
    if (
      source.value &&
      ![edge.from_receipt.source_id, edge.to_receipt.source_id].includes(source.value)
    )
      return false
    if (
      cutoff.value &&
      [edge.from_receipt, edge.to_receipt].some((receipt) => {
        const item = store.snapshot.sources.find((s) => s.id === receipt.source_id)
        return !item || !knownByCutoff(item.recorded_at)
      })
    )
      return false
    return true
  })
)
const candidateStateLabel = (edge) => {
  const state = relationshipAssessmentState(edge)
  if (state === 'withheld') return edge.assessment.result.verdict.replaceAll('_', ' ')
  if (state === 'discovery_pending') return 'discovery pending'
  return state
}
const candidateExplanation = (edge) => {
  if (edge.review_stale) return 'Context changed; review this relationship again'
  if (relationshipAssessmentState(edge) === 'discovery_pending')
    return 'No current similarity score; candidate discovery pending'
  if (relationshipAssessmentState(edge) === 'stale')
    return 'Context changed; assessment pending'
  return (
    edge.assessment?.result?.explanation ||
    edge.assessment?.error ||
    'Awaiting contextual assessment'
  )
}
const edges = computed(() => {
  let candidates = usable.value
    .filter((edge) => !relation.value || edge.relation === relation.value)
    .map((edge) => ({
      ...edge,
      source: edge.from_id,
      target: edge.to_id,
      score: layer.value === 'goal_path' && effects.includes(edge.relation) ? null : edge.similarity,
    }))
  if (layer.value === 'goal_path' && selectedGoals.value.length) {
    const reached = new Set(selectedGoals.value),
      included = new Set()
    for (let step = 0; step < depth.value; step++) {
      const next = new Set(reached)
      for (const edge of candidates)
        if (reached.has(edge.source) || reached.has(edge.target)) {
          included.add(edge.id)
          next.add(edge.source)
          next.add(edge.target)
        }
      next.forEach((id) => reached.add(id))
    }
    candidates = candidates.filter((edge) => included.has(edge.id))
  }
  return candidates
})
const availableNodes = computed(() =>
  [...new Set(edges.value.flatMap((edge) => [edge.source, edge.target]))].map((id) => ({
    id,
    label: nodeLabel(id),
  }))
)
const groups = computed(() =>
  layer.value === 'meaning' ? semanticNeighborhoodGroups(edges.value) : []
)
const selectedGroup = computed(() =>
  groups.value.find((group) => group.id === expandedGroupId.value)
)
const focused = computed(() => {
  const group = selectedGroup.value
  const candidates = group
    ? edges.value.filter(
        (edge) =>
          typeof edge.score === 'number' &&
          Number.isFinite(edge.score) &&
          group.members.includes(edge.source) &&
          group.members.includes(edge.target)
      )
    : edges.value
  return focusRelationships(candidates, focus.value, expanded.value)
})
const membershipCount = (id) => groups.value.filter((group) => group.members.includes(id)).length
function toggleGroup(id) {
  expandedGroupId.value = expandedGroupId.value === id ? '' : id
  focus.value = ''
  expanded.value = false
}
function focusNode(id) {
  focus.value = id
  expanded.value = false
}
function nodeLabel(id) {
  const goal = goals.value.find((item) => item.id === id)
  if (goal) return `Goal: ${goal.label}`
  const statement = statements.value.find((item) => item.id === id)
  if (statement) return `${statement.kind.replaceAll('_', ' ')}: ${statement.statement}`
  const evidenceNode = store.snapshot.nodes?.find((item) => item.id === id && !item.invalidated)
  if (evidenceNode) return `${evidenceNode.kind}: ${evidenceNode.label}`
  const caseRecord = store.snapshot.cases.find((item) => item.id === id)
  if (caseRecord) return `Action: ${caseRecord.chosen || caseRecord.situation}`
  const item = store.snapshot.sources.find((item) => item.id === id)
  if (item) return item.title || item.id
  const edge = store.snapshot.relationships.find((edge) => edge.from_id === id || edge.to_id === id)
  const receipt = edge?.from_id === id ? edge.from_receipt : edge?.to_receipt
  return receipt?.quote?.slice(0, 90) || id
}
const graphData = computed(() => {
  const links = focused.value.visible
  const isolated = selectedGroup.value
    ? selectedGroup.value.members
    : focus.value || relation.value
      ? []
      : layer.value === 'meaning'
        ? statements.value.map((item) => item.id)
        : goals.value
            .filter((goal) => !selectedGoals.value.length || selectedGoals.value.includes(goal.id))
            .map((goal) => goal.id)
  const nodes = [
    ...new Set([...links.flatMap((edge) => [edge.source, edge.target]), ...isolated]),
  ].map((id) => ({
    id,
    label: nodeLabel(id),
    val: links.filter((edge) => edge.source === id || edge.target === id).length,
    group: goals.value.some((goal) => goal.id === id)
      ? '#f59e0b'
      : { consequence: '#c084fc', constraint: '#f87171' }[
          store.snapshot.nodes?.find((item) => item.id === id)?.kind
        ] || '#38bdf8',
  }))
  return { nodes, links }
})
const selected = computed(() =>
  store.snapshot.relationships.find((edge) => edge.id === selectedId.value)
)
watch([layer, review, relation, source, asOf], () => {
  focus.value = ''
  expanded.value = false
  selectedId.value = ''
  expandedGroupId.value = ''
})
</script>
