<template>
  <div id="app">
    <CompanionShell v-if="isCompact">
      <router-view />
    </CompanionShell>
    <router-view v-else />
    <ToastNotification />
    <GuidePanel v-if="!isCompact" />
    <GuideTip v-if="!isCompact" />
    <Transition name="startup-splash">
      <StartupSplash
        v-if="boot.isVisible"
        :status="boot.status"
        @dismiss="boot.dismissSplash()"
      />
    </Transition>
  </div>
</template>

<script setup>
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import ToastNotification from '@/components/ToastNotification.vue'
import GuidePanel from '@/components/GuidePanel.vue'
import GuideTip from '@/components/GuideTip.vue'
import StartupSplash from '@/components/StartupSplash.vue'
import CompanionShell from '@/components/companion/CompanionShell.vue'
import { isDesktopApp, isTauriApp } from '@/api/client'
import { getRuntimeProfile, getTransport } from '@/api/transport'
import { useBootStore } from '@/stores/boot'
import { useGuide } from '@/composables/useGuide'
import { useToast } from '@/composables/useToast'
import { useCompanionLayout } from '@/composables/useCompanionLayout'

const COMMITTED_WARNING_EVENT = 'grafyn://committed-warning'
const DEGRADED_READINESS_MESSAGE =
  'Your change was saved, but derived views are temporarily unavailable.'
const COMMITTED_WARNING_MESSAGES = new Map([
  ['derived_state_unavailable', DEGRADED_READINESS_MESSAGE],
  [
    'optimizer_publication_pending',
    'Your change was saved, but its optimizer audit publication is still pending.',
  ],
  [
    'optimizer_rollback_recovery_pending',
    'The rollback is accepted, but restoring the target bytes is still pending. Do not retry.',
  ],
  [
    'optimizer_rollback_not_applied',
    'The rollback did not restore target bytes after authority advanced.',
  ],
])
const UNKNOWN_COMMITTED_WARNING_MESSAGE =
  'Your change was saved, but follow-up work is temporarily unavailable.'

const route = useRoute()
const guide = useGuide()
const boot = useBootStore()
const toast = useToast()
const { isCompact } = useCompanionLayout()

const seenCommittedWarnings = new Set()
let committedWarningRegistrationStarted = false
let committedWarningUnlisten = null
let appUnmounted = false

function committedWarningIdentity(payload) {
  const warning = payload?.warning ?? payload
  return JSON.stringify({
    operation: payload?.operation ?? warning?.operation ?? null,
    code: warning?.code ?? null,
  })
}

function handleCommittedWarning(event) {
  if (appUnmounted) return

  const identity = committedWarningIdentity(event?.payload)
  if (seenCommittedWarnings.has(identity)) return

  seenCommittedWarnings.add(identity)
  const warning = event?.payload?.warning ?? event?.payload
  toast.warning(
    COMMITTED_WARNING_MESSAGES.get(warning?.code) ?? UNKNOWN_COMMITTED_WARNING_MESSAGE,
  )
}

async function registerCommittedWarningListener() {
  if (committedWarningRegistrationStarted) return
  committedWarningRegistrationStarted = true

  try {
    const unlisten = await getTransport().listen(
      COMMITTED_WARNING_EVENT,
      handleCommittedWarning,
    )
    if (appUnmounted) {
      unlisten()
      return
    }
    committedWarningUnlisten = unlisten
  } catch (error) {
    if (!appUnmounted) {
      console.error('Failed to listen for committed mutation warnings:', error)
    }
  }
}

function handleExternalLinkClick(event) {
  if (getRuntimeProfile().nativePlugins !== true) return

  let el = event.target
  while (el && el.tagName !== 'A') {
    el = el.parentElement
  }
  if (!el) return

  const href = el.getAttribute('href')
  if (href && (href.startsWith('http://') || href.startsWith('https://'))) {
    event.preventDefault()
    getTransport().openExternal({ type: 'url', url: href })
      .catch((error) => console.error('Failed to open external link:', error))
  }
}

async function checkForDesktopUpdate() {
  if (getRuntimeProfile().nativePlugins !== true) return

  try {
    const [{ check }, { confirm }] = await Promise.all([
      import('@tauri-apps/plugin-updater'),
      import('@tauri-apps/plugin-dialog'),
    ])
    const update = await check()
    if (!update) return

    const shouldInstall = await confirm(
      `Grafyn ${update.version} is available. Install it now?`,
      {
        title: 'Grafyn update',
        kind: 'info',
        okLabel: 'Install',
        cancelLabel: 'Later',
      },
    )
    if (shouldInstall) {
      await update.downloadAndInstall()
      const currentPlatform = getRuntimeProfile().platform
      if (currentPlatform === 'macos' || currentPlatform === 'linux') {
        const { relaunch } = await import('@tauri-apps/plugin-process')
        await relaunch()
      }
    }
  } catch (error) {
    console.error('Failed to check for Grafyn updates:', error)
  }
}

onMounted(() => {
  window.dispatchEvent(new Event('grafyn-app-mounted'))
  boot.initialize()
  guide.setCurrentRoute(route.path)
  guide.checkNewFeatures()

  if (isTauriApp()) {
    void registerCommittedWarningListener()
  }
  if (getRuntimeProfile().nativePlugins === true) {
    document.addEventListener('click', handleExternalLinkClick)
  }
  if (isDesktopApp() && getRuntimeProfile().nativePlugins === true) {
    void checkForDesktopUpdate()
  }
})

let tipTimer = null
watch(() => route.path, (path) => {
  guide.setCurrentRoute(path)
  clearTimeout(tipTimer)
  tipTimer = setTimeout(() => {
    guide.showTipForRoute(path)
  }, 800)
})

onBeforeUnmount(() => {
  appUnmounted = true
  boot.cleanup()
  document.removeEventListener('click', handleExternalLinkClick)
  if (typeof committedWarningUnlisten === 'function') {
    committedWarningUnlisten()
    committedWarningUnlisten = null
  }
})
</script>

<style>
/* Global styles are imported in main.js */
.startup-splash-enter-active,
.startup-splash-leave-active {
  transition: opacity 0.3s ease;
}

.startup-splash-enter-from,
.startup-splash-leave-to {
  opacity: 0;
}
</style>
