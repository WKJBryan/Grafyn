<script>
// Extracted verbatim from TwinReviewView.vue (Task 4.3). Render-function
// component — no template block needed since it's built with h().
import { defineComponent, h } from 'vue'

export default defineComponent({
  name: 'SetupField',
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
</script>
