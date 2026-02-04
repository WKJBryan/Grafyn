<template>
  <div
    v-if="isOpen"
    class="modal-overlay"
    @click.self="handleClose"
  >
    <div class="modal-content settings-modal">
      <div class="modal-header">
        <h2>{{ isSetup ? '🚀 Welcome to Seedream' : '⚙️ Settings' }}</h2>
        <button
          v-if="!isSetup"
          class="close-btn"
          @click="handleClose"
        >
          ×
        </button>
      </div>

      <div class="modal-body">
        <p
          v-if="isSetup"
          class="setup-intro"
        >
          Let's set up your knowledge base. You can change these settings anytime.
        </p>

        <!-- Vault Path Section -->
        <div class="setting-section">
          <label class="setting-label">
            <span class="label-icon">📁</span>
            Vault Location
          </label>
          <p class="setting-description">
            Choose where to store your notes. You can use an existing Obsidian vault or create a new folder.
          </p>
          <div class="vault-picker">
            <input
              v-model="vaultPath"
              type="text"
              class="vault-input"
              placeholder="Click 'Browse' to select a folder..."
              readonly
            >
            <button
              class="browse-btn"
              :disabled="isLoading"
              @click="pickVaultFolder"
            >
              {{ isLoading ? '...' : 'Browse' }}
            </button>
          </div>
          <p
            v-if="!vaultPath && isSetup"
            class="setting-hint warning"
          >
            ⚠️ Please select a vault location to continue
          </p>
        </div>

        <!-- OpenRouter API Key Section -->
        <div class="setting-section">
          <label class="setting-label">
            <span class="label-icon">🤖</span>
            OpenRouter API Key
            <span class="optional-badge">Optional</span>
          </label>
          <p class="setting-description">
            Required for Canvas multi-LLM features. Get a key at
            <a
              href="https://openrouter.ai/keys"
              target="_blank"
              rel="noopener"
            >openrouter.ai/keys</a>
          </p>
          <div class="api-key-input">
            <input
              v-model="openrouterKey"
              :type="showApiKey ? 'text' : 'password'"
              class="key-input"
              placeholder="sk-or-v1-..."
              @blur="validateKey"
            >
            <button
              class="toggle-visibility"
              type="button"
              @click="showApiKey = !showApiKey"
            >
              {{ showApiKey ? '🙈' : '👁️' }}
            </button>
          </div>
          <div
            v-if="keyValidationState"
            class="validation-status"
            :class="keyValidationState"
          >
            <span v-if="keyValidationState === 'validating'">⏳ Validating...</span>
            <span v-else-if="keyValidationState === 'valid'">✅ API key is valid</span>
            <span v-else-if="keyValidationState === 'invalid'">❌ Invalid API key</span>
          </div>
          <p class="setting-hint">
            💡 You can skip this for now and add it later when using Canvas
          </p>
        </div>

        <!-- Theme Section (non-setup only) -->
        <div
          v-if="!isSetup"
          class="setting-section"
        >
          <label class="setting-label">
            <span class="label-icon">🎨</span>
            Theme
          </label>
          <div class="theme-options">
            <label
              class="theme-option"
              :class="{ active: theme === 'system' }"
            >
              <input
                v-model="theme"
                type="radio"
                value="system"
              >
              <span>System</span>
            </label>
            <label
              class="theme-option"
              :class="{ active: theme === 'light' }"
            >
              <input
                v-model="theme"
                type="radio"
                value="light"
              >
              <span>Light</span>
            </label>
            <label
              class="theme-option"
              :class="{ active: theme === 'dark' }"
            >
              <input
                v-model="theme"
                type="radio"
                value="dark"
              >
              <span>Dark</span>
            </label>
          </div>
        </div>

        <!-- MCP Sidecar Section (desktop only, non-setup only) -->
        <div
          v-if="!isSetup && isDesktop"
          class="setting-section"
        >
          <label class="setting-label">
            <span class="label-icon">🔌</span>
            MCP Integration
            <span class="optional-badge">Advanced</span>
          </label>
          <p class="setting-description">
            Enable the MCP sidecar to connect Claude Desktop or ChatGPT to your knowledge base.
          </p>
          <div class="mcp-toggle">
            <label class="mcp-checkbox-label">
              <input
                v-model="mcpEnabled"
                type="checkbox"
              >
              <span>Enable MCP Server</span>
            </label>
            <span
              v-if="mcpStatusText"
              class="mcp-status-badge"
              :class="mcpStatusClass"
            >
              {{ mcpStatusText }}
            </span>
          </div>
          <div
            v-if="mcpEnabled"
            class="mcp-details"
          >
            <div class="mcp-info-row">
              <span class="mcp-info-label">Endpoint:</span>
              <code class="mcp-info-value">{{ mcpUrl || 'http://localhost:8765/sse' }}</code>
            </div>
            <div
              v-if="configSnippet"
              class="config-snippet"
            >
              <div class="snippet-header">
                <span>Claude Desktop Config</span>
                <button
                  class="copy-btn"
                  @click="copyConfigSnippet"
                >
                  {{ copied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
              <pre class="snippet-code">{{ configSnippet }}</pre>
            </div>
            <p class="setting-hint">
              Paste this into Claude Desktop &rarr; Settings &rarr; Developer &rarr; Edit Config
            </p>
          </div>
        </div>
      </div>

      <div class="modal-footer">
        <button
          v-if="!isSetup"
          class="cancel-btn"
          @click="handleClose"
        >
          Cancel
        </button>
        <button
          class="save-btn"
          :disabled="isSetup && !vaultPath || isSaving"
          @click="saveSettings"
        >
          {{ isSaving ? 'Saving...' : isSetup ? 'Complete Setup' : 'Save Settings' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { settings as settingsApi, mcp as mcpApi, isDesktopApp } from '@/api/client'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false,
  },
  isSetup: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['update:modelValue', 'saved', 'setup-complete'])

const isOpen = ref(props.modelValue)
const isLoading = ref(false)
const isSaving = ref(false)
const showApiKey = ref(false)

const isDesktop = isDesktopApp()

// Form state
const vaultPath = ref('')
const openrouterKey = ref('')
const theme = ref('system')
const keyValidationState = ref(null) // 'validating' | 'valid' | 'invalid' | null

// MCP state
const mcpEnabled = ref(false)
const mcpStatus = ref('Stopped') // 'Stopped' | 'Starting' | 'Running' | 'Failed'
const mcpUrl = ref('')
const configSnippet = ref('')
const copied = ref(false)

const mcpStatusClass = computed(() => {
  if (typeof mcpStatus.value === 'object') {
    // SidecarStatus is an enum — Running is { Running: { port, url } }
    if (mcpStatus.value.Running) return 'running'
    if (mcpStatus.value.Failed) return 'failed'
    return 'stopped'
  }
  const s = String(mcpStatus.value).toLowerCase()
  if (s === 'running' || s.startsWith('running')) return 'running'
  if (s === 'starting') return 'starting'
  if (s === 'failed' || s.startsWith('failed')) return 'failed'
  return 'stopped'
})

const mcpStatusText = computed(() => {
  if (typeof mcpStatus.value === 'object') {
    if (mcpStatus.value.Running) return 'Running'
    if (mcpStatus.value.Failed) return 'Failed'
    if (mcpStatus.value === 'Starting') return 'Starting'
    return 'Stopped'
  }
  const s = String(mcpStatus.value)
  if (s === 'Stopped') return mcpEnabled.value ? 'Stopped' : ''
  return s
})

// Sync with v-model
watch(() => props.modelValue, (val) => {
  isOpen.value = val
  if (val) {
    loadCurrentSettings()
  }
})

watch(isOpen, (val) => {
  emit('update:modelValue', val)
})

// Load MCP status and config snippet
const loadMcpStatus = async () => {
  if (!isDesktop) return
  try {
    const status = await mcpApi.getStatus()
    if (status) {
      mcpStatus.value = status.status
      mcpUrl.value = status.mcp_url
    }
    const snippet = await mcpApi.getConfigSnippet()
    if (snippet) {
      configSnippet.value = snippet
    }
  } catch (e) {
    console.error('Failed to load MCP status:', e)
  }
}

// Copy Claude Desktop config snippet to clipboard
const copyConfigSnippet = async () => {
  if (!configSnippet.value) return
  try {
    await navigator.clipboard.writeText(configSnippet.value)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch (e) {
    console.error('Failed to copy:', e)
  }
}

// Load current settings when modal opens
const loadCurrentSettings = async () => {
  if (!isDesktopApp()) return

  try {
    const currentSettings = await settingsApi.get()
    if (currentSettings) {
      vaultPath.value = currentSettings.vault_path || ''
      openrouterKey.value = currentSettings.openrouter_api_key || ''
      theme.value = currentSettings.theme || 'system'
      mcpEnabled.value = currentSettings.mcp_enabled || false
    }

    // Load MCP status if enabled
    if (mcpEnabled.value) {
      await loadMcpStatus()
    }
  } catch (e) {
    console.error('Failed to load settings:', e)
  }
}

// Pick vault folder using native dialog
const pickVaultFolder = async () => {
  if (!isDesktopApp()) {
    alert('Folder picker is only available in the desktop app')
    return
  }

  isLoading.value = true
  try {
    const folder = await settingsApi.pickVaultFolder()
    if (folder) {
      vaultPath.value = folder
    }
  } catch (e) {
    console.error('Failed to pick folder:', e)
  } finally {
    isLoading.value = false
  }
}

// Validate OpenRouter API key
const validateKey = async () => {
  if (!openrouterKey.value || openrouterKey.value.length < 10) {
    keyValidationState.value = null
    return
  }

  keyValidationState.value = 'validating'
  try {
    const isValid = await settingsApi.validateOpenRouterKey(openrouterKey.value)
    keyValidationState.value = isValid ? 'valid' : 'invalid'
  } catch (e) {
    console.error('Failed to validate API key:', e)
    keyValidationState.value = 'invalid'
  }
}

// Save settings
const saveSettings = async () => {
  if (props.isSetup && !vaultPath.value) {
    alert('Please select a vault location')
    return
  }

  isSaving.value = true
  try {
    const update = {
      vault_path: vaultPath.value || null,
      openrouter_api_key: openrouterKey.value || null,
      theme: theme.value,
      mcp_enabled: mcpEnabled.value,
    }

    if (props.isSetup) {
      update.setup_completed = true
    }

    await settingsApi.update(update)

    if (props.isSetup) {
      emit('setup-complete')
    } else {
      emit('saved')
    }

    isOpen.value = false
  } catch (e) {
    console.error('Failed to save settings:', e)
    alert('Failed to save settings: ' + e.message)
  } finally {
    isSaving.value = false
  }
}

// Handle close (only allowed if not in setup mode)
const handleClose = () => {
  if (!props.isSetup) {
    isOpen.value = false
  }
}

// Load settings on mount if modal is already open
onMounted(() => {
  if (isOpen.value) {
    loadCurrentSettings()
  }
})
</script>

<style scoped>
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  backdrop-filter: blur(4px);
}

.settings-modal {
  background: var(--bg-primary, #fff);
  border-radius: 12px;
  width: 90%;
  max-width: 500px;
  max-height: 90vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.3);
}

.modal-header {
  padding: 20px 24px;
  border-bottom: 1px solid var(--border-color, #e0e0e0);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.modal-header h2 {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  cursor: pointer;
  color: var(--text-secondary, #666);
  padding: 0;
  line-height: 1;
}

.close-btn:hover {
  color: var(--text-primary, #1a1a1a);
}

.modal-body {
  padding: 24px;
  overflow-y: auto;
  flex: 1;
}

.setup-intro {
  margin: 0 0 20px;
  color: var(--text-secondary, #666);
  font-size: 0.95rem;
}

.setting-section {
  margin-bottom: 24px;
}

.setting-section:last-child {
  margin-bottom: 0;
}

.setting-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
  margin-bottom: 6px;
}

.label-icon {
  font-size: 1.1rem;
}

.optional-badge {
  font-size: 0.7rem;
  font-weight: 500;
  padding: 2px 6px;
  background: var(--bg-secondary, #f5f5f5);
  border-radius: 4px;
  color: var(--text-secondary, #666);
}

.setting-description {
  font-size: 0.85rem;
  color: var(--text-secondary, #666);
  margin: 0 0 12px;
  line-height: 1.4;
}

.setting-description a {
  color: var(--accent-color, #7c3aed);
}

.vault-picker {
  display: flex;
  gap: 8px;
}

.vault-input {
  flex: 1;
  padding: 10px 12px;
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  font-size: 0.9rem;
  background: var(--bg-secondary, #f9f9f9);
  color: var(--text-primary, #1a1a1a);
}

.browse-btn {
  padding: 10px 16px;
  background: var(--accent-color, #7c3aed);
  color: white;
  border: none;
  border-radius: 8px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
}

.browse-btn:hover:not(:disabled) {
  background: var(--accent-hover, #6d28d9);
}

.browse-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.api-key-input {
  display: flex;
  gap: 8px;
}

.key-input {
  flex: 1;
  padding: 10px 12px;
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  font-size: 0.9rem;
  font-family: monospace;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #1a1a1a);
}

.toggle-visibility {
  padding: 10px 12px;
  background: var(--bg-secondary, #f5f5f5);
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  cursor: pointer;
  font-size: 1rem;
}

.validation-status {
  margin-top: 8px;
  font-size: 0.85rem;
  padding: 6px 10px;
  border-radius: 6px;
}

.validation-status.validating {
  background: var(--bg-secondary, #f5f5f5);
  color: var(--text-secondary, #666);
}

.validation-status.valid {
  background: #d1fae5;
  color: #065f46;
}

.validation-status.invalid {
  background: #fee2e2;
  color: #991b1b;
}

.setting-hint {
  margin-top: 8px;
  font-size: 0.8rem;
  color: var(--text-tertiary, #999);
}

.setting-hint.warning {
  color: #d97706;
}

.theme-options {
  display: flex;
  gap: 8px;
}

.theme-option {
  flex: 1;
  padding: 10px;
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  text-align: center;
  cursor: pointer;
  transition: all 0.2s;
}

.theme-option:hover {
  border-color: var(--accent-color, #7c3aed);
}

.theme-option.active {
  background: var(--accent-color, #7c3aed);
  color: white;
  border-color: var(--accent-color, #7c3aed);
}

.theme-option input {
  display: none;
}

.modal-footer {
  padding: 16px 24px;
  border-top: 1px solid var(--border-color, #e0e0e0);
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}

.cancel-btn {
  padding: 10px 20px;
  background: var(--bg-secondary, #f5f5f5);
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  font-weight: 500;
  cursor: pointer;
  color: var(--text-primary, #1a1a1a);
}

.cancel-btn:hover {
  background: var(--bg-tertiary, #eee);
}

.save-btn {
  padding: 10px 24px;
  background: var(--accent-color, #7c3aed);
  color: white;
  border: none;
  border-radius: 8px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
}

.save-btn:hover:not(:disabled) {
  background: var(--accent-hover, #6d28d9);
}

.save-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* MCP Section */
.mcp-toggle {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
}

.mcp-checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 0.9rem;
  color: var(--text-primary, #1a1a1a);
}

.mcp-checkbox-label input[type="checkbox"] {
  width: 16px;
  height: 16px;
  accent-color: var(--accent-color, #7c3aed);
}

.mcp-status-badge {
  font-size: 0.75rem;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 10px;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.mcp-status-badge.running {
  background: #d1fae5;
  color: #065f46;
}

.mcp-status-badge.starting {
  background: #fef3c7;
  color: #92400e;
}

.mcp-status-badge.stopped {
  background: var(--bg-secondary, #f5f5f5);
  color: var(--text-secondary, #666);
}

.mcp-status-badge.failed {
  background: #fee2e2;
  color: #991b1b;
}

.mcp-details {
  padding: 12px;
  background: var(--bg-secondary, #f9f9f9);
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
}

.mcp-info-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  font-size: 0.85rem;
}

.mcp-info-label {
  font-weight: 600;
  color: var(--text-secondary, #666);
}

.mcp-info-value {
  font-family: monospace;
  font-size: 0.8rem;
  padding: 2px 6px;
  background: var(--bg-primary, #fff);
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 4px;
  color: var(--text-primary, #1a1a1a);
}

.config-snippet {
  margin-top: 8px;
}

.snippet-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 6px;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary, #666);
}

.copy-btn {
  padding: 3px 10px;
  font-size: 0.75rem;
  background: var(--accent-color, #7c3aed);
  color: white;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.2s;
}

.copy-btn:hover {
  background: var(--accent-hover, #6d28d9);
}

.snippet-code {
  margin: 0;
  padding: 10px;
  background: var(--bg-primary, #fff);
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 6px;
  font-family: monospace;
  font-size: 0.78rem;
  line-height: 1.5;
  overflow-x: auto;
  white-space: pre;
  color: var(--text-primary, #1a1a1a);
}

/* Dark mode support */
:root.dark .settings-modal {
  --bg-primary: #1a1a2e;
  --bg-secondary: #16213e;
  --text-primary: #eee;
  --text-secondary: #aaa;
  --border-color: #333;
}
</style>
