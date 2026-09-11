<template>
  <section class="quick-image-composer">
    <button
      v-if="collapsible"
      class="image-toggle"
      type="button"
      :aria-expanded="expanded"
      aria-controls="quick-image-panel"
      @click="expanded = !expanded"
    >
      <span>
        <span class="eyebrow">Visual draft</span>
        <strong>Generate image</strong>
      </span>
      <span aria-hidden="true">{{ expanded ? '−' : '+' }}</span>
    </button>

    <div
      v-show="expanded"
      id="quick-image-panel"
      class="image-panel"
    >
      <div
        v-if="!imageGenerationAvailable"
        class="unavailable-state"
        role="status"
      >
        <p>Image generation is unavailable in this runtime.</p>
      </div>

      <template v-else>
        <p
          v-if="discoveryLoading"
          class="provider-disclosure"
          data-test="image-discovery-status"
          role="status"
        >
          Loading image models…
        </p>
        <form
          class="quick-image-form"
          @submit.prevent="generateImage"
        >
          <div class="section-heading">
            <div>
              <span class="eyebrow">One-shot preview</span>
              <h2>Shape an image</h2>
            </div>
            <span class="privacy-mark">Local receipt</span>
          </div>

          <p
            v-if="error"
            class="image-error"
            role="alert"
            :data-error-code="error.code"
          >
            {{ error.message }}
          </p>

          <label class="field full-field">
            <span>Prompt</span>
            <textarea
              v-model="prompt"
              aria-label="Image prompt"
              placeholder="Describe the image you want to explore…"
              rows="4"
              :disabled="generating || Boolean(preview)"
            />
          </label>

          <div class="option-grid">
            <label class="field">
              <span>Model</span>
              <select
                v-model="modelId"
                aria-label="Image model"
                :disabled="discoveryLoading || generating || Boolean(preview)"
              >
                <option value="">Choose model</option>
                <option
                  v-for="model in models"
                  :key="model.modelId"
                  :value="model.modelId"
                >
                  {{ model.name || model.modelId }}
                </option>
              </select>
            </label>

            <label class="field">
              <span>Resolution</span>
              <select
                v-model="resolution"
                aria-label="Image resolution"
                :disabled="capabilityLoading || generating || Boolean(preview) || !modelCapability"
              >
                <option value="">Choose size</option>
                <option
                  v-for="option in resolutionOptions"
                  :key="option"
                  :value="option"
                >
                  {{ option }}
                </option>
              </select>
            </label>

            <label class="field">
              <span>Aspect</span>
              <select
                v-model="aspectRatio"
                aria-label="Image aspect ratio"
                :disabled="capabilityLoading || generating || Boolean(preview) || !modelCapability"
              >
                <option value="">Choose ratio</option>
                <option
                  v-for="option in aspectRatioOptions"
                  :key="option"
                  :value="option"
                >
                  {{ option }}
                </option>
              </select>
            </label>
          </div>

          <p
            v-if="modelCapability?.promptLeavesDevice"
            class="provider-disclosure"
          >
            Prompt is sent to OpenRouter for generation.
          </p>

          <button
            class="primary-action"
            type="submit"
            aria-label="Generate image"
            :disabled="!canGenerate"
          >
            {{ generating ? 'Generating…' : preview ? 'Preview ready' : 'Generate image' }}
          </button>
        </form>

        <div
          v-if="preview"
          class="preview-card"
        >
          <img
            :src="previewUrl"
            alt="Generated preview"
          >
          <div class="preview-meta">
            <span>{{ preview.width }} × {{ preview.height }}</span>
            <strong data-test="image-cost">{{ formattedCost }}</strong>
          </div>

          <div
            v-if="!receiptConsumed"
            class="save-controls"
          >
            <label class="field full-field">
              <span>Annotation</span>
              <input
                v-model="annotation"
                aria-label="Image annotation"
                maxlength="2048"
                placeholder="Why this image matters (optional)"
                :disabled="saving || exporting || sharing"
              >
            </label>
            <div class="option-grid save-options">
              <label class="field">
                <span>Metadata</span>
                <select
                  v-model="retentionPolicy"
                  aria-label="Metadata retention"
                  :disabled="saving || exporting || sharing"
                >
                  <option value="strip_metadata">Strip metadata</option>
                  <option value="retain_original">Retain original</option>
                </select>
              </label>
              <label class="field">
                <span>Sync</span>
                <select
                  v-model="syncPolicy"
                  aria-label="Image sync policy"
                  :disabled="saving || exporting || sharing"
                >
                  <option value="local_only">Local only</option>
                  <option value="inherit">Use vault sync policy</option>
                </select>
              </label>
            </div>
          </div>

          <div class="receipt-actions">
            <button
              v-if="!receiptConsumed"
              class="primary-action"
              type="button"
              aria-label="Save image to Grafyn"
              :disabled="saving || exporting || sharing"
              @click="saveToGrafyn"
            >
              {{ saving ? 'Saving…' : 'Save to Grafyn' }}
            </button>
            <button
              v-if="isDesktop && !receiptConsumed"
              class="secondary-action"
              type="button"
              aria-label="Save generated image as a desktop file"
              :disabled="saving || exporting || sharing"
              @click="saveAsDesktopFile"
            >
              {{ exporting ? 'Opening…' : 'Save As…' }}
            </button>
            <button
              v-if="isAndroid && nativeImageShareAvailable && !receiptConsumed"
              class="secondary-action"
              type="button"
              aria-label="Share generated image"
              :disabled="saving || exporting || sharing"
              @click="shareAndroidImage"
            >
              {{ sharing ? 'Sharing…' : 'Share' }}
            </button>
            <button
              class="quiet-action"
              type="button"
              aria-label="Discard generated preview"
              :disabled="saving || exporting || sharing"
              @click="showDiscardConfirm = true"
            >
              Discard preview
            </button>
          </div>

          <p
            v-if="saveStatus"
            class="receipt-status"
            data-test="image-save-status"
            role="status"
          >
            {{ saveStatus }}
          </p>
          <p
            v-if="exportStatus"
            class="receipt-status"
            data-test="image-export-status"
            role="status"
          >
            {{ exportStatus }}
          </p>
          <p
            v-if="shareStatus"
            class="receipt-status"
            data-test="image-share-status"
            role="status"
          >
            {{ shareStatus }}
          </p>
        </div>
      </template>
    </div>

    <ConfirmDialog
      :visible="showDiscardConfirm"
      title="Discard generated preview"
      :message="discardMessage"
      confirm-label="Discard"
      cancel-label="Keep preview"
      variant="warning"
      @confirm="discardPreview"
      @cancel="showDiscardConfirm = false"
    />
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { images } from '@/api/client'
import { getRuntimeProfile } from '@/api/transport'
import { hasCapability } from '@/platform/capabilities'
import { RUNTIME_PROFILES } from '@/platform/runtime'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import {
  createGeneratedImageUrl,
  formatGeneratedImageCost,
  revokeGeneratedImageUrl,
  toGeneratedImageError,
} from '@/utils/generatedImage'

const props = defineProps({
  collapsible: { type: Boolean, default: false },
})

const emit = defineEmits(['dirty-change', 'saved'])
const runtimeProfile = getRuntimeProfile()
const isDesktop = runtimeProfile.name === RUNTIME_PROFILES.DESKTOP_WIDE
const isAndroid = runtimeProfile.name === RUNTIME_PROFILES.ANDROID_COMPACT
const imageGenerationAvailable = computed(() => hasCapability(runtimeProfile, 'imageGeneration'))
const nativeImageShareAvailable = computed(() => hasCapability(runtimeProfile, 'nativeImageShare'))
const expanded = ref(!props.collapsible)
const models = ref([])
const modelCapability = ref(null)
const prompt = ref('')
const modelId = ref('')
const resolution = ref('')
const aspectRatio = ref('')
const annotation = ref('')
const retentionPolicy = ref('strip_metadata')
const syncPolicy = ref('local_only')
const preview = ref(null)
const previewUrl = ref('')
const error = ref(null)
const discoveryLoading = ref(false)
const capabilityLoading = ref(false)
const generating = ref(false)
const saving = ref(false)
const exporting = ref(false)
const sharing = ref(false)
const receiptConsumed = ref(false)
const saveStatus = ref('')
const exportStatus = ref('')
const shareStatus = ref('')
const showDiscardConfirm = ref(false)
let active = true
let capabilityGeneration = 0
let discoveryStarted = false

const resolutionOptions = computed(() => uniqueValues(
  squareEndpoints().flatMap(endpoint => endpoint.resolutions || []),
))
const aspectRatioOptions = computed(() => uniqueValues(
  squareEndpoints()
    .filter(endpoint => !resolution.value || endpoint.resolutions?.includes(resolution.value))
    .flatMap(endpoint => endpoint.aspectRatios || [])
    .filter(value => value === '1:1'),
))
const formattedCost = computed(() => formatGeneratedImageCost(preview.value?.cost))
const discardMessage = computed(() => receiptConsumed.value
  ? 'Remove this preview from the composer? The saved Grafyn image will remain.'
  : 'Discard this unsaved preview and its one-shot receipt?')
const canGenerate = computed(() => (
  !discoveryLoading.value
  && !capabilityLoading.value
  && !generating.value
  && !preview.value
  && Boolean(prompt.value.trim())
  && Boolean(modelId.value)
  && resolutionOptions.value.includes(resolution.value)
  && aspectRatioOptions.value.includes(aspectRatio.value)
))
const dirty = computed(() => Boolean(prompt.value.trim() || preview.value))

watch(dirty, value => emit('dirty-change', value), { immediate: true })

watch(expanded, value => {
  if (value) void loadModelsOnce()
})

watch(modelId, async value => {
  const generation = ++capabilityGeneration
  modelCapability.value = null
  resolution.value = ''
  aspectRatio.value = ''
  error.value = null
  if (!value) return
  capabilityLoading.value = true
  try {
    const capability = await images.getModelCapability(value)
    if (active && generation === capabilityGeneration && modelId.value === value) {
      modelCapability.value = capability
    }
  } catch (caught) {
    if (active && generation === capabilityGeneration) error.value = toGeneratedImageError(caught)
  } finally {
    if (active && generation === capabilityGeneration) capabilityLoading.value = false
  }
})

onMounted(() => {
  if (!props.collapsible) void loadModelsOnce()
})

async function loadModelsOnce() {
  if (!imageGenerationAvailable.value || discoveryStarted || !active) return
  discoveryStarted = true
  discoveryLoading.value = true
  try {
    const discovered = await images.discoverModels()
    if (active) models.value = Array.isArray(discovered) ? discovered : []
  } catch (caught) {
    if (active) error.value = toGeneratedImageError(caught)
  } finally {
    if (active) discoveryLoading.value = false
  }
}

onBeforeUnmount(() => {
  active = false
  capabilityGeneration += 1
  if (preview.value && !receiptConsumed.value) {
    discardBackendReceipt(preview.value.receiptId)
  }
  revokeGeneratedImageUrl(previewUrl.value)
})

function squareEndpoints() {
  return (modelCapability.value?.endpoints || [])
    .filter(endpoint => endpoint.aspectRatios?.includes('1:1'))
}

function uniqueValues(values) {
  return [...new Set(values)]
}

async function generateImage() {
  if (!imageGenerationAvailable.value || !canGenerate.value) return
  generating.value = true
  error.value = null
  exportStatus.value = ''
  shareStatus.value = ''
  let generated = null
  try {
    generated = await images.generate({
      prompt: prompt.value,
      modelId: modelId.value,
      resolution: resolution.value,
      aspectRatio: aspectRatio.value,
    })
    if (!active) {
      discardBackendReceipt(generated?.receiptId)
      return
    }
    previewUrl.value = createGeneratedImageUrl(generated)
    preview.value = generated
  } catch (caught) {
    if (generated?.receiptId) discardBackendReceipt(generated.receiptId)
    if (active) error.value = toGeneratedImageError(caught)
  } finally {
    if (active) generating.value = false
  }
}

async function saveToGrafyn() {
  if (!preview.value || receiptConsumed.value || saving.value || exporting.value || sharing.value) return
  const submittedSyncPolicy = syncPolicy.value
  saving.value = true
  error.value = null
  try {
    const saved = await images.save({
      receiptId: preview.value.receiptId,
      annotation: annotation.value.trim() || null,
      retentionPolicy: retentionPolicy.value,
      grafynSync: submittedSyncPolicy,
    })
    if (!active) return
    receiptConsumed.value = true
    saveStatus.value = formatAttachmentSyncDisposition(saved?.syncDisposition)
    emit('saved', saved)
  } catch (caught) {
    if (!active) return
    error.value = toGeneratedImageError(caught)
    lockTerminalReceipt(error.value)
  } finally {
    if (active) saving.value = false
  }
}

async function saveAsDesktopFile() {
  if (!isDesktop || !preview.value || receiptConsumed.value || saving.value || exporting.value || sharing.value) return
  exporting.value = true
  error.value = null
  exportStatus.value = ''
  try {
    const result = await images.saveAs(preview.value.receiptId, retentionPolicy.value)
    if (active) exportStatus.value = result?.exported
      ? 'Desktop file saved.'
      : 'Desktop save canceled.'
  } catch (caught) {
    if (active) {
      error.value = toGeneratedImageError(caught)
      lockTerminalReceipt(error.value)
    }
  } finally {
    if (active) exporting.value = false
  }
}

async function shareAndroidImage() {
  if (!isAndroid
    || !nativeImageShareAvailable.value
    || !preview.value
    || receiptConsumed.value
    || saving.value
    || exporting.value
    || sharing.value) return

  const submittedReceiptId = preview.value.receiptId
  const submittedRetentionPolicy = retentionPolicy.value
  sharing.value = true
  error.value = null
  shareStatus.value = ''
  try {
    const result = await images.shareGeneratedImage(
      submittedReceiptId,
      submittedRetentionPolicy,
    )
    if (result?.shareSheetOpened !== true) {
      throw new Error('Android share sheet did not open.')
    }
    if (active) shareStatus.value = 'Share sheet opened.'
  } catch (caught) {
    if (active) error.value = {
      code: 'SHARE_FAILED',
      message: shareErrorMessage(caught),
    }
  } finally {
    if (active) sharing.value = false
  }
}

function shareErrorMessage(caught) {
  if (typeof caught === 'string' && caught.trim()) return caught
  if (typeof caught?.message === 'string' && caught.message.trim()) return caught.message
  return 'Android image sharing failed.'
}

function formatAttachmentSyncDisposition(disposition) {
  if (disposition?.status === 'local_only') {
    return 'Saved locally to Grafyn · This image was not queued for sync.'
  }
  if (disposition?.status === 'awaiting_provisioning') {
    return 'Saved to Grafyn · This image is awaiting sync provisioning and is not queued yet.'
  }
  if (disposition?.status === 'queued') {
    const operations = disposition.operationCount || 0
    const manifests = disposition.manifestCount || 0
    const chunks = disposition.chunkCount || 0
    return `Saved to Grafyn · This image queued ${operations} attachment operations (${manifests} manifest, ${chunks} chunks).`
  }
  return 'Saved to Grafyn · Attachment sync disposition unavailable.'
}

function lockTerminalReceipt(generatedError) {
  if (['COMMIT_UNCERTAIN', 'RECEIPT_UNAVAILABLE'].includes(generatedError?.code)) {
    receiptConsumed.value = true
  }
}

function discardPreview() {
  if (saving.value || exporting.value || sharing.value) return
  const receiptId = preview.value?.receiptId
  showDiscardConfirm.value = false
  revokeGeneratedImageUrl(previewUrl.value)
  preview.value = null
  previewUrl.value = ''
  receiptConsumed.value = false
  saveStatus.value = ''
  exportStatus.value = ''
  shareStatus.value = ''
  annotation.value = ''
  discardBackendReceipt(receiptId)
}

function discardBackendReceipt(receiptId) {
  if (!receiptId) return
  void images.discard(receiptId).catch(() => {})
}
</script>

<style scoped>
.quick-image-composer,
.image-panel,
.quick-image-form,
.preview-card {
  display: grid;
  gap: var(--spacing-md);
  min-width: 0;
}

.quick-image-composer {
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
}

.image-toggle {
  min-height: 56px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-primary);
  background: transparent;
  border: 0;
  border-radius: inherit;
  text-align: left;
}

.image-toggle > span:first-child {
  display: grid;
  gap: 0.15rem;
}

.image-panel {
  padding: var(--spacing-md);
}

.section-heading,
.preview-meta,
.receipt-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
  flex-wrap: wrap;
}

h2 {
  margin: 0;
  font-size: 1.1rem;
}

.eyebrow,
.field > span {
  color: var(--text-muted);
  font-size: 0.7rem;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.privacy-mark {
  color: var(--accent-cyan);
  font-size: 0.72rem;
}

.field {
  display: grid;
  gap: 0.35rem;
  min-width: 0;
}

.full-field {
  grid-column: 1 / -1;
}

.option-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) repeat(2, minmax(0, 1fr));
  gap: var(--spacing-sm);
}

.save-options {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

textarea,
input,
select,
button {
  min-height: 44px;
  box-sizing: border-box;
  border-radius: var(--radius-md);
}

textarea,
input,
select {
  width: 100%;
  padding: var(--spacing-sm);
  color: var(--text-primary);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
}

textarea {
  resize: vertical;
}

button:focus-visible,
textarea:focus-visible,
input:focus-visible,
select:focus-visible {
  outline: 2px solid var(--accent-cyan);
  outline-offset: 2px;
}

.primary-action,
.secondary-action,
.quiet-action {
  padding: 0 var(--spacing-md);
  font-weight: 700;
}

.primary-action {
  color: var(--bg-primary);
  background: var(--accent-cyan);
  border: 1px solid transparent;
}

.secondary-action {
  color: var(--text-primary);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
}

.quiet-action {
  color: var(--text-secondary);
  background: transparent;
  border: 1px solid transparent;
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.52;
}

.provider-disclosure,
.receipt-status,
.unavailable-state p {
  margin: 0;
  color: var(--text-secondary);
  font-size: 0.82rem;
  line-height: 1.5;
}

.image-error {
  margin: 0;
  padding: var(--spacing-sm);
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-tertiary));
  border-radius: var(--radius-md);
}

.preview-card {
  padding-top: var(--spacing-md);
  border-top: 1px solid var(--border-default);
}

.preview-card img {
  width: 100%;
  max-height: min(56vh, 42rem);
  object-fit: contain;
  background: var(--bg-primary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.preview-meta {
  color: var(--text-secondary);
  font-size: 0.82rem;
}

.preview-meta strong {
  color: var(--text-primary);
}

.save-controls,
.unavailable-state {
  display: grid;
  gap: var(--spacing-sm);
}

@media (max-width: 42rem) {
  .option-grid,
  .save-options {
    grid-template-columns: 1fr;
  }

  .receipt-actions > button {
    flex: 1 1 100%;
  }
}
</style>
