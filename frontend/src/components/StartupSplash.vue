<template>
  <div
    class="startup-splash"
    :data-phase="status.phase"
  >
    <div class="startup-shell">
      <div class="startup-orbit">
        <div class="startup-core" />
        <div class="startup-ring startup-ring-primary" />
        <div class="startup-ring startup-ring-secondary" />
      </div>

      <div class="startup-copy">
        <p class="startup-kicker">
          Grafyn
        </p>
        <h1 class="startup-title">
          <span v-if="status.error">Startup hit a problem</span>
          <span v-else>Loading your knowledge workspace</span>
        </h1>
        <p class="startup-message">
          {{ status.error || status.message }}
        </p>
      </div>

      <div
        v-if="!status.error"
        class="startup-progress"
        aria-hidden="true"
      >
        <div class="startup-progress-track">
          <div class="startup-progress-bar" />
        </div>
      </div>

      <button
        v-else
        class="btn btn-primary"
        type="button"
        @click="$emit('dismiss')"
      >
        Continue to app
      </button>
    </div>
  </div>
</template>

<script setup>
defineProps({
  status: {
    type: Object,
    required: true,
  },
})

defineEmits(['dismiss'])
</script>

<style scoped>
.startup-splash {
  position: fixed;
  inset: 0;
  z-index: 1500;
  display: grid;
  place-items: center;
  padding: 24px;
  background:
    radial-gradient(circle at top, rgba(38, 139, 210, 0.2), transparent 42%),
    linear-gradient(180deg, #021f27 0%, #002b36 55%, #03161b 100%);
}

.startup-shell {
  width: min(540px, 100%);
  padding: 32px 28px;
  border: 1px solid rgba(131, 148, 150, 0.12);
  border-radius: 24px;
  background: rgba(7, 54, 66, 0.78);
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
  backdrop-filter: blur(14px);
}

.startup-orbit {
  position: relative;
  width: 108px;
  height: 108px;
  margin-bottom: 28px;
}

.startup-core,
.startup-ring {
  position: absolute;
  inset: 0;
  border-radius: 999px;
}

.startup-core {
  inset: 24px;
  background: radial-gradient(circle at 30% 30%, #6c71c4, #268bd2 70%, #2aa198 100%);
  box-shadow: 0 0 40px rgba(108, 113, 196, 0.4);
}

.startup-ring {
  border: 1px solid rgba(147, 161, 161, 0.22);
}

.startup-ring-primary {
  animation: orbit-spin 8s linear infinite;
}

.startup-ring-secondary {
  inset: 10px;
  border-style: dashed;
  animation: orbit-spin-reverse 5.5s linear infinite;
}

.startup-copy {
  margin-bottom: 20px;
}

.startup-kicker {
  margin: 0 0 8px;
  color: #2aa198;
  font-size: 0.75rem;
  font-weight: 700;
  letter-spacing: 0.28em;
  text-transform: uppercase;
}

.startup-title {
  margin: 0 0 10px;
  color: #f2f7f8;
  font-size: clamp(1.7rem, 4vw, 2.4rem);
  line-height: 1.05;
}

.startup-message {
  margin: 0;
  color: #93a1a1;
  font-size: 0.95rem;
}

.startup-progress-track {
  position: relative;
  height: 10px;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(10, 64, 80, 0.95);
}

.startup-progress-bar {
  width: 35%;
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, #2aa198, #268bd2, #6c71c4);
  animation: startup-progress 1.6s ease-in-out infinite;
}

@keyframes startup-progress {
  0% { transform: translateX(-120%); }
  100% { transform: translateX(320%); }
}

@keyframes orbit-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

@keyframes orbit-spin-reverse {
  from { transform: rotate(360deg); }
  to { transform: rotate(0deg); }
}
</style>
