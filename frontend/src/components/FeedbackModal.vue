<template>
  <div class="feedback-modal-overlay" @click.self="$emit('close')">
    <div class="feedback-modal">
      <div class="modal-header">
        <h3>Send Feedback</h3>
        <button class="close-btn" @click="$emit('close')">×</button>
      </div>

      <div class="modal-body">
        <!-- Feedback Type Selection -->
        <div class="type-section">
          <label class="section-label">Type</label>
          <div class="type-options">
            <div
              v-for="type in feedbackTypes"
              :key="type.value"
              class="type-option"
              :class="{ selected: selectedType === type.value }"
              @click="selectedType = type.value"
            >
              <span class="option-icon">{{ type.icon }}</span>
              <div class="option-text">
                <span class="option-label">{{ type.label }}</span>
                <span class="option-desc">{{ type.description }}</span>
              </div>
            </div>
          </div>
        </div>

        <!-- Title Input -->
        <div class="input-section">
          <label class="section-label">Title</label>
          <input
            v-model="title"
            type="text"
            class="text-input"
            placeholder="Brief summary of your feedback..."
            maxlength="200"
            @input="validateForm"
          />
          <span class="char-count" :class="{ error: title.length < 5 || title.length > 200 }">
            {{ title.length }}/200
          </span>
        </div>

        <!-- Description Input -->
        <div class="input-section">
          <label class="section-label">Description</label>
          <textarea
            v-model="description"
            class="textarea-input"
            placeholder="Please provide details about your feedback, bug, or feature request..."
            maxlength="10000"
            rows="6"
            @input="validateForm"
          ></textarea>
          <span class="char-count" :class="{ error: description.length < 10 || description.length > 10000 }">
            {{ description.length }}/10000
          </span>
        </div>

        <!-- System Info Opt-in -->
        <div class="system-info-section">
          <label class="checkbox-label">
            <input
              type="checkbox"
              v-model="includeSystemInfo"
              @change="loadSystemInfo"
            />
            Include system information
          </label>

          <div v-if="includeSystemInfo && systemInfo" class="system-info-preview">
            <div class="info-item">
              <span class="info-label">Platform:</span>
              <span class="info-value">{{ systemInfo.platform }}</span>
            </div>
            <div class="info-item">
              <span class="info-label">App Version:</span>
              <span class="info-value">{{ systemInfo.app_version }}</span>
            </div>
            <div class="info-item">
              <span class="info-label">Runtime:</span>
              <span class="info-value">{{ systemInfo.runtime }}</span>
            </div>
          </div>
        </div>

        <!-- Error Message -->
        <div v-if="errorMessage" class="error-message">
          {{ errorMessage }}
        </div>

        <!-- Success Message -->
        <div v-if="successMessage" class="success-message">
          {{ successMessage }}
          <a v-if="issueUrl" :href="issueUrl" target="_blank" class="issue-link">
            View Issue →
          </a>
        </div>
      </div>

      <div class="modal-footer">
        <button class="btn btn-ghost" @click="$emit('close')" :disabled="isSubmitting">
          {{ submitted ? 'Close' : 'Cancel' }}
        </button>
        <button
          v-if="!submitted"
          class="btn btn-primary"
          @click="handleSubmit"
          :disabled="!isValid || isSubmitting"
        >
          <span v-if="isSubmitting" class="loading-spinner"></span>
          {{ isSubmitting ? 'Submitting...' : 'Submit Feedback' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { feedback as feedbackApi, isDesktopApp } from '../api/client'

const emit = defineEmits(['close', 'submitted'])

const selectedType = ref('general')
const title = ref('')
const description = ref('')
const includeSystemInfo = ref(false)
const systemInfo = ref(null)
const isSubmitting = ref(false)
const errorMessage = ref('')
const successMessage = ref('')
const issueUrl = ref('')
const submitted = ref(false)

const feedbackTypes = [
  { value: 'bug', icon: '🐛', label: 'Bug Report', description: 'Something isn\'t working correctly' },
  { value: 'feature', icon: '💡', label: 'Feature Request', description: 'Suggest a new feature or improvement' },
  { value: 'general', icon: '💬', label: 'General Feedback', description: 'Share your thoughts or questions' },
]

const isValid = computed(() => {
  const titleLen = title.value.trim().length
  const descLen = description.value.trim().length
  return titleLen >= 5 && titleLen <= 200 && descLen >= 10 && descLen <= 10000
})

function validateForm() {
  errorMessage.value = ''
}

async function loadSystemInfo() {
  if (!includeSystemInfo.value) {
    systemInfo.value = null
    return
  }

  try {
    systemInfo.value = await feedbackApi.getSystemInfo(window.location.pathname)
  } catch (e) {
    console.error('Failed to load system info:', e)
    // Fallback for web mode
    systemInfo.value = {
      platform: navigator.platform || 'Unknown',
      app_version: '1.0.0',
      runtime: isDesktopApp() ? 'tauri-desktop' : 'web-browser',
      current_page: window.location.pathname,
    }
  }
}

async function handleSubmit() {
  if (!isValid.value || isSubmitting.value) return

  isSubmitting.value = true
  errorMessage.value = ''
  successMessage.value = ''

  try {
    const feedbackData = {
      title: title.value.trim(),
      description: description.value.trim(),
      feedback_type: selectedType.value,
      include_system_info: includeSystemInfo.value,
      system_info: includeSystemInfo.value ? systemInfo.value : null,
    }

    const response = await feedbackApi.submit(feedbackData)

    if (response.success) {
      submitted.value = true
      if (response.queued) {
        successMessage.value = 'Feedback saved! It will be submitted when you\'re back online.'
      } else {
        successMessage.value = response.message
        issueUrl.value = response.issue_url || ''
      }
      emit('submitted', response)
    } else {
      errorMessage.value = response.message || 'Failed to submit feedback'
    }
  } catch (e) {
    console.error('Failed to submit feedback:', e)
    errorMessage.value = e.message || 'An unexpected error occurred'
  } finally {
    isSubmitting.value = false
  }
}

onMounted(async () => {
  // Check if service is configured
  try {
    const status = await feedbackApi.status()
    if (!status.configured) {
      errorMessage.value = 'Feedback service is not configured. Please contact the administrator.'
    }
  } catch (e) {
    // Service might not be available in web mode without backend
    console.warn('Could not check feedback status:', e)
  }
})
</script>

<style scoped>
.feedback-modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  animation: fadeIn 0.2s ease;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.feedback-modal {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 520px;
  max-height: 90vh;
  overflow-y: auto;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  animation: slideUp 0.3s ease;
}

@keyframes slideUp {
  from { transform: translateY(20px); opacity: 0; }
  to { transform: translateY(0); opacity: 1; }
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
}

.modal-header h3 {
  margin: 0;
  font-size: 1.1rem;
  color: var(--text-primary);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0;
  line-height: 1;
  transition: color var(--transition-fast);
}

.close-btn:hover {
  color: var(--text-primary);
}

.modal-body {
  padding: var(--spacing-lg);
}

.section-label {
  display: block;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: var(--spacing-sm);
}

.type-section {
  margin-bottom: var(--spacing-lg);
}

.type-options {
  display: flex;
  gap: var(--spacing-sm);
}

.type-option {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-md);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
  text-align: center;
}

.type-option:hover {
  background: var(--bg-hover);
  border-color: var(--text-muted);
}

.type-option.selected {
  background: var(--accent-primary);
  background: linear-gradient(135deg, var(--accent-primary) 0%, var(--accent-secondary) 100%);
  border-color: var(--accent-primary);
}

.type-option.selected .option-label,
.type-option.selected .option-desc {
  color: white;
}

.option-icon {
  font-size: 1.5rem;
}

.option-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.option-label {
  font-weight: 600;
  font-size: 0.9rem;
  color: var(--text-primary);
}

.option-desc {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.input-section {
  margin-bottom: var(--spacing-md);
  position: relative;
}

.text-input,
.textarea-input {
  width: 100%;
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.95rem;
  font-family: inherit;
  transition: all var(--transition-fast);
  box-sizing: border-box;
}

.textarea-input {
  resize: vertical;
  min-height: 120px;
}

.text-input:focus,
.textarea-input:focus {
  outline: none;
  border-color: var(--accent-primary);
  box-shadow: 0 0 0 3px rgba(var(--accent-primary-rgb), 0.15);
}

.char-count {
  position: absolute;
  right: 8px;
  bottom: -18px;
  font-size: 0.75rem;
  color: var(--text-muted);
}

.char-count.error {
  color: var(--error);
}

.system-info-section {
  margin-top: var(--spacing-lg);
  margin-bottom: var(--spacing-md);
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  font-size: 0.9rem;
  color: var(--text-secondary);
  cursor: pointer;
}

.checkbox-label input {
  accent-color: var(--accent-primary);
}

.system-info-preview {
  margin-top: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border-radius: var(--radius-md);
  font-size: 0.85rem;
}

.info-item {
  display: flex;
  gap: var(--spacing-sm);
  margin-bottom: 4px;
}

.info-item:last-child {
  margin-bottom: 0;
}

.info-label {
  color: var(--text-muted);
  min-width: 90px;
}

.info-value {
  color: var(--text-secondary);
}

.error-message {
  margin-top: var(--spacing-md);
  padding: var(--spacing-sm) var(--spacing-md);
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.3);
  border-radius: var(--radius-md);
  color: #ef4444;
  font-size: 0.9rem;
}

.success-message {
  margin-top: var(--spacing-md);
  padding: var(--spacing-sm) var(--spacing-md);
  background: rgba(34, 197, 94, 0.1);
  border: 1px solid rgba(34, 197, 94, 0.3);
  border-radius: var(--radius-md);
  color: #22c55e;
  font-size: 0.9rem;
}

.issue-link {
  display: inline-block;
  margin-top: var(--spacing-xs);
  color: var(--accent-primary);
  text-decoration: none;
}

.issue-link:hover {
  text-decoration: underline;
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
}

.btn {
  padding: var(--spacing-sm) var(--spacing-lg);
  border-radius: var(--radius-md);
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.btn-ghost {
  background: transparent;
  border: 1px solid var(--bg-tertiary);
  color: var(--text-secondary);
}

.btn-ghost:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.btn-primary {
  background: linear-gradient(135deg, var(--accent-primary) 0%, var(--accent-secondary) 100%);
  border: none;
  color: white;
}

.btn-primary:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(var(--accent-primary-rgb), 0.3);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.loading-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* Responsive adjustments */
@media (max-width: 480px) {
  .type-options {
    flex-direction: column;
  }

  .type-option {
    flex-direction: row;
    justify-content: flex-start;
    text-align: left;
  }

  .option-icon {
    font-size: 1.25rem;
  }
}
</style>
