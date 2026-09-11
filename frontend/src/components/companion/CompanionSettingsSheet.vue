<template>
  <div
    class="settings-sheet-overlay"
    @click.self="close"
  >
    <section
      ref="dialog"
      class="settings-sheet"
      role="dialog"
      aria-modal="true"
      aria-labelledby="companion-settings-title"
      tabindex="-1"
    >
      <header class="settings-sheet-header">
        <div>
          <p>On-device companion</p>
          <h1 id="companion-settings-title">
            Settings
          </h1>
        </div>
        <button
          type="button"
          aria-label="Close companion settings"
          @click="close"
        >
          <GIcon
            name="x"
            :size="20"
            aria-hidden="true"
          />
        </button>
      </header>

      <div class="settings-sheet-body">
        <p
          v-if="loadError"
          class="settings-notice is-error"
          role="alert"
        >
          Companion settings are temporarily unavailable.
        </p>

        <section
          class="settings-section"
          aria-labelledby="companion-theme-heading"
        >
          <div class="section-heading">
            <div>
              <p>Appearance</p>
              <h2 id="companion-theme-heading">
                Theme
              </h2>
            </div>
            <span v-if="themeSaving">Saving…</span>
          </div>
          <div class="segmented-options">
            <label
              v-for="option in themeOptions"
              :key="option.value"
              :class="{ active: theme === option.value }"
            >
              <input
                v-model="theme"
                type="radio"
                :value="option.value"
                :disabled="themeSaving"
                @change="saveTheme"
              >
              <span>{{ option.label }}</span>
            </label>
          </div>
        </section>

        <section
          class="settings-section"
          aria-labelledby="companion-vault-heading"
        >
          <div class="section-heading">
            <div>
              <p>Local data</p>
              <h2 id="companion-vault-heading">
                Vault
              </h2>
            </div>
            <span :class="vaultAvailable ? 'is-ready' : 'is-muted'">
              {{ vaultAvailable ? 'Ready' : 'Unavailable' }}
            </span>
          </div>
          <p class="section-copy">
            {{ vaultSummary }}
          </p>
          <p class="privacy-note">
            Its filesystem path is never shown in the companion surface.
          </p>
        </section>

        <section
          class="settings-section"
          aria-labelledby="companion-openrouter-heading"
        >
          <div class="section-heading">
            <div>
              <p>Secure provider</p>
              <h2 id="companion-openrouter-heading">
                OpenRouter
              </h2>
            </div>
            <span :class="openRouterConfigured ? 'is-ready' : 'is-muted'">
              {{ openRouterLabel }}
            </span>
          </div>
          <p class="section-copy">
            {{ secureSecretCopy }}
          </p>
          <label class="secret-field">
            <span>Replace API key</span>
            <input
              v-model="openRouterKey"
              data-testid="openrouter-key"
              type="password"
              autocomplete="off"
              autocapitalize="none"
              spellcheck="false"
              placeholder="sk-or-v1-…"
              :disabled="!secureSecretsReady || keySaving"
              @copy.prevent
              @cut.prevent
            >
          </label>
          <p
            v-if="keyError"
            class="settings-notice is-error"
            role="alert"
          >
            {{ keyError }}
          </p>
          <button
            type="button"
            class="primary-action"
            data-testid="save-openrouter-key"
            :disabled="!secureSecretsReady || keySaving || openRouterKey.trim().length < 10"
            @click="saveOpenRouterKey"
          >
            {{ keySaving ? 'Saving securely…' : 'Save key securely' }}
          </button>
        </section>

        <section
          class="settings-section"
          aria-labelledby="companion-sync-heading"
        >
          <div class="section-heading">
            <div>
              <p>Encrypted continuity</p>
              <h2 id="companion-sync-heading">
                Sync
              </h2>
            </div>
            <span :class="syncAvailable ? 'is-ready' : 'is-muted'">
              {{ syncLabel }}
            </span>
          </div>
          <p class="section-copy">
            {{ syncCopy }}
          </p>
        </section>

        <section
          class="settings-section"
          aria-labelledby="companion-diagnostics-heading"
        >
          <div class="section-heading">
            <div>
              <p>Runtime truth</p>
              <h2 id="companion-diagnostics-heading">
                Capability diagnostics
              </h2>
            </div>
          </div>
          <dl class="capability-list">
            <div
              v-for="item in capabilityDiagnostics"
              :key="item.name"
            >
              <dt>{{ item.label }}</dt>
              <dd :class="item.enabled ? 'is-ready' : 'is-muted'">
                {{ item.enabled ? 'Ready' : 'Unavailable' }}
              </dd>
            </div>
          </dl>
          <ul
            v-if="diagnosticCodes.length"
            class="diagnostic-codes"
            aria-label="Runtime diagnostic codes"
          >
            <li
              v-for="code in diagnosticCodes"
              :key="code"
            >
              {{ code }}
            </li>
          </ul>
        </section>
      </div>
    </section>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { settings as settingsApi, sync as syncApi } from '@/api/client'
import { getRuntimeProfile, getRuntimeStatus } from '@/api/transport'
import { CAPABILITY_NAMES, hasCapability } from '@/platform/capabilities'
import { resolveThemePreference, useThemeStore } from '@/stores/theme'
import GIcon from '@/components/ui/GIcon.vue'

const emit = defineEmits(['close'])
const runtimeProfile = getRuntimeProfile()
const runtimeStatus = computed(() => getRuntimeStatus())
const themeStore = useThemeStore()
const dialog = ref(null)
const theme = ref('system')
const themeSaving = ref(false)
const openRouterKey = ref('')
const openRouterConfigured = ref(false)
const keySaving = ref(false)
const keyError = ref('')
const syncState = ref(null)
const loadError = ref(false)
let opener = null

const themeOptions = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]
const capabilityLabels = Object.freeze({
  notesRead: 'Read notes',
  notesWrite: 'Capture notes',
  recall: 'Recall',
  twinReview: 'Twin review',
  twinChat: 'Twin chat',
  linearCanvas: 'Linear Canvas',
  imageGeneration: 'Image generation',
  nativeImageShare: 'Native image share',
  sync: 'Encrypted sync',
})

const secureSecretsReady = computed(() => runtimeStatus.value?.secureSecrets?.status === 'ready')
const vaultAvailable = computed(() => runtimeStatus.value?.vault?.available === true)
const syncAvailable = computed(() => hasCapability(runtimeProfile, 'sync', runtimeStatus.value))
const vaultSummary = computed(() => {
  if (!vaultAvailable.value) return 'The local vault is not available in this runtime.'
  return runtimeStatus.value?.vault?.kind === 'app_private'
    ? 'Private on-device vault'
    : 'User-selected local vault'
})
const secureSecretCopy = computed(() => secureSecretsReady.value
  ? 'The key is stored by the operating system and is never returned to this screen.'
  : 'Secure storage is unavailable, so OpenRouter and other secret-dependent features stay off.')
const openRouterLabel = computed(() => {
  if (!secureSecretsReady.value) return 'Unavailable'
  return openRouterConfigured.value ? 'Stored securely' : 'Not configured'
})
const syncLabel = computed(() => {
  if (!syncAvailable.value) return 'Unavailable'
  return ({
    not_provisioned: 'Not provisioned',
    local_only: 'Local only',
    pending: 'Pending',
    conflict: 'Conflict',
    error: 'Unavailable',
  })[syncState.value?.status] || 'Ready'
})
const syncCopy = computed(() => {
  if (!syncAvailable.value) return 'Sync is unavailable until secure storage is healthy.'
  if (syncState.value?.status === 'local_only') {
    return 'Encrypted changes remain on this device until a transport is configured.'
  }
  return 'Only the local encrypted sync foundation is reported here; no hosted relay is implied.'
})
const capabilityDiagnostics = computed(() => CAPABILITY_NAMES
  .filter(name => Object.hasOwn(capabilityLabels, name))
  .map(name => ({
    name,
    label: capabilityLabels[name],
    enabled: hasCapability(runtimeProfile, name, runtimeStatus.value),
  })))
const diagnosticCodes = computed(() => (runtimeStatus.value?.diagnostics || [])
  .map(diagnostic => diagnostic.code))

onMounted(() => {
  opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
  window.addEventListener('keydown', handleKeydown, true)
  dialog.value?.focus()
  void loadSettings()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeydown, true)
  if (opener?.isConnected) opener.focus()
})

async function loadSettings() {
  try {
    const current = await settingsApi.get()
    theme.value = ['system', 'light', 'dark'].includes(current?.theme)
      ? current.theme
      : 'system'
    const requests = []
    if (secureSecretsReady.value) requests.push(loadOpenRouterStatus())
    if (syncAvailable.value) requests.push(loadSyncStatus())
    await Promise.all(requests)
  } catch {
    loadError.value = true
  }
}

async function loadOpenRouterStatus() {
  try {
    const status = await settingsApi.getOpenRouterStatus()
    openRouterConfigured.value = status?.has_key === true && status?.is_configured === true
  } catch {
    openRouterConfigured.value = false
  }
}

async function loadSyncStatus() {
  try {
    syncState.value = await syncApi.getStatus()
  } catch {
    syncState.value = { status: 'error' }
  }
}

async function saveTheme() {
  themeSaving.value = true
  try {
    await settingsApi.update({ theme: theme.value })
    themeStore.setTheme(resolveThemePreference(theme.value))
  } catch {
    loadError.value = true
  } finally {
    themeSaving.value = false
  }
}

async function saveOpenRouterKey() {
  if (!secureSecretsReady.value || keySaving.value) return
  const secret = openRouterKey.value.trim()
  if (secret.length < 10) return

  keySaving.value = true
  keyError.value = ''
  try {
    await settingsApi.update({ openrouter_api_key: secret })
    openRouterKey.value = ''
    await loadOpenRouterStatus()
  } catch {
    keyError.value = 'The key could not be stored securely.'
  } finally {
    keySaving.value = false
  }
}

function handleKeydown(event) {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    close()
    return
  }
  if (event.key === 'Tab') trapFocus(event)
}

function trapFocus(event) {
  const controls = [...(dialog.value?.querySelectorAll(
    'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  ) || [])]
  event.stopImmediatePropagation()
  if (!controls.length) {
    event.preventDefault()
    dialog.value?.focus()
    return
  }
  const first = controls[0]
  const last = controls.at(-1)
  const focusOutside = !dialog.value?.contains(document.activeElement)
  if (event.shiftKey && (document.activeElement === first || focusOutside)) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && (document.activeElement === last || focusOutside)) {
    event.preventDefault()
    first.focus()
  }
}

function close() {
  emit('close')
}
</script>

<style scoped>
.settings-sheet-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-top: env(safe-area-inset-top);
  background: rgba(4, 7, 10, 0.72);
  backdrop-filter: blur(4px);
}

.settings-sheet {
  width: min(34rem, 100%);
  max-height: calc(100dvh - env(safe-area-inset-top));
  min-width: 0;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  overflow: hidden;
  color: var(--text-primary);
  background: var(--bg-primary);
  border: 1px solid var(--border-default);
  border-bottom: 0;
  border-radius: 18px 18px 0 0;
  box-shadow: 0 -20px 64px rgba(0, 0, 0, 0.42);
}

.settings-sheet-header {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  padding: var(--spacing-md) max(var(--spacing-md), env(safe-area-inset-right)) var(--spacing-sm) max(var(--spacing-md), env(safe-area-inset-left));
  border-bottom: 1px solid var(--border-subtle);
}

.settings-sheet-header p,
.section-heading p {
  margin: 0 0 2px;
  color: var(--text-muted);
  font-size: 0.67rem;
  font-weight: 700;
  letter-spacing: 0.07em;
  text-transform: uppercase;
}

.settings-sheet-header h1,
.section-heading h2 {
  margin: 0;
}

.settings-sheet-header h1 {
  font-size: 1.35rem;
}

.settings-sheet-header button,
.primary-action {
  min-width: 44px;
  min-height: 44px;
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.settings-sheet-header button {
  display: grid;
  place-items: center;
  padding: 0;
}

.settings-sheet-body {
  min-width: 0;
  display: grid;
  gap: var(--spacing-sm);
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: var(--spacing-md) max(var(--spacing-md), env(safe-area-inset-right)) max(var(--spacing-xl), env(safe-area-inset-bottom)) max(var(--spacing-md), env(safe-area-inset-left));
}

.settings-section {
  min-width: 0;
  display: grid;
  gap: var(--spacing-sm);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
}

.section-heading {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--spacing-sm);
}

.section-heading h2 {
  font-size: 1rem;
}

.section-heading > span,
.capability-list dd {
  flex: none;
  margin: 0;
  font-size: 0.72rem;
  font-weight: 700;
}

.section-copy,
.privacy-note,
.settings-notice {
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--text-secondary);
  font-size: 0.8rem;
  line-height: 1.5;
}

.privacy-note {
  color: var(--text-muted);
  font-size: 0.72rem;
}

.segmented-options {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 4px;
  padding: 4px;
  background: var(--bg-primary);
  border-radius: var(--radius-md);
}

.segmented-options label {
  min-width: 0;
  min-height: 44px;
  display: grid;
  place-items: center;
  border-radius: calc(var(--radius-md) - 2px);
  color: var(--text-muted);
  font-size: 0.78rem;
  font-weight: 700;
}

.segmented-options label.active {
  color: var(--text-primary);
  background: var(--bg-tertiary);
}

.segmented-options input {
  position: absolute;
  opacity: 0;
  pointer-events: none;
}

.secret-field {
  min-width: 0;
  display: grid;
  gap: 5px;
  color: var(--text-secondary);
  font-size: 0.74rem;
  font-weight: 650;
}

.secret-field input {
  width: 100%;
  min-width: 0;
  min-height: 44px;
  padding: 0 var(--spacing-sm);
  color: var(--text-primary);
  background: var(--bg-primary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.primary-action {
  width: 100%;
  padding: 0 var(--spacing-md);
  font-size: 0.8rem;
  font-weight: 700;
}

button:disabled,
input:disabled {
  cursor: default;
  opacity: 0.5;
}

button:focus-visible,
input:focus-visible,
.segmented-options label:has(input:focus-visible) {
  outline: 2px solid var(--accent-cyan);
  outline-offset: 2px;
}

.capability-list {
  display: grid;
  gap: 1px;
  margin: 0;
  overflow: hidden;
  background: var(--border-subtle);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.capability-list div {
  min-width: 0;
  min-height: 44px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
  padding: 0 var(--spacing-sm);
  background: var(--bg-primary);
}

.capability-list dt {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 0.78rem;
}

.diagnostic-codes {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.diagnostic-codes li {
  max-width: 100%;
  padding: 4px 7px;
  overflow-wrap: anywhere;
  color: var(--text-muted);
  background: var(--bg-primary);
  border-radius: 999px;
  font: 0.68rem/1.3 ui-monospace, SFMono-Regular, Consolas, monospace;
}

.is-ready {
  color: var(--accent-cyan);
}

.is-muted {
  color: var(--text-muted);
}

.is-error {
  color: var(--accent-danger);
}

@media (min-width: 35rem) {
  .settings-sheet-overlay {
    padding: max(var(--spacing-md), env(safe-area-inset-top));
  }

  .settings-sheet {
    max-height: calc(100dvh - 2 * var(--spacing-md));
    border-bottom: 1px solid var(--border-default);
    border-radius: 18px;
  }
}
</style>
