import { createPinia } from 'pinia'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import CompanionSettingsSheet from '@/components/companion/CompanionSettingsSheet.vue'
import {
  resetTransport,
  setRuntimeProfile,
  setRuntimeStatus,
} from '@/api/transport'

const api = vi.hoisted(() => ({
  settingsGet: vi.fn(),
  settingsUpdate: vi.fn(),
  openRouterStatus: vi.fn(),
  pickVaultFolder: vi.fn(),
  ollamaStatus: vi.fn(),
  syncStatus: vi.fn(),
}))

vi.mock('@/api/client', () => ({
  settings: {
    get: api.settingsGet,
    update: api.settingsUpdate,
    getOpenRouterStatus: api.openRouterStatus,
    pickVaultFolder: api.pickVaultFolder,
    getOllamaStatus: api.ollamaStatus,
  },
  sync: { getStatus: api.syncStatus },
}))

function runtimeStatus({ secrets = 'ready', share = 'ready', sync = true } = {}) {
  return {
    schemaVersion: 1,
    runtime: 'android',
    capabilities: {
      notesRead: true,
      notesWrite: true,
      recall: true,
      twinReview: true,
      twinChat: true,
      linearCanvas: true,
      imageGeneration: secrets === 'ready',
      nativeImageShare: share === 'ready',
      sync,
      spatialCanvas: false,
      nativeVaultPicker: false,
      importByPath: false,
      localOllama: false,
      mcp: false,
      vaultMigration: false,
      optimizerAdmin: false,
      desktopUpdater: false,
    },
    vault: { kind: 'app_private', available: true },
    secureSecrets: {
      status: secrets,
      code: secrets === 'ready' ? null : 'keystore_unavailable',
      message: secrets === 'ready' ? null : 'C:\\private\\secret-store failed',
    },
    nativeImageShare: {
      status: share,
      code: share === 'ready' ? null : 'share_unavailable',
      message: null,
    },
    diagnostics: [{ code: 'offline_ready', message: 'Private path: C:\\Users\\Bryan' }],
  }
}

function mountSheet() {
  mountedSheet = mount(CompanionSettingsSheet, {
    attachTo: document.body,
    global: { plugins: [createPinia()] },
  })
  return mountedSheet
}

let mountedSheet = null

describe('CompanionSettingsSheet', () => {
  beforeEach(() => {
    resetTransport()
    setRuntimeProfile({ name: 'android-compact', isTauri: true, platform: 'android' })
    setRuntimeStatus(runtimeStatus())
    api.settingsGet.mockResolvedValue({
      theme: 'system',
      vault_path: 'C:\\Users\\Bryan\\Documents\\Grafyn',
    })
    api.settingsUpdate.mockResolvedValue({})
    api.openRouterStatus.mockResolvedValue({ has_key: true, is_configured: true })
    api.syncStatus.mockResolvedValue({ status: 'local_only', pendingOperations: 0 })
  })

  afterEach(() => {
    mountedSheet?.unmount()
    mountedSheet = null
    document.body.innerHTML = ''
    resetTransport()
  })

  it('shows only compact-safe settings and never exposes the local vault path', async () => {
    const wrapper = mountSheet()
    await flushPromises()

    expect(wrapper.text()).toContain('Private on-device vault')
    expect(wrapper.text()).toContain('OpenRouter')
    expect(wrapper.text()).toContain('Local only')
    expect(wrapper.text()).toContain('offline_ready')
    expect(wrapper.text()).not.toContain('C:\\Users')
    expect(wrapper.text()).not.toMatch(/Browse|Ollama|MCP|Migration|Optimizer|Updater/)
    expect(api.pickVaultFolder).not.toHaveBeenCalled()
    expect(api.ollamaStatus).not.toHaveBeenCalled()
    expect(api.openRouterStatus).toHaveBeenCalledOnce()
    expect(api.syncStatus).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('keeps secret-dependent controls disabled and does not invoke them without secure health', async () => {
    setRuntimeStatus(runtimeStatus({ secrets: 'unavailable', sync: true }))
    const wrapper = mountSheet()
    await flushPromises()

    expect(wrapper.get('[data-testid="openrouter-key"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="save-openrouter-key"]').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Secure storage is unavailable')
    expect(wrapper.text()).toContain('Sync is unavailable')
    expect(wrapper.text()).not.toContain('secret-store failed')
    expect(api.openRouterStatus).not.toHaveBeenCalled()
    expect(api.syncStatus).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('revokes mounted controls when authoritative runtime health becomes unavailable', async () => {
    const wrapper = mountSheet()
    await flushPromises()
    expect(wrapper.get('[data-testid="openrouter-key"]').attributes('disabled')).toBeUndefined()

    setRuntimeStatus(revokedRuntimeStatus())
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="openrouter-key"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="save-openrouter-key"]').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Secure storage is unavailable')
    expect(wrapper.text()).toContain('Sync is unavailable')
    expect(wrapper.text()).toContain('canonical_runtime_unavailable')
    wrapper.unmount()
  })

  it('writes a replacement OpenRouter secret only through the secure settings command', async () => {
    api.openRouterStatus
      .mockResolvedValueOnce({ has_key: false, is_configured: false })
      .mockResolvedValueOnce({ has_key: true, is_configured: true })
    const wrapper = mountSheet()
    await flushPromises()

    const input = wrapper.get('[data-testid="openrouter-key"]')
    expect(input.attributes('type')).toBe('password')
    await input.setValue('sk-or-v1-new-secret')
    await wrapper.get('[data-testid="save-openrouter-key"]').trigger('click')
    await flushPromises()

    expect(api.settingsUpdate).toHaveBeenCalledWith({
      openrouter_api_key: 'sk-or-v1-new-secret',
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="openrouter-key"]').element.value).toBe('')
    expect(wrapper.text()).toContain('Stored securely')
    expect(wrapper.text()).not.toContain('sk-or-v1-new-secret')
    wrapper.unmount()
  })

  it('saves theme preference and applies the resolved on-device theme', async () => {
    const wrapper = mountSheet()
    await flushPromises()

    await wrapper.get('input[value="dark"]').setValue(true)
    await flushPromises()

    expect(api.settingsUpdate).toHaveBeenCalledWith({ theme: 'dark' })
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    wrapper.unmount()
  })

  it('is modal, traps focus, closes on Escape, and restores the opener', async () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    const wrapper = mountSheet()
    await flushPromises()

    const dialog = wrapper.get('[role="dialog"]')
    const close = wrapper.get('[aria-label="Close companion settings"]')
    const last = wrapper.get('[data-testid="openrouter-key"]')
    expect(dialog.attributes('aria-modal')).toBe('true')
    await vi.waitFor(() => expect(document.activeElement).toBe(dialog.element))

    last.element.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))
    expect(document.activeElement).toBe(close.element)

    close.element.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab',
      shiftKey: true,
      bubbles: true,
    }))
    expect(document.activeElement).toBe(last.element)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
    expect(document.activeElement).toBe(opener)
  })
})

function revokedRuntimeStatus() {
  const current = runtimeStatus()
  return {
    ...current,
    capabilities: Object.fromEntries(Object.keys(current.capabilities).map(name => [name, false])),
    vault: { ...current.vault, available: false },
    secureSecrets: {
      status: 'unavailable',
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    },
    nativeImageShare: {
      status: 'unavailable',
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    },
    diagnostics: [{
      code: 'canonical_runtime_unavailable',
      message: 'The canonical local runtime is unavailable.',
    }],
  }
}
