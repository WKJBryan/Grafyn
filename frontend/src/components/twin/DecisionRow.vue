<script>
// Extracted verbatim from TwinReviewView.vue (Task 4.3). Render-function
// component — no template block needed since it's built with h().
import { defineComponent, h, ref } from 'vue'
import {
  decisionState,
  decisionChips,
  presetLabel,
  sourceTypeLabel,
  formatWeight,
  scoreRows,
  feedbackEventLabel,
  feedbackEventNote
} from '@/utils/twinFormat'

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

export default defineComponent({
  name: 'DecisionRow',
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
</script>
