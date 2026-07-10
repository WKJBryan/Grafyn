<script>
// Extracted verbatim from TwinReviewView.vue (Task 4.3). Shared by the
// Constitution, Action Gaps, and Memory Review tabs. Render-function
// component — no template block needed since it's built with h().
import { defineComponent, h } from 'vue'
import { actionLabel } from '@/utils/twinFormat'

export default defineComponent({
  name: 'ReviewActions',
  emits: ['review'],
  setup(_, { emit }) {
    const actions = ['keep', 'soften', 'not_me', 'private', 'no_train', 'reject']
    return () => h('div', { class: 'review-actions' }, actions.map(action =>
      h('button', { onClick: () => emit('review', action) }, actionLabel(action))
    ))
  }
})
</script>
