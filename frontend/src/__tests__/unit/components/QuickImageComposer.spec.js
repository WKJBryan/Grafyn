import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import QuickImageComposer from '@/components/companion/QuickImageComposer.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import { resetTransport, setRuntimeProfile, setRuntimeStatus } from '@/api/transport'
import { createRuntimeProfile } from '@/platform/runtime'
import { CAPABILITY_NAMES, normalizeRuntimeStatus } from '@/platform/capabilities'

const api = vi.hoisted(() => ({
  discoverModels: vi.fn(),
  getModelCapability: vi.fn(),
  generate: vi.fn(),
  save: vi.fn(),
  saveAs: vi.fn(),
  shareGeneratedImage: vi.fn(),
  discard: vi.fn(),
  getSyncStatus: vi.fn(),
}))

vi.mock('@/api/client', () => ({
  images: {
    discoverModels: api.discoverModels,
    getModelCapability: api.getModelCapability,
    generate: api.generate,
    save: api.save,
    saveAs: api.saveAs,
    shareGeneratedImage: api.shareGeneratedImage,
    discard: api.discard,
  },
  sync: { getStatus: api.getSyncStatus },
}))

const MODELS = [{
  modelId: 'openai/gpt-image-1',
  name: 'GPT Image 1',
  description: 'OpenRouter image model',
  resolutions: ['1024x1024', '2048x2048'],
  aspectRatios: ['1:1', '16:9'],
  endpointPath: '/images/generations',
}]

const CAPABILITY = {
  modelId: 'openai/gpt-image-1',
  endpoints: [{
    endpointId: 'openrouter-primary',
    provider: 'openrouter',
    resolutions: ['1024x1024', '2048x2048'],
    aspectRatios: ['1:1', '16:9'],
    outputFormats: ['image/png'],
    publishedPrice: null,
  }],
  pricingIsFinal: false,
  promptLeavesDevice: true,
}

const PREVIEW = {
  receiptId: 'receipt-1',
  mediaType: 'image/png',
  base64Data: 'AQID',
  byteSize: 3,
  width: 1024,
  height: 1024,
  modelId: 'openai/gpt-image-1',
  resolution: '1024x1024',
  aspectRatio: '1:1',
  cost: { status: 'exact_usd', usd: '0.004200' },
  promptLeavesDevice: true,
}

const GOVERNED_SAVE = {
  note: { id: 'note-image-1' },
  observationEventId: 'event-1',
  attachmentDigest: 'a'.repeat(64),
  mediaType: 'image/png',
  byteSize: 3,
  width: 1024,
  height: 1024,
  syncDisposition: {
    status: 'queued',
    manifestCount: 1,
    chunkCount: 2,
    operationCount: 3,
  },
}

function useDesktopRuntime() {
  setRuntimeProfile(createRuntimeProfile({ isTauri: true, platform: 'windows' }))
  setRuntimeStatus(normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'desktop',
    capabilities: Object.fromEntries(CAPABILITY_NAMES.map(name => [name, true])),
    vault: { kind: 'user_selected', available: true },
    secureSecrets: { status: 'ready', code: null, message: null },
    nativeImageShare: {
      status: 'unavailable',
      code: 'desktop_save_as',
      message: 'Desktop uses Save As.',
    },
    diagnostics: [],
  }))
}

function revokeDesktopRuntime() {
  setRuntimeStatus(normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'desktop',
    capabilities: Object.fromEntries(CAPABILITY_NAMES.map(name => [name, false])),
    vault: { kind: 'user_selected', available: false },
    secureSecrets: {
      status: 'unavailable',
      code: 'boot_failed',
      message: 'Runtime startup failed.',
    },
    nativeImageShare: {
      status: 'unavailable',
      code: 'boot_failed',
      message: 'Runtime startup failed.',
    },
    diagnostics: [],
  }))
}

function useAndroidRuntime({ nativeShare = true } = {}) {
  setRuntimeProfile(createRuntimeProfile({ isTauri: true, platform: 'android' }))
  const capabilities = Object.fromEntries(CAPABILITY_NAMES.map(name => [name, false]))
  Object.assign(capabilities, {
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview: true,
    twinChat: true,
    linearCanvas: true,
    imageGeneration: true,
    nativeImageShare: nativeShare,
  })
  setRuntimeStatus(normalizeRuntimeStatus({
    schemaVersion: 1,
    runtime: 'android',
    capabilities,
    vault: { kind: 'app_private', available: true },
    secureSecrets: { status: 'ready', code: null, message: null },
    nativeImageShare: nativeShare
      ? { status: 'ready', code: null, message: null }
      : {
        status: 'unavailable',
        code: 'share_unavailable',
        message: 'Native image sharing is unavailable.',
      },
    diagnostics: [],
  }))
}

function deferred() {
  let resolve
  let reject
  const promise = new Promise((onResolve, onReject) => {
    resolve = onResolve
    reject = onReject
  })
  return { promise, resolve, reject }
}

async function mountComposer(props = {}) {
  const wrapper = mount(QuickImageComposer, { props })
  await flushPromises()
  return wrapper
}

async function chooseRequest(wrapper) {
  await wrapper.get('[aria-label="Image model"]').setValue('openai/gpt-image-1')
  await flushPromises()
  await wrapper.get('[aria-label="Image resolution"]').setValue('1024x1024')
  await wrapper.get('[aria-label="Image aspect ratio"]').setValue('1:1')
  await wrapper.get('[aria-label="Image prompt"]').setValue('A careful systems sketch')
}

async function generatePreview(wrapper, preview = PREVIEW) {
  api.generate.mockResolvedValueOnce(preview)
  await chooseRequest(wrapper)
  await wrapper.get('.quick-image-form').trigger('submit')
  await flushPromises()
}

describe('QuickImageComposer', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    useDesktopRuntime()
    api.discoverModels.mockResolvedValue(MODELS)
    api.getModelCapability.mockResolvedValue(CAPABILITY)
    api.discard.mockResolvedValue()
    api.getSyncStatus.mockResolvedValue({
      status: 'pending',
      provisioned: true,
      outboxOperations: 3,
      pendingOperations: 0,
      conflicts: 0,
      error: null,
    })
    vi.stubGlobal('URL', {
      createObjectURL: vi.fn(() => 'blob:grafyn-image-preview'),
      revokeObjectURL: vi.fn(),
    })
  })

  afterEach(() => {
    resetTransport()
    vi.unstubAllGlobals()
  })

  it('discovers live models and requires explicit model, resolution, and aspect choices', async () => {
    const wrapper = await mountComposer()

    expect(api.discoverModels).toHaveBeenCalledOnce()
    expect(wrapper.get('[aria-label="Image model"]').text()).toContain('GPT Image 1')
    expect(wrapper.get('[aria-label="Image model"]').element.value).toBe('')
    expect(wrapper.get('[aria-label="Generate image"]').attributes('disabled')).toBeDefined()

    await wrapper.get('[aria-label="Image model"]').setValue('openai/gpt-image-1')
    await flushPromises()

    expect(api.getModelCapability).toHaveBeenCalledWith('openai/gpt-image-1')
    expect(wrapper.get('[aria-label="Image resolution"]').text()).toContain('1024x1024')
    expect(wrapper.get('[aria-label="Image aspect ratio"]').text()).toContain('1:1')
    expect(wrapper.get('[aria-label="Image resolution"]').element.value).toBe('')
    expect(wrapper.get('[aria-label="Image aspect ratio"]').element.value).toBe('')
    expect(wrapper.text()).toContain('Prompt is sent to OpenRouter')
  })

  it('removes mounted generation and blocks a submit as runtime capability is revoked', async () => {
    const wrapper = await mountComposer()
    await chooseRequest(wrapper)
    const form = wrapper.get('.quick-image-form')

    revokeDesktopRuntime()
    await form.trigger('submit')
    await flushPromises()

    expect(api.generate).not.toHaveBeenCalled()
    expect(wrapper.find('.quick-image-form').exists()).toBe(false)
    expect(wrapper.get('[role="status"]').text()).toContain('unavailable')
  })

  it('offers only resolution and square-aspect pairs supported by the same endpoint', async () => {
    api.getModelCapability.mockResolvedValueOnce({
      ...CAPABILITY,
      endpoints: [
        {
          ...CAPABILITY.endpoints[0],
          endpointId: 'wide-only',
          resolutions: ['1024x1024'],
          aspectRatios: ['16:9'],
        },
        {
          ...CAPABILITY.endpoints[0],
          endpointId: 'square-only',
          resolutions: ['2048x2048'],
          aspectRatios: ['1:1'],
        },
      ],
    })
    const wrapper = await mountComposer()
    await wrapper.get('[aria-label="Image model"]').setValue('openai/gpt-image-1')
    await flushPromises()

    const resolutionSelect = wrapper.get('[aria-label="Image resolution"]')
    expect(resolutionSelect.text()).not.toContain('1024x1024')
    expect(resolutionSelect.text()).toContain('2048x2048')
    expect(wrapper.get('[aria-label="Image aspect ratio"]').text()).not.toContain('16:9')
    expect(wrapper.get('[aria-label="Image aspect ratio"]').text()).toContain('1:1')
  })

  it('renders an honest loading state while model discovery is pending', async () => {
    const pending = deferred()
    api.discoverModels.mockReturnValueOnce(pending.promise)
    const wrapper = mount(QuickImageComposer)
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-test="image-discovery-status"]').text())
      .toContain('Loading image models')
    expect(wrapper.get('[aria-label="Generate image"]').attributes('disabled')).toBeDefined()

    pending.resolve(MODELS)
    await flushPromises()
    expect(wrapper.find('[data-test="image-discovery-status"]').exists()).toBe(false)
  })

  it('discovers models exactly once only after a compact composer is expanded', async () => {
    const wrapper = await mountComposer({ collapsible: true })

    expect(api.discoverModels).not.toHaveBeenCalled()
    await wrapper.get('.image-toggle').trigger('click')
    await flushPromises()
    expect(api.discoverModels).toHaveBeenCalledOnce()

    await wrapper.get('.image-toggle').trigger('click')
    await wrapper.get('.image-toggle').trigger('click')
    await flushPromises()
    expect(api.discoverModels).toHaveBeenCalledOnce()
  })

  it('does not commit a pending lazy discovery after the compact composer unmounts', async () => {
    const pending = deferred()
    api.discoverModels.mockReturnValueOnce(pending.promise)
    const wrapper = await mountComposer({ collapsible: true })
    await wrapper.get('.image-toggle').trigger('click')
    await wrapper.vm.$nextTick()
    wrapper.unmount()

    pending.resolve(MODELS)
    await flushPromises()

    expect(api.discoverModels).toHaveBeenCalledOnce()
    expect(api.getModelCapability).not.toHaveBeenCalled()
  })

  it('submits once, renders a one-shot Blob preview, and preserves it across safe rerenders', async () => {
    const pending = deferred()
    api.generate.mockReturnValue(pending.promise)
    const wrapper = await mountComposer()
    await chooseRequest(wrapper)

    await wrapper.get('.quick-image-form').trigger('submit')
    await wrapper.get('.quick-image-form').trigger('submit')
    expect(api.generate).toHaveBeenCalledOnce()
    expect(api.generate).toHaveBeenCalledWith({
      prompt: 'A careful systems sketch',
      modelId: 'openai/gpt-image-1',
      resolution: '1024x1024',
      aspectRatio: '1:1',
    })
    expect(wrapper.get('[aria-label="Generate image"]').attributes('disabled')).toBeDefined()

    pending.resolve(PREVIEW)
    await flushPromises()

    expect(wrapper.get('img[alt="Generated preview"]').attributes('src'))
      .toBe('blob:grafyn-image-preview')
    expect(wrapper.get('[data-test="image-cost"]').text()).toBe('US$0.004200 exact')
    expect(wrapper.get('[aria-label="Generate image"]').attributes('disabled')).toBeDefined()
    expect(api.save).not.toHaveBeenCalled()

    await wrapper.setProps({ collapsible: true })
    expect(wrapper.get('img[alt="Generated preview"]').attributes('src'))
      .toBe('blob:grafyn-image-preview')
    expect(wrapper.get('[aria-label="Image prompt"]').element.value)
      .toBe('A careful systems sketch')
  })

  it('preserves the exact visible multiline prompt in the immutable generation receipt', async () => {
    const exactPrompt = '\n  First line\n\tSecond line  \n'
    api.generate.mockResolvedValueOnce(PREVIEW)
    const wrapper = await mountComposer()
    await wrapper.get('[aria-label="Image model"]').setValue('openai/gpt-image-1')
    await flushPromises()
    await wrapper.get('[aria-label="Image resolution"]').setValue('1024x1024')
    await wrapper.get('[aria-label="Image aspect ratio"]').setValue('1:1')
    await wrapper.get('[aria-label="Image prompt"]').setValue(exactPrompt)
    await wrapper.get('.quick-image-form').trigger('submit')
    await flushPromises()

    expect(api.generate).toHaveBeenCalledWith(expect.objectContaining({ prompt: exactPrompt }))
    expect(wrapper.get('[aria-label="Image prompt"]').element.value).toBe(exactPrompt)
  })

  it('labels unavailable cost without inventing an estimate', async () => {
    const wrapper = await mountComposer()
    await generatePreview(wrapper, {
      ...PREVIEW,
      cost: { status: 'unavailable' },
    })

    expect(wrapper.get('[data-test="image-cost"]').text()).toBe('Cost unavailable')
  })

  it('freezes the evidence prompt while a receipt exists and unlocks it after discard', async () => {
    const wrapper = await mountComposer()
    await generatePreview(wrapper)

    expect(wrapper.get('[aria-label="Image prompt"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="Image annotation"]').attributes('disabled')).toBeUndefined()

    await wrapper.get('[aria-label="Discard generated preview"]').trigger('click')
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[aria-label="Image prompt"]').attributes('disabled')).toBeUndefined()
  })

  it.each([
    ['Failed to fetch', 'NETWORK'],
    ['Failed to send OpenRouter image request', 'NETWORK'],
    ['Image capability query timed out', 'NETWORK'],
    ['OpenRouter API key is not configured', 'UNCONFIGURED'],
    ['Provider rejected the request', 'PROVIDER_ERROR'],
  ])('renders %s as a typed, actionable error', async (message, code) => {
    api.generate.mockRejectedValueOnce(new Error(message))
    const wrapper = await mountComposer()
    await chooseRequest(wrapper)
    await wrapper.get('.quick-image-form').trigger('submit')
    await flushPromises()

    const alert = wrapper.get('[role="alert"]')
    expect(alert.attributes('data-error-code')).toBe(code)
    expect(alert.text()).toBeTruthy()
    expect(wrapper.get('[aria-label="Image prompt"]').element.value)
      .toBe('A careful systems sketch')
  })

  it('warns before discarding a generated receipt and revokes its Blob URL only on confirm', async () => {
    const wrapper = await mountComposer()
    await generatePreview(wrapper)

    await wrapper.get('[aria-label="Discard generated preview"]').trigger('click')
    expect(wrapper.getComponent(ConfirmDialog).props('visible')).toBe(true)
    wrapper.getComponent(ConfirmDialog).vm.$emit('cancel')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('img[alt="Generated preview"]').exists()).toBe(true)
    expect(URL.revokeObjectURL).not.toHaveBeenCalled()

    await wrapper.get('[aria-label="Discard generated preview"]').trigger('click')
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await flushPromises()
    expect(wrapper.find('img[alt="Generated preview"]').exists()).toBe(false)
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:grafyn-image-preview')
    expect(api.discard).toHaveBeenCalledWith('receipt-1')
  })

  it('clears local preview state even when best-effort backend discard fails', async () => {
    api.discard.mockRejectedValueOnce(new Error('Discard IPC unavailable'))
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Discard generated preview"]').trigger('click')
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await flushPromises()

    expect(api.discard).toHaveBeenCalledWith('receipt-1')
    expect(wrapper.find('img[alt="Generated preview"]').exists()).toBe(false)
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  })

  it('revokes an outstanding preview URL when its owner unmounts', async () => {
    const wrapper = await mountComposer()
    await generatePreview(wrapper)

    wrapper.unmount()

    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:grafyn-image-preview')
    expect(api.discard).toHaveBeenCalledWith('receipt-1')
  })

  it('discards a late generated receipt that resolves after its owner unmounts', async () => {
    const pending = deferred()
    api.generate.mockReturnValueOnce(pending.promise)
    const wrapper = await mountComposer()
    await chooseRequest(wrapper)
    await wrapper.get('.quick-image-form').trigger('submit')
    wrapper.unmount()

    pending.resolve(PREVIEW)
    await flushPromises()

    expect(api.discard).toHaveBeenCalledWith('receipt-1')
    expect(URL.createObjectURL).not.toHaveBeenCalled()
  })

  it('performs an explicit governed save and reports the resulting sync receipt status', async () => {
    api.save.mockResolvedValueOnce(GOVERNED_SAVE)
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Image annotation"]').setValue('Systems sketch')
    await wrapper.get('[aria-label="Metadata retention"]').setValue('retain_original')
    await wrapper.get('[aria-label="Image sync policy"]').setValue('inherit')
    await wrapper.get('[aria-label="Save image to Grafyn"]').trigger('click')
    await flushPromises()

    expect(api.save).toHaveBeenCalledWith({
      receiptId: 'receipt-1',
      annotation: 'Systems sketch',
      retentionPolicy: 'retain_original',
      grafynSync: 'inherit',
    })
    expect(api.getSyncStatus).not.toHaveBeenCalled()
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('Saved to Grafyn')
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('3 attachment operations')
    expect(wrapper.find('[aria-label="Save image to Grafyn"]').exists()).toBe(false)
    expect(wrapper.emitted('saved')).toEqual([[GOVERNED_SAVE]])
  })

  it('does not emit a governed save for failed or discarded image actions', async () => {
    api.save.mockRejectedValueOnce(new Error('Provider save failed'))
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Save image to Grafyn"]').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('saved')).toBeUndefined()

    await wrapper.get('[aria-label="Discard generated preview"]').trigger('click')
    wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
    await flushPromises()

    expect(wrapper.emitted('saved')).toBeUndefined()
  })

  it('reports a local-only save without borrowing the vault global sync status', async () => {
    api.save.mockResolvedValueOnce({
      attachmentDigest: 'c'.repeat(64),
      syncDisposition: {
        status: 'local_only',
        manifestCount: 0,
        chunkCount: 0,
        operationCount: 0,
      },
    })
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Save image to Grafyn"]').trigger('click')
    await flushPromises()

    expect(api.save).toHaveBeenCalledWith(expect.objectContaining({
      grafynSync: 'local_only',
    }))
    expect(api.getSyncStatus).not.toHaveBeenCalled()
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('Saved locally to Grafyn')
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('not queued for sync')
  })

  it('reports attachment-specific awaiting-provisioning status and consumes the receipt', async () => {
    api.save.mockResolvedValueOnce({
      attachmentDigest: 'b'.repeat(64),
      syncDisposition: {
        status: 'awaiting_provisioning',
        manifestCount: 0,
        chunkCount: 0,
        operationCount: 0,
      },
    })
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Image sync policy"]').setValue('inherit')
    await wrapper.get('[aria-label="Save image to Grafyn"]').trigger('click')
    await flushPromises()

    expect(api.save).toHaveBeenCalledOnce()
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('Saved to Grafyn')
    expect(wrapper.get('[data-test="image-save-status"]').text())
      .toContain('awaiting sync provisioning')
    expect(api.getSyncStatus).not.toHaveBeenCalled()
    expect(wrapper.find('[aria-label="Save image to Grafyn"]').exists()).toBe(false)
  })

  it.each([
    ['Generated image save committed before a later failure; do not retry automatically'],
    ['mutation authority advanced and durable recovery remains pending: digest'],
  ])('locks a terminal receipt after backend error: %s', async message => {
    api.save.mockRejectedValueOnce(new Error(message))
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Save image to Grafyn"]').trigger('click')
    await flushPromises()

    expect(wrapper.get('[role="alert"]').attributes('data-error-code')).toBe('COMMIT_UNCERTAIN')
    expect(wrapper.find('[aria-label="Save image to Grafyn"]').exists()).toBe(false)
  })

  it('keeps desktop Save As distinct from governed vault saving and never accepts a path', async () => {
    api.saveAs.mockResolvedValueOnce({ exported: false })
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Metadata retention"]').setValue('retain_original')
    await wrapper.get('[aria-label="Save generated image as a desktop file"]').trigger('click')
    await flushPromises()

    expect(api.saveAs).toHaveBeenCalledWith('receipt-1', 'retain_original')
    expect(api.save).not.toHaveBeenCalled()
    expect(api.saveAs.mock.calls[0]).toEqual(['receipt-1', 'retain_original'])
    expect(wrapper.get('[data-test="image-export-status"]').text()).toContain('canceled')
    expect(wrapper.find('img[alt="Generated preview"]').exists()).toBe(true)
  })

  it.each([
    ['generated image receipt is missing or expired', false],
    ['Desktop dialog could not open', true],
  ])('handles Save As failure %s with retryable=%s', async (message, retryable) => {
    api.saveAs.mockRejectedValueOnce(new Error(message))
    const wrapper = await mountComposer()
    await generatePreview(wrapper)
    await wrapper.get('[aria-label="Save generated image as a desktop file"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[aria-label="Save generated image as a desktop file"]').exists())
      .toBe(retryable)
    expect(wrapper.find('[aria-label="Save image to Grafyn"]').exists()).toBe(retryable)
  })

  it('does not render native Share when Android capability health is unavailable', async () => {
    useAndroidRuntime({ nativeShare: false })
    const wrapper = await mountComposer()
    await generatePreview(wrapper)

    expect(wrapper.find('[aria-label="Share generated image"]').exists()).toBe(false)
    expect(wrapper.find('[aria-label="Share generated image unavailable"]').exists()).toBe(false)
    expect(wrapper.find('[aria-label="Save image to Grafyn"]').exists()).toBe(true)
    expect(api.saveAs).not.toHaveBeenCalled()
    expect(api.shareGeneratedImage).not.toHaveBeenCalled()
  })

  it('does not advertise native Share without an Android receipt producer', async () => {
    useAndroidRuntime()
    const wrapper = await mountComposer()
    await generatePreview(wrapper)

    expect(wrapper.find('[aria-label="Share generated image"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="image-share-status"]').exists()).toBe(false)
    expect(api.shareGeneratedImage).not.toHaveBeenCalled()
  })
})
