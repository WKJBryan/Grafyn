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
        <span>{{ healthSummary }}</span>
      </div>
      <div class="header-actions">
        <button
          class="btn btn-secondary"
          :disabled="runningTwinInference"
          @click="runTwinInference"
        >
          {{ runningTwinInference ? 'Running...' : 'Run Records' }}
        </button>
        <button
          class="btn btn-primary"
          :disabled="runningConstitutionInference"
          @click="runConstitutionInference"
        >
          {{ runningConstitutionInference ? 'Running...' : 'Run Constitution' }}
        </button>
      </div>
    </header>

    <nav class="workspace-tabs">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="tab-button"
        :class="{ active: activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        <span>{{ tab.label }}</span>
        <strong v-if="tab.count !== null">{{ tab.count }}</strong>
      </button>
    </nav>

    <main class="workspace-main">
      <section
        v-if="activeTab === 'overview'"
        class="tab-panel overview-grid"
      >
        <article class="metric-tile">
          <span>Constitution</span>
          <strong>{{ activeConstitutionCount }}</strong>
          <small>{{ constitutionItems.length }} total items</small>
        </article>
        <article class="metric-tile">
          <span>Action Gaps</span>
          <strong>{{ activeActionGapCount }}</strong>
          <small>{{ actionGaps.length }} total gaps</small>
        </article>
        <article class="metric-tile">
          <span>Decisions</span>
          <strong>{{ decisions.length }}</strong>
          <small>{{ pendingFollowUps }} follow-ups pending</small>
        </article>
        <article class="metric-tile">
          <span>Review</span>
          <strong>{{ pendingReviewCount }}</strong>
          <small>digest plus candidate records</small>
        </article>

        <section
          v-if="showTutorialIntro"
          class="workspace-band tutorial-intro"
        >
          <div class="band-header">
            <h2>How To Use</h2>
            <div class="header-actions compact">
              <button
                class="text-button"
                @click="activeTab = 'guide'"
              >
                Open Guide
              </button>
              <button
                class="text-button"
                @click="dismissTutorial"
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
              @click="activeTab = 'decisions'"
            >
              View all
            </button>
          </div>
          <div
            v-if="recentDecisions.length === 0"
            class="empty-panel"
          >
            No decision episodes yet.
          </div>
          <DecisionRow
            v-for="item in recentDecisions"
            :key="item.episode.id"
            :item="item"
            :highlighted="item.episode.id === routeDecisionId"
            @update-outcome="updateDecisionOutcome"
          />
        </section>

        <section class="workspace-band">
          <div class="band-header">
            <h2>Current Action Gaps</h2>
            <button
              class="text-button"
              @click="activeTab = 'action_gaps'"
            >
              Review
            </button>
          </div>
          <div
            v-if="topActionGaps.length === 0"
            class="empty-panel"
          >
            No action gaps found.
          </div>
          <ActionGapRow
            v-for="gap in topActionGaps"
            :key="gap.id"
            :gap="gap"
            @review="reviewActionGap"
          />
        </section>
      </section>

      <section
        v-else-if="activeTab === 'constitution'"
        class="tab-panel"
      >
        <div class="panel-header">
          <div>
            <h2>Constitution</h2>
            <span>{{ groupedConstitution.length }} dimensions</span>
          </div>
          <button
            class="btn btn-secondary btn-sm"
            @click="activeTab = 'setup'"
          >
            Setup
          </button>
        </div>

        <div
          v-if="constitutionItems.length === 0"
          class="empty-panel"
        >
          No constitution items yet.
        </div>
        <section
          v-for="group in groupedConstitution"
          :key="group.dimension"
          class="dimension-section"
        >
          <h3>{{ dimensionLabel(group.dimension) }}</h3>
          <article
            v-for="item in group.items"
            :key="item.id"
            class="constitution-card"
          >
            <div class="card-topline">
              <span class="status-pill">{{ statusLabel(item.status) }}</span>
              <span>{{ formatPercent(item.confidence) }} confidence</span>
              <span>{{ item.evidence_refs?.length || 0 }} evidence</span>
              <span>{{ constitutionSourceLabel(item) }}</span>
            </div>
            <p>{{ item.claim }}</p>
            <div
              v-if="constitutionEvidenceLabels(item).length"
              class="source-row"
            >
              <span
                v-for="label in constitutionEvidenceLabels(item)"
                :key="`${item.id}-${label}`"
              >{{ label }}</span>
            </div>
            <div
              v-if="item.scope?.length"
              class="tag-row"
            >
              <span
                v-for="scope in item.scope"
                :key="scope"
              >{{ scope }}</span>
            </div>
            <ReviewActions
              @review="action => reviewConstitutionItem(item.id, action)"
            />
          </article>
        </section>
      </section>

      <section
        v-else-if="activeTab === 'action_gaps'"
        class="tab-panel"
      >
        <div class="panel-header">
          <div>
            <h2>Action Gaps</h2>
            <span>{{ actionGaps.length }} stated versus revealed patterns</span>
          </div>
        </div>
        <div
          v-if="actionGaps.length === 0"
          class="empty-panel"
        >
          No action gaps yet.
        </div>
        <ActionGapRow
          v-for="gap in actionGaps"
          :key="gap.id"
          :gap="gap"
          @review="reviewActionGap"
        />
      </section>

      <section
        v-else-if="activeTab === 'decisions'"
        class="tab-panel"
      >
        <div class="panel-header">
          <div>
            <h2>Decisions</h2>
            <span>{{ decisions.length }} episodes</span>
          </div>
        </div>
        <div
          v-if="decisions.length === 0"
          class="empty-panel"
        >
          No decision episodes yet.
        </div>
        <DecisionRow
          v-for="item in decisions"
          :key="item.episode.id"
          :item="item"
          :highlighted="item.episode.id === routeDecisionId"
          :trace-open="item.episode.id === routeDecisionId && routeTraceRequested"
          @update-outcome="updateDecisionOutcome"
        />
      </section>

      <section
        v-else-if="activeTab === 'memory'"
        class="tab-panel memory-grid"
      >
        <section class="workspace-band">
          <div class="panel-header compact">
            <div>
              <h2>Adaptive Digest</h2>
              <span>{{ memoryDigestItems.length }} clustered items</span>
            </div>
          </div>
          <div
            v-if="memoryDigestItems.length === 0"
            class="empty-panel"
          >
            No digest items need review.
          </div>
          <article
            v-for="item in memoryDigestItems"
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
              @review="action => reviewMemoryDigestItem(item.id, action)"
            />
          </article>
        </section>

        <section class="workspace-band">
          <div class="panel-header compact">
            <div>
              <h2>User Records</h2>
              <span>{{ filteredReviewRecords.length }} shown</span>
            </div>
            <select v-model="selectedRecordState">
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
            v-if="filteredReviewRecords.length === 0"
            class="empty-panel"
          >
            No records in this state.
          </div>
          <article
            v-for="item in filteredReviewRecords"
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
              <button @click="openEvidence(item.record.id)">
                Evidence
              </button>
              <button @click="setPromotion(item.record.id, 'endorsed')">
                Endorse
              </button>
              <button @click="setPromotion(item.record.id, 'private')">
                Private
              </button>
              <button @click="setPromotion(item.record.id, 'no_train')">
                No Train
              </button>
              <button @click="setPromotion(item.record.id, 'rejected')">
                Reject
              </button>
            </div>
          </article>
        </section>
      </section>

      <section
        v-else-if="activeTab === 'setup'"
        class="tab-panel setup-grid"
      >
        <section class="identity-section">
          <div>
            <h2>Twin Identity</h2>
            <span>Name and role are required before Twin Simulation can run.</span>
          </div>
          <label class="identity-field">
            <span>Name</span>
            <input
              v-model="setupDraft.twin_name"
              aria-label="Twin name"
              type="text"
              placeholder="Who is this twin?"
            >
          </label>
          <label class="identity-field">
            <span>Role / context</span>
            <input
              v-model="setupDraft.twin_role"
              aria-label="Twin role"
              type="text"
              placeholder="What role should this twin reason from?"
            >
          </label>
          <SetupField
            v-model="setupDraft.source_boundaries"
            title="Source Boundaries"
          />
        </section>
        <div class="setup-section-heading">
          <h2>Operating Priors</h2>
          <span>Values, taste, constraints, somatic cues, and action tendencies.</span>
        </div>
        <SetupField
          v-model="setupDraft.values"
          title="Values"
        />
        <SetupField
          v-model="setupDraft.tastes"
          title="Taste"
        />
        <SetupField
          v-model="setupDraft.constraints"
          title="Constraints"
        />
        <SetupField
          v-model="setupDraft.somatic_cues"
          title="Somatic Cues"
        />
        <SetupField
          v-model="setupDraft.action_tendencies"
          title="Action Tendencies"
        />
        <div class="setup-actions">
          <button
            class="btn btn-primary"
            :disabled="savingSetup"
            @click="saveSetup"
          >
            {{ savingSetup ? 'Saving...' : 'Save Setup' }}
          </button>
        </div>
      </section>

      <section
        v-else-if="activeTab === 'guide'"
        class="tab-panel guide-panel"
      >
        <div class="panel-header">
          <div>
            <h2>How To Use</h2>
            <span>Button guide and short task walkthroughs</span>
          </div>
        </div>
        <section class="workspace-band guide-walkthrough">
          <h3>Fast Decision Session</h3>
          <ol>
            <li>Open Canvas and click <strong>+ New</strong>.</li>
            <li>Choose <strong>Decision</strong>, then write or paste the decision.</li>
            <li>Add options, stakes, leaning, and follow-up only when useful.</li>
            <li>Click <strong>Create Reflection Card</strong>.</li>
            <li>Use one feedback button if the card is useful or wrong.</li>
          </ol>
        </section>
        <section
          v-for="group in tutorialButtonGroups"
          :key="group.title"
          class="workspace-band guide-group"
        >
          <h3>{{ group.title }}</h3>
          <dl>
            <template
              v-for="item in group.items"
              :key="item.name"
            >
              <dt>{{ item.name }}</dt>
              <dd>{{ item.description }}</dd>
            </template>
          </dl>
        </section>
      </section>

      <section
        v-else-if="activeTab === 'config'"
        class="tab-panel config-panel"
      >
        <div class="panel-header">
          <div>
            <h2>Decision Mirror Config</h2>
            <span>Presets change retrieval and scoring weights without hiding raw sub-scores.</span>
          </div>
          <button
            class="btn btn-secondary btn-sm"
            :disabled="exportingBenchmark"
            @click="exportDecisionBenchmark"
          >
            {{ exportingBenchmark ? 'Exporting...' : 'Export Benchmark' }}
          </button>
        </div>

        <section class="workspace-band config-card">
          <label class="config-row">
            <span>Preset</span>
            <select v-model="configDraft.preset">
              <option
                v-for="preset in decisionMirrorPresets"
                :key="preset.value"
                :value="preset.value"
              >
                {{ preset.label }}
              </option>
            </select>
          </label>
          <label class="config-toggle">
            <input
              v-model="configDraft.advanced_enabled"
              type="checkbox"
            >
            <span>Advanced</span>
          </label>
          <div class="config-actions">
            <button
              class="btn btn-primary"
              :disabled="savingConfig"
              @click="saveDecisionMirrorConfig"
            >
              {{ savingConfig ? 'Saving...' : 'Save Config' }}
            </button>
            <button
              class="btn btn-secondary"
              :disabled="savingConfig"
              @click="resetDecisionMirrorConfig"
            >
              Reset
            </button>
          </div>
        </section>

        <section
          v-if="configDraft.advanced_enabled"
          class="workspace-band config-card"
        >
          <div class="panel-header compact">
            <div>
              <h2>Advanced Weights</h2>
              <span>0 ignores the signal, 3 gives it strong priority.</span>
            </div>
          </div>
          <label
            v-for="weight in configWeightRows"
            :key="weight.key"
            class="weight-row"
          >
            <span>{{ weight.label }}</span>
            <input
              v-model.number="configDraft.weights[weight.key]"
              type="range"
              min="0"
              max="3"
              step="0.05"
            >
            <strong>{{ formatWeight(configDraft.weights[weight.key]) }}</strong>
          </label>
        </section>
      </section>
    </main>

    <aside
      v-if="selectedRecordId"
      class="evidence-drawer"
    >
      <div class="drawer-header">
        <h2>Evidence</h2>
        <button @click="selectedRecordId = null">
          x
        </button>
      </div>
      <div
        v-if="evidenceLoading"
        class="empty-panel"
      >
        Loading evidence...
      </div>
      <div
        v-else-if="selectedEvidence.length === 0"
        class="empty-panel"
      >
        No evidence events found.
      </div>
      <article
        v-for="item in selectedEvidence"
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
      v-if="message"
      class="save-toast"
      :class="message.type"
    >
      {{ message.text }}
    </div>
  </div>
</template>

<script setup>
import { computed, defineComponent, h, onMounted, reactive, ref } from 'vue'
import { useRoute } from 'vue-router'
import { twin } from '@/api/client'

const tabs = computed(() => [
  { id: 'overview', label: 'Overview', count: null },
  { id: 'constitution', label: 'Constitution', count: constitutionItems.value.length },
  { id: 'action_gaps', label: 'Action Gaps', count: actionGaps.value.length },
  { id: 'decisions', label: 'Decisions', count: decisions.value.length },
  { id: 'memory', label: 'Memory Review', count: pendingReviewCount.value },
  { id: 'setup', label: 'Setup', count: null },
  { id: 'config', label: 'Config', count: null },
  { id: 'guide', label: 'Guide', count: null }
])

const ReviewActions = defineComponent({
  emits: ['review'],
  setup(_, { emit }) {
    const actions = ['keep', 'soften', 'not_me', 'private', 'no_train', 'reject']
    return () => h('div', { class: 'review-actions' }, actions.map(action =>
      h('button', { onClick: () => emit('review', action) }, actionLabel(action))
    ))
  }
})

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
    const outcome = ref(props.item.episode.outcome || '')
    const lesson = ref(props.item.episode.lesson || '')
    const regretScore = ref(props.item.episode.regret_score ?? '')
    const save = () => emit('update-outcome', props.item.episode.id, {
      outcome: outcome.value || null,
      lesson: lesson.value || null,
      regret_score: regretScore.value === '' ? null : Number(regretScore.value)
    })
    return () => {
      const latestCard = props.item.reflection_cards?.[0]
      const trace = latestCard?.evidence_packet || {}
      const selectedSources = trace.selected_sources || []
      const excludedTotal = (trace.excluded_private_count || 0)
        + (trace.excluded_rejected_count || 0)
        + (trace.excluded_no_train_count || 0)
      const scores = latestCard?.scores || {}
      const feedbackEvents = props.item.feedback_events || []

      return h('article', { class: ['decision-card', props.highlighted ? 'highlighted' : ''] }, [
      h('div', { class: 'card-topline' }, [
        h('span', props.item.episode.review_date || 'No follow-up'),
        h('span', `${props.item.reflection_cards?.length || 0} cards`),
        props.item.episode.confidence != null ? h('span', `${Math.round(props.item.episode.confidence * 100)}% confidence`) : null
      ]),
      h('h3', props.item.episode.decision),
      props.item.episode.options?.length
        ? h('div', { class: 'tag-row' }, props.item.episode.options.map(option => h('span', option)))
        : null,
      h('div', { class: 'primitive-grid' }, primitiveFields.map(field =>
        props.item.episode.primitive_assessment?.[field.key]
          ? h('span', [h('strong', field.label), ` ${props.item.episode.primitive_assessment[field.key]}`])
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
      h('div', { class: 'outcome-row' }, [
        h('input', {
          value: outcome.value,
          placeholder: 'Outcome',
          onInput: event => { outcome.value = event.target.value }
        }),
        h('input', {
          value: lesson.value,
          placeholder: 'Lesson',
          onInput: event => { lesson.value = event.target.value }
        }),
        h('input', {
          value: regretScore.value,
          type: 'number',
          min: '0',
          max: '10',
          placeholder: 'Regret',
          onInput: event => { regretScore.value = event.target.value }
        }),
        h('button', { onClick: save }, 'Save')
      ])
    ])
    }
  }
})

const SetupField = defineComponent({
  props: {
    modelValue: { type: String, default: '' },
    title: { type: String, required: true }
  },
  emits: ['update:modelValue'],
  setup(props, { emit }) {
    return () => h('label', { class: 'setup-field' }, [
      h('span', props.title),
      h('textarea', {
        value: props.modelValue,
        rows: 5,
        onInput: event => emit('update:modelValue', event.target.value)
      })
    ])
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

const tutorialStorageKey = 'grafyn.twinWorkspaceTutorial.dismissed'
const decisionMirrorPresets = [
  { value: 'balanced', label: 'Balanced' },
  { value: 'evidence_strict', label: 'Stricter Evidence' },
  { value: 'insight_search', label: 'Find Blind Spots' },
  { value: 'action_bias', label: 'Push Next Action' }
]
const configWeightRows = [
  { key: 'notes_weight', label: 'Vault Evidence' },
  { key: 'approved_records_weight', label: 'Trusted Self-Model' },
  { key: 'candidate_records_weight', label: 'Tentative Patterns' },
  { key: 'constitution_weight', label: 'Values Fit' },
  { key: 'action_gaps_weight', label: 'Follow-Through Risk' },
  { key: 'recency_weight', label: 'Current Self' },
  { key: 'evidence_count_weight', label: 'Repeated Evidence' },
  { key: 'outcome_history_weight', label: 'Past Outcomes' },
  { key: 'contradiction_weight', label: 'Tensions' },
  { key: 'breadth_weight', label: 'Reflection Breadth' },
  { key: 'depth_weight', label: 'Reflection Depth' },
  { key: 'evidence_grounding_weight', label: 'Grounded Claims' },
  { key: 'blind_spot_weight', label: 'Blind Spots' },
  { key: 'counter_position_weight', label: 'Counterargument' },
  { key: 'actionability_weight', label: 'Next Step Clarity' },
  { key: 'uncertainty_weight', label: 'Honest Uncertainty' },
  { key: 'privacy_weight', label: 'Privacy Safety' },
  { key: 'unsupported_penalty_weight', label: 'Unsupported Claim Penalty' }
]
const tutorialButtonGroups = [
  {
    title: 'Canvas Buttons',
    items: [
      { name: '+ New', description: 'Create a normal prompt or Decision Mirror session.' },
      { name: 'Decision', description: 'Switches the prompt into Decision Mirror mode.' },
      { name: 'Options', description: 'One option per line, useful for comparisons.' },
      { name: 'Stakes', description: 'What changes if the decision goes right or wrong.' },
      { name: 'Initial Leaning', description: 'What the user currently thinks they may choose.' },
      { name: 'Follow-up', description: 'Optional date for outcome review.' },
      { name: 'Create Reflection Card', description: 'Generates the Decision Mirror answer.' },
      { name: 'Useful', description: 'Marks the reflection as useful evidence.' },
      { name: 'Not Me', description: 'Records that the mirror misrepresented the user.' },
      { name: 'Save Insight', description: 'Saves the reflection as a reasoning-pattern insight.' },
      { name: 'Reject Pattern', description: 'Rejects the inferred pattern behind the reflection.' },
      { name: 'Open in Twin', description: 'Jumps to the decision record in Twin Workspace.' }
    ]
  },
  {
    title: 'Twin Workspace Buttons',
    items: [
      { name: 'Run Records', description: 'Infer candidate user records from Canvas traces and feedback.' },
      { name: 'Run Constitution', description: 'Infer Constitution items and Action Gaps from records and decisions.' },
      { name: 'Overview', description: 'Shows health summary, recent decisions, action gaps, and pending review.' },
      { name: 'Constitution', description: 'Review values, tastes, constraints, somatic cues, and reasoning principles.' },
      { name: 'Action Gaps', description: 'Review gaps between stated intent and revealed behavior.' },
      { name: 'Decisions', description: 'Review Reflection Cards and record outcomes.' },
      { name: 'Memory Review', description: 'Review clustered memory digest items.' },
      { name: 'Setup', description: 'Progressively seed values, tastes, constraints, somatic cues, and action tendencies.' },
      { name: 'Keep', description: 'Approve a memory or constitution item for future context.' },
      { name: 'Soften', description: 'Keep it as tentative or lower confidence.' },
      { name: 'Not Me', description: 'Mark it as inaccurate.' },
      { name: 'Private', description: 'Exclude it from live twin context.' },
      { name: 'No Train', description: 'Preserve locally but exclude from model, training, and export use.' },
      { name: 'Reject', description: 'Remove it from future twin context.' },
      { name: 'Save Setup', description: 'Turns setup entries into evidence-backed Constitution seed items.' }
    ]
  },
  {
    title: 'Config Buttons',
    items: [
      { name: 'Preset', description: 'Choose Balanced, Stricter Evidence, Find Blind Spots, or Push Next Action.' },
      { name: 'Advanced', description: 'Reveals scoring and retrieval sliders.' },
      { name: 'Reset', description: 'Restores default Decision Mirror settings.' },
      { name: 'Export Benchmark', description: 'Exports decision episodes, Reflection Cards, scores, evidence packets, and outcomes.' }
    ]
  }
]

const recordStates = ['candidate', 'auto_promoted', 'endorsed', 'private', 'no_train', 'rejected']
const route = useRoute() || { query: {} }
const activeTab = ref(route.query?.decision ? 'decisions' : 'overview')
const reviewRecords = ref([])
const memoryDigestItems = ref([])
const constitutionItems = ref([])
const actionGaps = ref([])
const decisions = ref([])
const selectedRecordState = ref('candidate')
const selectedRecordId = ref(null)
const selectedEvidence = ref([])
const evidenceLoading = ref(false)
const runningTwinInference = ref(false)
const runningConstitutionInference = ref(false)
const savingSetup = ref(false)
const savingConfig = ref(false)
const exportingBenchmark = ref(false)
const message = ref(null)
const showTutorialIntro = ref(localStorage.getItem(tutorialStorageKey) !== 'true')
const setupDraft = reactive({
  twin_name: '',
  twin_role: '',
  source_boundaries: '',
  values: '',
  tastes: '',
  constraints: '',
  somatic_cues: '',
  action_tendencies: ''
})
const configDraft = reactive({
  preset: 'balanced',
  advanced_enabled: false,
  weights: defaultDecisionMirrorWeights()
})

const routeDecisionId = computed(() => String(route.query?.decision || ''))
const routeTraceRequested = computed(() => route.query?.trace === '1')
const activeConstitutionCount = computed(() =>
  constitutionItems.value.filter(item => ['active', 'candidate', 'softened'].includes(item.status)).length
)
const activeActionGapCount = computed(() =>
  actionGaps.value.filter(gap => ['active', 'candidate', 'softened'].includes(gap.status)).length
)
const pendingReviewCount = computed(() =>
  memoryDigestItems.value.length + reviewRecords.value.filter(item => item.record.promotion_state === 'candidate').length
)
const pendingFollowUps = computed(() =>
  decisions.value.filter(item => item.episode.review_date && !item.episode.outcome).length
)
const healthSummary = computed(() =>
  `${constitutionItems.value.length} principles / ${actionGaps.value.length} gaps / ${decisions.value.length} decisions`
)
const recentDecisions = computed(() => decisions.value.slice(0, 4))
const topActionGaps = computed(() => actionGaps.value.slice(0, 4))
const filteredReviewRecords = computed(() =>
  reviewRecords.value.filter(item => item.record.promotion_state === selectedRecordState.value)
)
const groupedConstitution = computed(() => {
  const groups = new Map()
  for (const item of constitutionItems.value) {
    const key = item.dimension || 'general'
    if (!groups.has(key)) groups.set(key, [])
    groups.get(key).push(item)
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([dimension, items]) => ({ dimension, items }))
})

onMounted(loadWorkspace)

async function loadWorkspace() {
  try {
    const [
      review,
      digest,
      constitution,
      gaps,
      decisionRows,
      setup,
      mirrorConfig
    ] = await Promise.all([
      twin.getReview(),
      twin.listMemoryDigest(),
      twin.listConstitutionItems(),
      twin.listActionGaps(),
      twin.listDecisionEpisodes(),
      twin.getConstitutionSetup(),
      twin.getDecisionMirrorConfig()
    ])
    reviewRecords.value = review
    memoryDigestItems.value = digest
    constitutionItems.value = constitution
    actionGaps.value = gaps
    decisions.value = decisionRows
    loadSetupDraft(setup)
    loadConfigDraft(mirrorConfig)
  } catch (err) {
    showMessage('error', err.message || 'Failed to load twin workspace')
  }
}

async function runTwinInference() {
  runningTwinInference.value = true
  try {
    const summary = await twin.runInference()
    await loadWorkspace()
    showMessage('success', `Records: ${summary.created_records} created, ${summary.updated_records} updated`, 3500)
  } catch (err) {
    showMessage('error', err.message || 'Failed to run record inference')
  } finally {
    runningTwinInference.value = false
  }
}

async function runConstitutionInference() {
  runningConstitutionInference.value = true
  try {
    const summary = await twin.runConstitutionInference()
    await loadWorkspace()
    showMessage('success', constitutionRunSummary(summary), 4500)
  } catch (err) {
    showMessage('error', err.message || 'Failed to run constitution inference')
  } finally {
    runningConstitutionInference.value = false
  }
}

async function reviewConstitutionItem(id, action) {
  try {
    await twin.reviewConstitutionItem(id, { action })
    await loadWorkspace()
    showMessage('success', 'Updated constitution item', 1800)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update constitution item')
  }
}

async function reviewActionGap(id, action) {
  try {
    await twin.reviewActionGap(id, { action })
    await loadWorkspace()
    showMessage('success', 'Updated action gap', 1800)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update action gap')
  }
}

async function reviewMemoryDigestItem(id, action) {
  try {
    await twin.reviewMemoryDigestItem(id, { action })
    await loadWorkspace()
    showMessage('success', 'Updated digest item', 1800)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update digest item')
  }
}

async function setPromotion(recordId, promotionState) {
  try {
    await twin.setPromotion(recordId, promotionState, null)
    await loadWorkspace()
    showMessage('success', `Set record to ${statusLabel(promotionState)}`, 1800)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update record')
  }
}

async function openEvidence(recordId) {
  selectedRecordId.value = recordId
  selectedEvidence.value = []
  evidenceLoading.value = true
  try {
    selectedEvidence.value = await twin.resolveEvidence(recordId)
  } catch (err) {
    showMessage('error', err.message || 'Failed to load evidence')
  } finally {
    evidenceLoading.value = false
  }
}

async function updateDecisionOutcome(id, update) {
  try {
    const payload = { ...update }
    if (payload.regret_score == null) delete payload.regret_score
    await twin.updateDecisionOutcome(id, payload)
    await loadWorkspace()
    showMessage('success', 'Updated decision outcome', 1800)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update decision')
  }
}

async function saveSetup() {
  savingSetup.value = true
  try {
    await twin.saveConstitutionSetup({
      twin_name: setupDraft.twin_name.trim(),
      twin_role: setupDraft.twin_role.trim(),
      source_boundaries: splitLines(setupDraft.source_boundaries),
      values: splitLines(setupDraft.values),
      tastes: splitLines(setupDraft.tastes),
      constraints: splitLines(setupDraft.constraints),
      somatic_cues: splitLines(setupDraft.somatic_cues),
      action_tendencies: splitLines(setupDraft.action_tendencies)
    })
    await loadWorkspace()
    showMessage('success', 'Saved setup', 2000)
  } catch (err) {
    showMessage('error', err.message || 'Failed to save setup')
  } finally {
    savingSetup.value = false
  }
}

async function saveDecisionMirrorConfig() {
  savingConfig.value = true
  try {
    const update = {
      preset: configDraft.preset,
      advanced_enabled: configDraft.advanced_enabled
    }
    if (configDraft.advanced_enabled) {
      update.weights = { ...configDraft.weights }
    }
    const config = await twin.updateDecisionMirrorConfig(update)
    loadConfigDraft(config)
    showMessage('success', 'Saved Decision Mirror config', 2000)
  } catch (err) {
    showMessage('error', err.message || 'Failed to save Decision Mirror config')
  } finally {
    savingConfig.value = false
  }
}

async function resetDecisionMirrorConfig() {
  savingConfig.value = true
  try {
    const config = await twin.resetDecisionMirrorConfig()
    loadConfigDraft(config)
    showMessage('success', 'Reset Decision Mirror config', 2000)
  } catch (err) {
    showMessage('error', err.message || 'Failed to reset Decision Mirror config')
  } finally {
    savingConfig.value = false
  }
}

async function exportDecisionBenchmark() {
  exportingBenchmark.value = true
  try {
    const bundle = await twin.exportData({ bundle_name: 'decision-mirror-benchmark' })
    showMessage(
      'success',
      `Exported benchmark: ${bundle.decision_mirror_benchmark?.count || 0} decisions`,
      3500
    )
  } catch (err) {
    showMessage('error', err.message || 'Failed to export benchmark')
  } finally {
    exportingBenchmark.value = false
  }
}

function dismissTutorial() {
  localStorage.setItem(tutorialStorageKey, 'true')
  showTutorialIntro.value = false
}

function loadSetupDraft(setup) {
  setupDraft.twin_name = setup?.twin_name || ''
  setupDraft.twin_role = setup?.twin_role || ''
  setupDraft.source_boundaries = (setup?.source_boundaries || []).join('\n')
  setupDraft.values = (setup?.values || []).join('\n')
  setupDraft.tastes = (setup?.tastes || []).join('\n')
  setupDraft.constraints = (setup?.constraints || []).join('\n')
  setupDraft.somatic_cues = (setup?.somatic_cues || []).join('\n')
  setupDraft.action_tendencies = (setup?.action_tendencies || []).join('\n')
}

function loadConfigDraft(config) {
  configDraft.preset = config?.preset || 'balanced'
  configDraft.advanced_enabled = Boolean(config?.advanced_enabled)
  configDraft.weights = {
    ...defaultDecisionMirrorWeights(),
    ...(config?.weights || {})
  }
}

function defaultDecisionMirrorWeights() {
  return Object.fromEntries(configWeightRows.map(row => [row.key, 1]))
}

function splitLines(value) {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean)
}

function actionLabel(action) {
  return {
    keep: 'Keep',
    soften: 'Soften',
    not_me: 'Not Me',
    private: 'Private',
    no_train: 'No Train',
    reject: 'Reject'
  }[action] || action
}

function statusLabel(value) {
  return String(value || 'unknown')
    .split('_')
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
}

function kindLabel(value) {
  return statusLabel(value)
}

function dimensionLabel(value) {
  return statusLabel(value || 'general')
}

function formatPercent(value) {
  return `${Math.round((value || 0) * 100)}%`
}

function formatWeight(value) {
  return Number(value || 0).toFixed(2)
}

function presetLabel(value) {
  const preset = decisionMirrorPresets.find(item => item.value === value)
  return preset?.label || statusLabel(value || 'balanced')
}

function sourceTypeLabel(value) {
  const labels = {
    note: 'Note',
    behavior: 'Behavior',
    'interview-question': 'Interview Question',
    'interview-answer': 'Interview Answer',
    decision: 'Decision',
    setup: 'Setup',
    approved_record: 'Approved Record',
    candidate_record: 'Candidate Record',
    constitution_item: 'Constitution',
    action_gap: 'Action Gap'
  }
  return labels[value] || statusLabel(value)
}

function constitutionSourceLabel(item) {
  if (item.source) return sourceTypeLabel(item.source)
  const first = item.evidence_refs?.[0]
  return sourceTypeLabel(first?.source_type || 'evidence')
}

function constitutionEvidenceLabels(item) {
  const labels = new Set()
  for (const ref of item.evidence_refs || []) {
    if (ref.source_type) labels.add(sourceTypeLabel(ref.source_type))
    if (ref.source_label) labels.add(ref.source_label)
  }
  return [...labels].slice(0, 4)
}

function constitutionRunSummary(summary = {}) {
  const parts = [
    `${summary.created_constitution_items || 0} items`,
    `${summary.created_action_gaps || 0} gaps`
  ]
  if (summary.auto_active_items) parts.push(`${summary.auto_active_items} active`)
  if (summary.review_candidate_items) parts.push(`${summary.review_candidate_items} review`)
  if (summary.scanned_behavior_events) parts.push(`${summary.scanned_behavior_events} behavior events`)
  if (summary.scanned_notes) parts.push(`${summary.scanned_notes} notes`)
  if (summary.scanned_interviews) parts.push(`${summary.scanned_interviews} interviews`)
  if (summary.extracted_research_findings) parts.push(`${summary.extracted_research_findings} findings`)
  if (summary.pruned_stale_constitution_items) parts.push(`${summary.pruned_stale_constitution_items} stale items removed`)
  if (summary.pruned_stale_records) parts.push(`${summary.pruned_stale_records} stale records removed`)
  if (summary.updated_setup_entries) parts.push(`${summary.updated_setup_entries} setup entries`)
  if (summary.skipped_domain_claims) parts.push(`${summary.skipped_domain_claims} skipped`)
  return `Constitution: ${parts.join(' / ')}`
}

function scoreRows(scores = {}) {
  return [
    ['Breadth', scores.breadth_score],
    ['Depth', scores.depth_score],
    ['Grounding', scores.evidence_grounding_score],
    ['Blind Spot', scores.blind_spot_score],
    ['Action', scores.actionability_score],
    ['Counter', scores.counterargument_score],
    ['Uncertainty', scores.uncertainty_score],
    ['Privacy', scores.privacy_score],
    ['Overall', scores.overall_score]
  ].map(([label, value]) => ({
    label,
    value: value == null ? 'n/a' : formatPercent(value)
  }))
}

function feedbackEventLabel(event = {}) {
  return statusLabel(event.payload?.feedback_type || event.event_type || 'feedback')
}

function feedbackEventNote(event = {}) {
  return event.payload?.rationale
    || event.payload?.content
    || event.payload?.response?.response_excerpt
    || event.payload?.response?.response_content
    || 'Feedback recorded'
}

function formatDate(value) {
  return value ? new Date(value).toLocaleString() : ''
}

function eventLabel(value) {
  return statusLabel(value)
}

function showMessage(type, text, duration = 3500) {
  message.value = { type, text }
  setTimeout(() => {
    if (message.value?.text === text) message.value = null
  }, duration)
}
</script>

<style scoped>
.twin-workspace {
  min-height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
}

.workspace-header {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: var(--spacing-md);
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--border-subtle);
  background: var(--bg-secondary);
}

.header-links,
.header-actions {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.header-actions {
  justify-content: flex-end;
}

.header-actions.compact {
  gap: 6px;
}

.header-links a {
  color: var(--text-secondary);
  text-decoration: none;
}

.header-links a:hover {
  color: var(--accent-primary);
}

.header-title {
  text-align: center;
}

.header-title h1 {
  margin: 0;
  font-size: 1.125rem;
}

.header-title span,
.panel-header span,
.card-topline,
.metric-tile small,
.evidence-item small,
.digest-card small,
.muted {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.workspace-tabs {
  display: flex;
  gap: 4px;
  padding: var(--spacing-sm) var(--spacing-lg);
  border-bottom: 1px solid var(--border-subtle);
  overflow-x: auto;
}

.tab-button {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 34px;
  padding: 0 var(--spacing-sm);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.tab-button.active {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
}

.workspace-main {
  padding: var(--spacing-lg);
}

.tab-panel {
  max-width: 1180px;
  margin: 0 auto;
}

.overview-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--spacing-md);
}

.metric-tile,
.workspace-band,
.constitution-card,
.action-gap-card,
.decision-card,
.digest-card,
.record-card,
.evidence-drawer {
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
}

.metric-tile {
  padding: var(--spacing-md);
}

.metric-tile strong {
  display: block;
  margin: 6px 0;
  font-size: 1.7rem;
}

.workspace-band {
  grid-column: span 2;
  padding: var(--spacing-md);
}

.tutorial-intro {
  grid-column: span 4;
}

.band-header,
.panel-header,
.card-topline,
.drawer-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
}

.panel-header {
  margin-bottom: var(--spacing-md);
}

.panel-header h2,
.band-header h2,
.drawer-header h2 {
  margin: 0;
  font-size: 1rem;
}

.panel-header.compact {
  margin-bottom: var(--spacing-sm);
}

.text-button,
.record-actions button,
.review-actions button,
.outcome-row button,
.drawer-header button {
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.text-button {
  border: none;
  color: var(--accent-primary);
}

.empty-panel {
  padding: var(--spacing-lg);
  border: 1px dashed var(--border-subtle);
  border-radius: var(--radius-md);
  color: var(--text-muted);
  text-align: center;
}

.dimension-section {
  margin-bottom: var(--spacing-lg);
}

.dimension-section h3 {
  margin: 0 0 var(--spacing-sm);
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.constitution-card,
.action-gap-card,
.decision-card,
.digest-card,
.record-card {
  padding: var(--spacing-md);
  margin-bottom: var(--spacing-sm);
}

.decision-card.highlighted {
  border-color: var(--accent-cyan);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-cyan) 18%, transparent);
}

.constitution-card p,
.action-gap-card p,
.decision-card h3,
.digest-card p,
.record-card p,
.evidence-item p {
  margin: var(--spacing-sm) 0;
}

.status-pill,
.tag-row span,
.source-row span {
  display: inline-flex;
  align-items: center;
  min-height: 22px;
  padding: 0 7px;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.6875rem;
}

.tag-row,
.source-row {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.review-actions,
.record-actions,
.outcome-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: var(--spacing-sm);
}

.review-actions button,
.record-actions button,
.outcome-row button {
  min-height: 28px;
  padding: 4px 8px;
}

.review-actions button:hover,
.record-actions button:hover,
.outcome-row button:hover {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
}

.gap-columns {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing-md);
}

.gap-columns small {
  color: var(--text-muted);
}

.primitive-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: var(--spacing-sm) 0;
}

.primitive-grid span {
  padding: 4px 8px;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.reflection-details {
  margin-top: var(--spacing-sm);
}

.context-trace-details {
  margin-top: var(--spacing-sm);
  border-top: 1px solid var(--border-subtle);
  padding-top: var(--spacing-sm);
}

.context-trace-details summary,
.reflection-details summary {
  cursor: pointer;
  font-weight: 700;
}

.reflection-details pre {
  max-height: 220px;
  overflow: auto;
  white-space: pre-wrap;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.trace-metrics,
.trace-score-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: 6px;
  margin: var(--spacing-sm) 0;
}

.trace-metrics span,
.trace-score-grid span {
  padding: 6px 8px;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.78rem;
}

.trace-warning {
  padding: 6px 8px;
  border: 1px solid color-mix(in srgb, var(--accent-warning) 55%, transparent);
  border-radius: var(--radius-sm);
  color: var(--accent-warning);
  background: color-mix(in srgb, var(--accent-warning) 10%, transparent);
}

.trace-section {
  margin-top: var(--spacing-sm);
}

.trace-section h4 {
  margin: 0 0 6px;
  font-size: 0.78rem;
  color: var(--text-primary);
}

.trace-source-list,
.trace-feedback-list {
  display: grid;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.trace-source-list li,
.trace-feedback-list li {
  display: grid;
  gap: 2px;
  padding: 7px 8px;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: var(--bg-primary);
}

.trace-source-list li span,
.trace-feedback-list li span {
  color: var(--accent-cyan);
  font-size: 0.72rem;
  font-weight: 700;
}

.trace-source-list li small,
.trace-feedback-list li small {
  color: var(--text-secondary);
}

.outcome-row input,
.config-row select,
.identity-field input,
.setup-field textarea,
.panel-header select {
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: var(--bg-primary);
  color: var(--text-primary);
}

.outcome-row input {
  min-height: 30px;
  padding: 4px 8px;
}

.memory-grid,
.setup-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing-md);
}

.guide-panel,
.config-panel {
  display: grid;
  gap: var(--spacing-md);
}

.guide-panel .workspace-band,
.config-panel .workspace-band {
  grid-column: auto;
}

.guide-walkthrough ol {
  margin: var(--spacing-sm) 0 0;
  padding-left: 1.2rem;
}

.guide-group dl {
  display: grid;
  grid-template-columns: minmax(120px, 180px) 1fr;
  gap: 8px var(--spacing-md);
  margin: var(--spacing-sm) 0 0;
}

.guide-group dt {
  font-weight: 700;
}

.guide-group dd {
  margin: 0;
  color: var(--text-secondary);
}

.config-row,
.config-toggle,
.weight-row,
.config-actions {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.config-row,
.weight-row {
  justify-content: space-between;
}

.config-row select {
  min-height: 32px;
  padding: 4px 8px;
}

.weight-row {
  margin-top: var(--spacing-sm);
}

.weight-row span {
  min-width: 160px;
}

.weight-row input {
  flex: 1;
}

.weight-row strong {
  min-width: 44px;
  text-align: right;
}

.setup-field {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  padding: var(--spacing-md);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
}

.identity-section,
.setup-section-heading {
  grid-column: 1 / -1;
}

.identity-section {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing-md);
}

.identity-section > div {
  grid-column: 1 / -1;
}

.identity-section h2,
.setup-section-heading h2 {
  margin: 0;
}

.identity-section span,
.setup-section-heading span {
  color: var(--text-secondary);
}

.identity-field {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.identity-field span {
  font-weight: 600;
  color: var(--text-primary);
}

.identity-field input {
  min-height: 36px;
  padding: 4px 8px;
}

.identity-section .setup-field {
  grid-column: 1 / -1;
}

.setup-field span {
  font-weight: 600;
}

.setup-field textarea {
  resize: vertical;
  padding: var(--spacing-sm);
}

.setup-actions {
  display: flex;
  align-items: flex-start;
}

.evidence-drawer {
  position: fixed;
  top: 96px;
  right: var(--spacing-lg);
  width: min(420px, calc(100vw - 32px));
  max-height: calc(100vh - 128px);
  overflow: auto;
  padding: var(--spacing-md);
  box-shadow: var(--shadow-xl);
  z-index: 40;
}

.evidence-item {
  padding: var(--spacing-sm) 0;
  border-top: 1px solid var(--border-subtle);
}

.save-toast {
  position: fixed;
  bottom: var(--spacing-lg);
  right: var(--spacing-lg);
  padding: var(--spacing-sm) var(--spacing-md);
  border-radius: var(--radius-sm);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  box-shadow: var(--shadow-lg);
}

.save-toast.success {
  border-color: var(--accent-green);
}

.save-toast.error {
  border-color: var(--accent-red);
}

@media (max-width: 980px) {
  .workspace-header,
  .overview-grid,
  .identity-section,
  .memory-grid,
  .setup-grid,
  .gap-columns {
    grid-template-columns: 1fr;
  }

  .workspace-band {
    grid-column: span 1;
  }

  .header-title {
    text-align: left;
  }
}
</style>
