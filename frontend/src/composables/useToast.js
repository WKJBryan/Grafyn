import { reactive } from 'vue'

const state = reactive({
  toasts: []
})

let nextId = 0

function addToast(message, type = 'info', duration = 4000) {
  const id = ++nextId
  state.toasts.push({ id, message, type, duration })
  if (duration > 0) {
    setTimeout(() => removeToast(id), duration)
  }
  return id
}

function removeToast(id) {
  const idx = state.toasts.findIndex(t => t.id === id)
  if (idx !== -1) state.toasts.splice(idx, 1)
}

export function useToast() {
  return {
    toasts: state.toasts,
    success: (msg, duration) => addToast(msg, 'success', duration),
    error: (msg, duration) => addToast(msg, 'error', duration ?? 6000),
    warning: (msg, duration) => addToast(msg, 'warning', duration),
    info: (msg, duration) => addToast(msg, 'info', duration),
    remove: removeToast,
  }
}
