<template>
  <div class="import-upload">
    <div class="header">
      <h2>Import LLM Conversations</h2>
      <p class="subtitle">
        Upload exports from ChatGPT, Claude, Grok, or Gemini to import them into Seedream
      </p>
    </div>

    <div
      class="upload-zone"
      :class="{ 'dragging': isDragging }"
      @dragover.prevent="onDragOver"
      @dragleave="onDragLeave"
      @drop.prevent="onDrop"
      @click="triggerFileInput"
    >
      <input
        ref="fileInput"
        type="file"
        accept=".json,.txt,.md,.dms"
        @change="onFileSelect"
        style="display: none"
      />

      <div v-if="!isUploading" class="upload-content">
        <div class="icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 17a5 5 0 01-9.9-1A5 5 0 005 2v4a5 5 0 0010 0v-4a5 5 0 00-5-5c0-1.646.395-3.1 1.097-4.903z"
            />
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M12 2v20M12 2l7-7M5 5l7-7"
            />
          </svg>
        </div>
        <div class="text">
          <h3>Drop file here or click to browse</h3>
          <p>Supports: ChatGPT (conversations.json), Claude (.dms/.json), Grok (.json), Gemini (.json)</p>
          <p class="max-size">Maximum file size: {{ maxSize }}MB</p>
        </div>
      </div>

      <div v-else class="upload-progress">
        <div class="spinner"></div>
        <p>Uploading file...</p>
      </div>
    </div>

    <div v-if="errors.length > 0" class="error-banner">
      <div v-for="error in errors" :key="error.type" class="error-item">
        <span class="error-icon">⚠️</span>
        <span class="error-message">{{ error.message }}</span>
      </div>
    </div>

    <div v-if="currentJob" class="next-steps">
      <h3>File Uploaded Successfully!</h3>
      <div class="job-info">
        <p><strong>Job ID:</strong> {{ currentJob.id }}</p>
        <p><strong>File:</strong> {{ currentJob.file_name }}</p>
        <p><strong>Status:</strong> {{ currentJob.status }}</p>
      </div>
      <button @click="goToReview" class="btn btn-primary">
        Review & Import Conversations →
      </button>
    </div>
  </div>
</template>

<script>
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useImportStore } from '@/stores/import'

export default {
  name: 'ImportUpload',
  
  setup() {
    const router = useRouter()
    const importStore = useImportStore()
    const fileInput = ref(null)
    const isDragging = ref(false)
    const maxSize = 100

    const onDragOver = () => {
      isDragging.value = true
    }

    const onDragLeave = () => {
      isDragging.value = false
    }

    const onDrop = async (event) => {
      isDragging.value = false
      const files = event.dataTransfer.files
      if (files.length > 0) {
        await handleFile(files[0])
      }
    }

    const triggerFileInput = () => {
      fileInput.value.click()
    }

    const onFileSelect = async (event) => {
      const files = event.target.files
      if (files.length > 0) {
        await handleFile(files[0])
      }
    }

    const handleFile = async (file) => {
      const sizeMB = file.size / (1024 * 1024)
      if (sizeMB > maxSize) {
        importStore.errors.push({
          type: 'validation',
          message: `File too large. Maximum size is ${maxSize}MB`,
          severity: 'error'
        })
        return
      }

      await importStore.uploadFile(file)
    }

    const goToReview = () => {
      if (importStore.currentJobId) {
        router.push('/import/review')
      }
    }

    return {
      fileInput,
      isDragging,
      maxSize,
      isUploading: computed(() => importStore.isUploading),
      errors: computed(() => importStore.errors),
      currentJob: computed(() => importStore.currentJob),
      onDragOver,
      onDragLeave,
      onDrop,
      triggerFileInput,
      onFileSelect,
      goToReview
    }
  }
}
</script>

<style scoped>
.import-upload {
  max-width: 800px;
  margin: 0 auto;
  padding: 2rem;
}

.header {
  text-align: center;
  margin-bottom: 2rem;
}

.header h2 {
  font-size: 1.5rem;
  margin-bottom: 0.5rem;
}

.subtitle {
  color: #666;
  font-size: 0.95rem;
}

.upload-zone {
  border: 2px dashed #ccc;
  border-radius: 8px;
  padding: 3rem;
  text-align: center;
  cursor: pointer;
  transition: all 0.3s ease;
  background: #fafafa;
}

.upload-zone:hover {
  border-color: #4a90e2;
  background: #f0f7ff;
}

.upload-zone.dragging {
  border-color: #4a90e2;
  background: #e8f4fd;
}

.upload-content {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
}

.icon {
  color: #4a90e2;
  margin-bottom: 0.5rem;
}

.text h3 {
  margin: 0 0 0.5rem;
}

.text p {
  color: #666;
  margin: 0.25rem 0;
}

.max-size {
  font-size: 0.85rem;
  color: #999;
}

.upload-progress {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
}

.spinner {
  width: 40px;
  height: 40px;
  border: 4px solid #f3f3f3;
  border-top: 4px solid #4a90e2;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}

.error-banner {
  margin-top: 1.5rem;
  padding: 1rem;
  background: #fee;
  border: 1px solid #fcc;
  border-radius: 4px;
}

.error-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0;
}

.error-icon {
  font-size: 1.2rem;
}

.error-message {
  color: #c00;
}

.next-steps {
  margin-top: 2rem;
  padding: 1.5rem;
  background: #f0f7ff;
  border-radius: 8px;
}

.next-steps h3 {
  margin-top: 0;
}

.job-info {
  background: white;
  padding: 1rem;
  border-radius: 4px;
  margin-bottom: 1rem;
}

.job-info p {
  margin: 0.5rem 0;
}

.btn {
  padding: 0.75rem 1.5rem;
  border: none;
  border-radius: 4px;
  font-size: 1rem;
  cursor: pointer;
  transition: background 0.2s;
}

.btn-primary {
  background: #4a90e2;
  color: white;
}

.btn-primary:hover {
  background: #357abd;
}
</style>