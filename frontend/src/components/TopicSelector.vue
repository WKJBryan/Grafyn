<template>
  <div
    class="topic-selector-overlay"
    @click.self="$emit('close')"
  >
    <div class="topic-selector">
      <div class="selector-header">
        <h3>Select Note Topic</h3>
        <button
          class="close-btn"
          @click="$emit('close')"
        >
          ×
        </button>
      </div>
      
      <div class="selector-body">
        <!-- Note type selection -->
        <div class="type-section">
          <label class="section-label">Note Type</label>
          <div class="type-options">
            <div 
              v-for="type in noteTypes" 
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

        <!-- Topic selection -->
        <div class="topic-section">
          <label class="section-label">Topic</label>
          
          <!-- New topic input -->
          <div class="new-topic-input">
            <input 
              v-model="newTopic"
              type="text"
              placeholder="Create new topic or select below..."
              @keyup.enter="handleCreate"
            >
          </div>
          
          <!-- Existing topics -->
          <div
            v-if="existingTopics.length > 0"
            class="existing-topics"
          >
            <div 
              v-for="topic in existingTopics" 
              :key="topic"
              class="topic-chip"
              :class="{ selected: selectedTopic === topic }"
              @click="selectTopic(topic)"
            >
              {{ formatTopicName(topic) }}
            </div>
          </div>
          
          <div class="no-topic-option">
            <label class="checkbox-label">
              <input 
                v-model="skipTopic" 
                type="checkbox"
              >
              Skip - create without topic
            </label>
          </div>
        </div>
      </div>

      <div class="selector-footer">
        <button
          class="btn btn-ghost"
          @click="$emit('close')"
        >
          Cancel
        </button>
        <button 
          class="btn btn-primary" 
          :disabled="!canCreate"
          @click="handleCreate"
        >
          Create Note
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'

const props = defineProps({
  existingTopics: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['create', 'close'])

const selectedType = ref('general')
const selectedTopic = ref('')
const newTopic = ref('')
const skipTopic = ref(false)

const noteTypes = [
  { value: 'general', icon: '📄', label: 'Note', description: 'General purpose note' },
  { value: 'container', icon: '📂', label: 'Source', description: 'Research or reference material' },
  { value: 'hub', icon: '🗺️', label: 'Map of Content', description: 'Hub linking related notes' },
]

const canCreate = computed(() => {
  return skipTopic.value || selectedTopic.value || newTopic.value.trim()
})

function selectTopic(topic) {
  selectedTopic.value = topic
  newTopic.value = ''
}

function formatTopicName(name) {
  if (!name) return 'Unknown'
  return name.charAt(0).toUpperCase() + name.slice(1)
}

function handleCreate() {
  if (!canCreate.value) return
  
  const topic = skipTopic.value 
    ? null 
    : (newTopic.value.trim() || selectedTopic.value)
  
  emit('create', {
    note_type: selectedType.value,
    topic: topic ? topic.toLowerCase() : null
  })
}
</script>

<style scoped>
.topic-selector-overlay {
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

.topic-selector {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 480px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  animation: slideUp 0.3s ease;
}

@keyframes slideUp {
  from { transform: translateY(20px); opacity: 0; }
  to { transform: translateY(0); opacity: 1; }
}

.selector-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
}

.selector-header h3 {
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

.selector-body {
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
  flex-direction: column;
  gap: var(--spacing-xs);
}

.type-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
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
  font-size: 1.25rem;
}

.option-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.option-label {
  font-weight: 600;
  color: var(--text-primary);
}

.option-desc {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.topic-section {
  margin-bottom: var(--spacing-md);
}

.new-topic-input input {
  width: 100%;
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.95rem;
  transition: all var(--transition-fast);
}

.new-topic-input input:focus {
  outline: none;
  border-color: var(--accent-primary);
  box-shadow: 0 0 0 3px rgba(var(--accent-primary-rgb), 0.15);
}

.existing-topics {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-sm);
}

.topic-chip {
  padding: 4px 12px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-full);
  font-size: 0.85rem;
  color: var(--text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.topic-chip:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.topic-chip.selected {
  background: var(--accent-primary);
  color: white;
}

.no-topic-option {
  margin-top: var(--spacing-md);
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

.selector-footer {
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
}

.btn-ghost {
  background: transparent;
  border: 1px solid var(--bg-tertiary);
  color: var(--text-secondary);
}

.btn-ghost:hover {
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

.btn-primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
