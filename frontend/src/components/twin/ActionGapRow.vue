<script>
// Extracted verbatim from TwinReviewView.vue (Task 4.3). Render-function
// component — no template block needed since it's built with h().
import { defineComponent, h } from 'vue'
import { statusLabel, formatPercent } from '@/utils/twinFormat'
import ReviewActions from './ReviewActions.vue'

export default defineComponent({
  name: 'ActionGapRow',
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
</script>
