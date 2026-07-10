<template>
  <div class="twin-workspace">
    <header class="workspace-header">
      <div class="header-links">
        <router-link to="/">
          Notes
        </router-link>
        <router-link to="/canvas">
          Canvas
        </router-link>
      </div>
      <div class="header-title">
        <h1>Twin Workspace</h1>
        <span>{{ twinStore.healthSummary }}</span>
      </div>
      <div class="header-actions">
        <button
          class="btn btn-secondary"
          :disabled="twinStore.runningTwinInference"
          @click="twinStore.runTwinInference"
        >
          {{ twinStore.runningTwinInference ? 'Running...' : 'Run Records' }}
        </button>
        <button
          class="btn btn-primary"
          :disabled="twinStore.runningConstitutionInference"
          @click="twinStore.runConstitutionInference"
        >
          {{ twinStore.runningConstitutionInference ? 'Running...' : 'Run Constitution' }}
        </button>
      </div>
    </header>

    <div class="workspace-body">
      <nav class="workspace-rail">
        <template
          v-for="group in navGroups"
          :key="group.id"
        >
          <span
            v-if="group.label"
            class="rail-label"
          >{{ group.label }}</span>
          <button
            v-for="tab in group.tabs"
            :key="tab.id"
            class="tab-button"
            :class="{ active: twinStore.activeTab === tab.id }"
            @click="twinStore.activeTab = tab.id"
          >
            <span>{{ tab.label }}</span>
            <strong v-if="tab.count !== null">{{ tab.count }}</strong>
          </button>
        </template>
      </nav>

      <main class="workspace-main">
        <section
          v-if="twinStore.activeTab === 'overview'"
          class="tab-panel"
        >
          <div class="overview-stats">
            <button
              class="metric-tile"
              @click="twinStore.activeTab = 'constitution'"
            >
              <span>Constitution</span>
              <strong>{{ twinStore.activeConstitutionCount }}</strong>
              <small>{{ twinStore.constitutionItems.length }} total items</small>
            </button>
            <button
              class="metric-tile"
              @click="twinStore.activeTab = 'action_gaps'"
            >
              <span>Action Gaps</span>
              <strong>{{ twinStore.activeActionGapCount }}</strong>
              <small>{{ twinStore.actionGaps.length }} total gaps</small>
            </button>
            <button
              class="metric-tile"
              @click="twinStore.activeTab = 'decisions'"
            >
              <span>Decisions</span>
              <strong>{{ twinStore.decisions.length }}</strong>
              <small>{{ twinStore.pendingOutcomeCount }} outcomes pending</small>
            </button>
            <button
              class="metric-tile"
              @click="twinStore.activeTab = 'memory'"
            >
              <span>Review</span>
              <strong>{{ twinStore.pendingReviewCount }}</strong>
              <small>digest plus candidate records</small>
            </button>
          </div>

          <section
            v-if="twinStore.showTutorialIntro"
            class="workspace-band tutorial-intro"
          >
            <div class="band-header">
              <h2>How To Use</h2>
              <div class="header-actions compact">
                <button
                  class="text-button"
                  @click="twinStore.activeTab = 'guide'"
                >
                  Open Guide
                </button>
                <button
                  class="text-button"
                  @click="twinStore.dismissTutorial"
                >
                  Dismiss
                </button>
              </div>
            </div>
            <p>
              Start decisions in Canvas, then use Twin Workspace for review, setup, outcomes,
              configuration, and benchmark export.
            </p>
          </section>

          <section class="workspace-band">
            <div class="band-header">
              <h2>Recent Decisions</h2>
              <button
                class="text-button"
                @click="twinStore.activeTab = 'decisions'"
              >
                View all
              </button>
            </div>
            <div
              v-if="twinStore.recentDecisions.length === 0"
              class="empty-panel"
            >
              No decision episodes yet. Start a Decision in Canvas and it will appear here.
            </div>
            <button
              v-for="item in twinStore.recentDecisions"
              :key="item.episode.id"
              class="compact-row"
              :class="{ highlighted: item.episode.id === routeDecisionId }"
              @click="twinStore.activeTab = 'decisions'"
            >
              <span
                class="row-dot"
                :class="decisionState(item)"
              />
              <span class="row-title">{{ item.episode.decision }}</span>
              <span class="row-chips">
                <span
                  v-for="chip in decisionChips(item)"
                  :key="chip.id"
                  class="chip"
                  :class="chip.cls"
                >{{ chip.label }}</span>
              </span>
            </button>
          </section>

          <section class="workspace-band">
            <div class="band-header">
              <h2>Current Action Gaps</h2>
              <button
                class="text-button"
                @click="twinStore.activeTab = 'action_gaps'"
              >
                Review
              </button>
            </div>
            <div
              v-if="twinStore.topActionGaps.length === 0"
              class="empty-panel"
            >
              No action gaps found.
            </div>
            <button
              v-for="gap in twinStore.topActionGaps"
              :key="gap.id"
              class="compact-row"
              @click="twinStore.activeTab = 'action_gaps'"
            >
              <span class="row-dot is-pending" />
              <span class="row-title">{{ gap.decision_risk }}</span>
              <span class="row-chips">
                <span class="chip">{{ formatPercent(gap.confidence) }}</span>
              </span>
            </button>
          </section>
        </section>

        <TwinConstitutionTab v-else-if="twinStore.activeTab === 'constitution'" />

        <section
          v-else-if="twinStore.activeTab === 'action_gaps'"
          class="tab-panel"
        >
          <div class="panel-header">
            <div>
              <h2>Action Gaps</h2>
              <span>{{ twinStore.actionGaps.length }} stated versus revealed patterns</span>
            </div>
          </div>
          <div
            v-if="twinStore.actionGaps.length === 0"
            class="empty-panel"
          >
            No action gaps yet.
          </div>
          <ActionGapRow
            v-for="gap in twinStore.actionGaps"
            :key="gap.id"
            :gap="gap"
            @review="twinStore.reviewActionGap"
          />
        </section>

        <section
          v-else-if="twinStore.activeTab === 'decisions'"
          class="tab-panel"
        >
          <div class="panel-header">
            <div>
              <h2>Decisions</h2>
              <span>{{ twinStore.decisions.length }} episodes — record what you actually chose to reveal each sealed twin prediction</span>
            </div>
          </div>
          <div
            v-if="twinStore.decisions.length === 0"
            class="empty-panel"
          >
            No decision episodes yet. Start a Decision in Canvas and it will appear here.
          </div>
          <DecisionRow
            v-for="item in twinStore.decisions"
            :key="item.episode.id"
            :item="item"
            :highlighted="item.episode.id === routeDecisionId"
            :trace-open="item.episode.id === routeDecisionId && routeTraceRequested"
            @update-outcome="twinStore.updateDecisionOutcome"
          />
        </section>

        <section
          v-else-if="twinStore.activeTab === 'memory'"
          class="tab-panel memory-grid"
        >
          <section class="workspace-band">
            <div class="panel-header compact">
              <div>
                <h2>Adaptive Digest</h2>
                <span>{{ twinStore.memoryDigestItems.length }} clustered items</span>
              </div>
            </div>
            <div
              v-if="twinStore.memoryDigestItems.length === 0"
              class="empty-panel"
            >
              No digest items need review.
            </div>
            <article
              v-for="item in twinStore.memoryDigestItems"
              :key="item.id"
              class="digest-card"
            >
              <div class="card-topline">
                <span class="status-pill">{{ statusLabel(item.state) }}</span>
                <span>{{ item.evidence_count }} evidence</span>
                <span>{{ item.trigger_reason }}</span>
              </div>
              <p>{{ item.pattern }}</p>
              <small v-if="item.latest_evidence?.summary">{{ item.latest_evidence.summary }}</small>
              <ReviewActions
                @review="action => twinStore.reviewMemoryDigestItem(item.id, action)"
              />
            </article>
          </section>

          <section class="workspace-band">
            <div class="panel-header compact">
              <div>
                <h2>User Records</h2>
                <span>{{ twinStore.filteredReviewRecords.length }} shown</span>
              </div>
              <select v-model="twinStore.selectedRecordState">
                <option
                  v-for="state in recordStates"
                  :key="state"
                  :value="state"
                >
                  {{ statusLabel(state) }}
                </option>
              </select>
            </div>
            <div
              v-if="twinStore.filteredReviewRecords.length === 0"
              class="empty-panel"
            >
              No records in this state.
            </div>
            <article
              v-for="item in twinStore.filteredReviewRecords"
              :key="item.record.id"
              class="record-card"
            >
              <div class="card-topline">
                <span class="status-pill">{{ kindLabel(item.record.kind) }}</span>
                <span>{{ statusLabel(item.record.promotion_state) }}</span>
                <span>{{ item.evidence_count }} evidence</span>
              </div>
              <p>{{ item.record.content }}</p>
              <div class="record-actions">
                <button @click="twinStore.openEvidence(item.record.id)">
                  Evidence
                </button>
                <button @click="twinStore.setPromotion(item.record.id, 'endorsed')">
                  Endorse
                </button>
                <button @click="twinStore.setPromotion(item.record.id, 'private')">
                  Private
                </button>
                <button @click="twinStore.setPromotion(item.record.id, 'no_train')">
                  No Train
                </button>
                <button @click="twinStore.setPromotion(item.record.id, 'rejected')">
                  Reject
                </button>
              </div>
            </article>
          </section>
        </section>

        <TwinSetupTab v-else-if="twinStore.activeTab === 'setup'" />

        <TwinGuideTab v-else-if="twinStore.activeTab === 'guide'" />

        <TwinConfigTab v-else-if="twinStore.activeTab === 'config'" />
      </main>
    </div>

    <aside
      v-if="twinStore.selectedRecordId"
      class="evidence-drawer"
    >
      <div class="drawer-header">
        <h2>Evidence</h2>
        <button @click="twinStore.selectedRecordId = null">
          x
        </button>
      </div>
      <div
        v-if="twinStore.evidenceLoading"
        class="empty-panel"
      >
        Loading evidence...
      </div>
      <div
        v-else-if="twinStore.selectedEvidence.length === 0"
        class="empty-panel"
      >
        No evidence events found.
      </div>
      <article
        v-for="item in twinStore.selectedEvidence"
        :key="item.event_id"
        class="evidence-item"
      >
        <div class="card-topline">
          <span>{{ eventLabel(item.event_type) }}</span>
          <span>{{ formatDate(item.created_at) }}</span>
        </div>
        <p v-if="item.prompt_excerpt">
          {{ item.prompt_excerpt }}
        </p>
        <p v-if="item.response_excerpt">
          {{ item.response_excerpt }}
        </p>
        <small>{{ item.session_id }} <span v-if="item.model_id">/ {{ item.model_id }}</span></small>
      </article>
    </aside>

    <div
      v-if="twinStore.message"
      class="save-toast"
      :class="twinStore.message.type"
    >
      {{ twinStore.message.text }}
    </div>
  </div>
</template>

<script setup>
import { computed, defineComponent, h, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { useTwinStore } from '@/stores/twin'
import {
  decisionState,
  decisionChips,
  statusLabel,
  kindLabel,
  formatPercent,
  formatWeight,
  presetLabel,
  sourceTypeLabel,
  scoreRows,
  feedbackEventLabel,
  feedbackEventNote,
  formatDate,
  eventLabel
} from '@/utils/twinFormat'
import ReviewActions from '@/components/twin/ReviewActions.vue'
import TwinConstitutionTab from '@/components/twin/TwinConstitutionTab.vue'
import TwinGuideTab from '@/components/twin/TwinGuideTab.vue'
import TwinSetupTab from '@/components/twin/TwinSetupTab.vue'
import TwinConfigTab from '@/components/twin/TwinConfigTab.vue'
import '@/components/twin/twin-workspace.css'

const twinStore = useTwinStore()

const navGroups = computed(() => [
  {
    id: 'home',
    label: null,
    tabs: [{ id: 'overview', label: 'Overview', count: null }]
  },
  {
    id: 'work',
    label: 'Work',
    tabs: [{ id: 'decisions', label: 'Decisions', count: twinStore.decisions.length }]
  },
  {
    id: 'review',
    label: 'Review',
    tabs: [
      { id: 'memory', label: 'Memory Review', count: twinStore.pendingReviewCount },
      { id: 'constitution', label: 'Constitution', count: twinStore.constitutionItems.length },
      { id: 'action_gaps', label: 'Action Gaps', count: twinStore.actionGaps.length }
    ]
  },
  {
    id: 'tune',
    label: 'Configure',
    tabs: [
      { id: 'setup', label: 'Setup', count: null },
      { id: 'config', label: 'Config', count: null }
    ]
  },
  {
    id: 'help',
    label: 'Help',
    tabs: [{ id: 'guide', label: 'Guide', count: null }]
  }
])

const ActionGapRow = defineComponent({
  props: {
    gap: { type: Object, required: true }
  },
  emits: ['review'],
  setup(props, { emit }) {
    return () => h('article', { class: 'action-gap-card' }, [
      h('div', { class: 'card-topline' }, [
        h('span', { class: 'status-pill' }, statusLabel(props.gap.status)),
        h('span', formatPercent(props.gap.confidence)),
        h('span', `${props.gap.evidence_refs?.length || 0} evidence`)
      ]),
      h('div', { class: 'gap-columns' }, [
        h('div', [h('small', 'Stated'), h('p', props.gap.stated_value)]),
        h('div', [h('small', 'Revealed'), h('p', props.gap.revealed_behavior)])
      ]),
      props.gap.driver_hypothesis ? h('p', { class: 'muted' }, props.gap.driver_hypothesis) : null,
      h('p', props.gap.decision_risk),
      h(ReviewActions, { onReview: action => emit('review', props.gap.id, action) })
    ])
  }
})

const DecisionRow = defineComponent({
  props: {
    item: { type: Object, required: true },
    highlighted: { type: Boolean, default: false },
    traceOpen: { type: Boolean, default: false }
  },
  emits: ['update-outcome'],
  setup(props, { emit }) {
    const episodeOptions = props.item.episode.options || []
    const savedChoice = props.item.episode.chosen_option || ''
    const choiceInList = savedChoice && episodeOptions.includes(savedChoice)
    const outcome = ref(props.item.episode.outcome || '')
    const lesson = ref(props.item.episode.lesson || '')
    const regretScore = ref(props.item.episode.regret_score ?? '')
    const chosenSelect = ref(choiceInList ? savedChoice : (savedChoice ? '__other__' : ''))
    const chosenOther = ref(choiceInList ? '' : savedChoice)
    const correctionNote = ref(props.item.episode.correction_note || '')
    const resolvedChoice = () =>
      chosenSelect.value === '__other__' ? chosenOther.value.trim() : chosenSelect.value
    const save = () => emit('update-outcome', props.item.episode.id, {
      outcome: outcome.value || null,
      lesson: lesson.value || null,
      regret_score: regretScore.value === '' ? null : Number(regretScore.value),
      chosen_option: resolvedChoice() || null,
      correction_note: correctionNote.value.trim() || null
    })
    return () => {
      const episode = props.item.episode
      const latestCard = props.item.reflection_cards?.[0]
      const trace = latestCard?.evidence_packet || {}
      const selectedSources = trace.selected_sources || []
      const excludedTotal = (trace.excluded_private_count || 0)
        + (trace.excluded_rejected_count || 0)
        + (trace.excluded_no_train_count || 0)
      const scores = latestCard?.scores || {}
      const feedbackEvents = props.item.feedback_events || []
      const outcomeField = (label, node) =>
        h('label', { class: 'outcome-field' }, [h('span', label), node])

      return h('article', {
        class: [
          'decision-card',
          decisionState(props.item),
          props.highlighted ? 'highlighted' : ''
        ]
      }, [
        h('div', { class: 'card-topline' }, [
          h('span', episode.review_date || 'No follow-up'),
          h('span', `${props.item.reflection_cards?.length || 0} cards`),
          episode.confidence != null ? h('span', `${Math.round(episode.confidence * 100)}% confidence`) : null,
          h('span', { class: 'topline-chips' }, decisionChips(props.item).map(chip =>
            h('span', { key: chip.id, class: ['chip', chip.cls] }, chip.label)
          ))
        ]),
        h('h3', episode.decision),
        episode.options?.length
          ? h('div', { class: 'tag-row' }, episode.options.map(option => h('span', option)))
          : null,
        h('div', { class: 'primitive-grid' }, primitiveFields.map(field =>
          episode.primitive_assessment?.[field.key]
            ? h('span', [h('strong', field.label), ` ${episode.primitive_assessment[field.key]}`])
            : null
        )),
        props.item.reflection_cards?.[0]
          ? h('details', { class: 'reflection-details' }, [
              h('summary', 'Latest Reflection Card'),
              h('pre', props.item.reflection_cards[0].content)
            ])
          : null,
        latestCard
          ? h('details', { class: 'context-trace-details', open: props.traceOpen }, [
              h('summary', 'Context Trace'),
              h('div', { class: 'trace-metrics' }, [
                h('span', [h('strong', 'Preset'), ` ${presetLabel(trace.config_snapshot?.preset)}`]),
                h('span', [h('strong', 'Sources'), ` ${selectedSources.length}`]),
                h('span', [h('strong', 'Excluded'), ` ${excludedTotal}`]),
                h('span', [h('strong', 'Unsupported'), ` ${scores.unsupported_claim_count || 0}`])
              ]),
              scores.evidence_grounding_score < 0.5
                ? h('p', { class: 'trace-warning' }, 'Weak evidence signal: treat self-model claims as tentative.')
                : null,
              h('section', { class: 'trace-section' }, [
                h('h4', 'Selected Context'),
                selectedSources.length
                  ? h('ul', { class: 'trace-source-list' }, selectedSources.map(source =>
                      h('li', { key: `${source.source_type}-${source.id}` }, [
                        h('span', sourceTypeLabel(source.source_type)),
                        h('strong', source.label || source.id),
                        h('small', `${source.reason || 'Selected for this decision'} · ${formatWeight(source.weight)}`)
                      ])
                    ))
                  : h('p', { class: 'muted' }, 'No selected context recorded for this card.')
              ]),
              h('section', { class: 'trace-section' }, [
                h('h4', 'Score Breakdown'),
                h('div', { class: 'trace-score-grid' }, scoreRows(scores).map(row =>
                  h('span', { key: row.label }, [h('strong', row.label), ` ${row.value}`])
                ))
              ]),
              h('section', { class: 'trace-section' }, [
                h('h4', 'Feedback Afterward'),
                feedbackEvents.length
                  ? h('ul', { class: 'trace-feedback-list' }, feedbackEvents.map(event =>
                      h('li', { key: event.id }, [
                        h('span', feedbackEventLabel(event)),
                        h('small', feedbackEventNote(event))
                      ])
                    ))
                  : h('p', { class: 'muted' }, 'No correction or one-click feedback recorded yet.')
              ])
            ])
          : null,
        props.item.prediction_sealed
          ? h('div', { class: 'prediction-sealed-badge' },
              'Twin sealed a prediction — record your choice to reveal it')
          : null,
        episode.twin_prediction
          ? h('div', { class: 'prediction-reveal' }, [
              h('div', { class: 'prediction-headline' }, [
                h('strong', 'Twin predicted: '),
                h('span', episode.twin_prediction.predicted_option),
                episode.twin_prediction.confidence != null
                  ? h('span', { class: 'prediction-confidence' },
                      ` (${Math.round(episode.twin_prediction.confidence * 100)}% confident)`)
                  : null,
                episode.agreement === true
                  ? h('span', { class: 'agreement-badge match' }, 'Matched my choice')
                  : null,
                episode.agreement === false
                  ? h('span', { class: 'agreement-badge miss' }, 'Missed')
                  : null
              ]),
              episode.twin_prediction.rationale
                ? h('p', { class: 'prediction-rationale' }, episode.twin_prediction.rationale)
                : null
            ])
          : null,
        h('details', { class: 'outcome-details', open: !episode.outcome }, [
          h('summary', episode.outcome ? 'Outcome recorded — edit' : 'Record outcome'),
          h('div', { class: 'outcome-row' }, [
            episodeOptions.length
              ? outcomeField('Chosen option', h('select', {
                  class: 'chosen-option-select',
                  value: chosenSelect.value,
                  onChange: event => { chosenSelect.value = event.target.value }
                }, [
                  h('option', { value: '' }, 'Chosen option…'),
                  ...episodeOptions.map(option => h('option', { value: option }, option)),
                  h('option', { value: '__other__' }, 'Other…')
                ]))
              : null,
            (!episodeOptions.length || chosenSelect.value === '__other__')
              ? outcomeField(episodeOptions.length ? 'Other choice' : 'Chosen option', h('input', {
                  value: chosenOther.value,
                  placeholder: 'Chosen option',
                  onInput: event => { chosenOther.value = event.target.value }
                }))
              : null,
            outcomeField('Outcome', h('input', {
              value: outcome.value,
              placeholder: 'What actually happened',
              onInput: event => { outcome.value = event.target.value }
            })),
            outcomeField('Lesson', h('input', {
              value: lesson.value,
              placeholder: 'What it taught you',
              onInput: event => { lesson.value = event.target.value }
            })),
            outcomeField('Regret (0-10)', h('input', {
              value: regretScore.value,
              type: 'number',
              min: '0',
              max: '10',
              placeholder: '0',
              onInput: event => { regretScore.value = event.target.value }
            })),
            h('button', { class: 'btn btn-primary btn-sm save-outcome', onClick: save }, 'Save Outcome')
          ]),
          episode.agreement === false
            ? h('div', { class: 'correction-row' }, [
                outcomeField('Why was the twin wrong?', h('input', {
                  value: correctionNote.value,
                  placeholder: 'Why was the twin wrong?',
                  onInput: event => { correctionNote.value = event.target.value }
                }))
              ])
            : null
        ])
      ])
    }
  }
})

const primitiveFields = [
  { key: 'stakes', label: 'Stakes' },
  { key: 'reversibility', label: 'Reversibility' },
  { key: 'time_horizon', label: 'Time' },
  { key: 'uncertainty', label: 'Uncertainty' },
  { key: 'agency', label: 'Agency' },
  { key: 'value_tension', label: 'Value Tension' },
  { key: 'constraint_pressure', label: 'Constraint' },
  { key: 'taste_aesthetic_pull', label: 'Taste' },
  { key: 'somatic_signal', label: 'Somatic' },
  { key: 'action_gap_risk', label: 'Gap Risk' }
]

const recordStates = ['candidate', 'auto_promoted', 'endorsed', 'private', 'no_train', 'rejected']
const route = useRoute() || { query: {} }
// Matches the original per-mount ref initialization: every time this view
// mounts, the active tab is (re)computed from the current route query.
twinStore.activeTab = route.query?.decision ? 'decisions' : 'overview'

const routeDecisionId = computed(() => String(route.query?.decision || ''))
const routeTraceRequested = computed(() => route.query?.trace === '1')

onMounted(twinStore.loadWorkspace)
</script>

