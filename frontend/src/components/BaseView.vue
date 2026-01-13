<template>
  <div class="base-view">
    <!-- View Controls -->
    <div class="view-controls">
      <div class="view-controls-left">
        <!-- View Mode Switcher -->
        <div class="view-mode-switcher">
          <button
            v-for="mode in viewModes"
            :key="mode.value"
            :class="['view-mode-btn', { active: currentViewMode === mode.value }]"
            @click="currentViewMode = mode.value"
            :title="mode.label"
          >
            <span class="view-mode-icon">{{ mode.icon }}</span>
            <span class="view-mode-label">{{ mode.label }}</span>
          </button>
        </div>

        <!-- Search -->
        <div class="search-box">
          <input
            v-model="searchQuery"
            type="text"
            placeholder="Search notes..."
            @input="applyFilters"
          />
        </div>
      </div>

      <div class="view-controls-right">
        <!-- Sort Controls -->
        <div class="sort-controls">
          <select v-model="sortBy" @change="applySort">
            <option value="title">Sort by Title</option>
            <option value="created">Sort by Created</option>
            <option value="modified">Sort by Modified</option>
            <option value="status">Sort by Status</option>
            <option v-for="prop in availableProperties" :key="prop" :value="`property.${prop}`">
              Sort by {{ prop }}
            </option>
          </select>
          <button
            :class="['sort-direction-btn', { active: sortDirection === 'asc' }]"
            @click="toggleSortDirection"
            title="Toggle sort direction"
          >
            {{ sortDirection === 'asc' ? '↑' : '↓' }}
          </button>
        </div>

        <!-- Filter Button -->
        <button
          :class="['filter-toggle-btn', { active: showFilters }]"
          @click="showFilters = !showFilters"
        >
          <span>Filters</span>
          <span v-if="activeFilterCount > 0" class="filter-badge">{{ activeFilterCount }}</span>
        </button>

        <!-- Column Selector (Table View Only) -->
        <button
          v-if="currentViewMode === 'table'"
          class="column-selector-btn"
          @click="showColumnSelector = !showColumnSelector"
          title="Select columns"
        >
          Columns
        </button>

        <!-- Group Selector (Kanban View Only) -->
        <select
          v-if="currentViewMode === 'kanban'"
          v-model="groupBy"
          @change="applyGrouping"
          class="group-selector"
        >
          <option value="status">Group by Status</option>
          <option v-for="prop in availableProperties" :key="prop" :value="`property.${prop}`">
            Group by {{ prop }}
          </option>
        </select>

        <!-- Refresh Button -->
        <button class="refresh-btn" @click="loadNotes" :disabled="loading">
          <span v-if="loading">⟳</span>
          <span v-else>⟳</span>
        </button>
      </div>
    </div>

    <!-- Filter Panel -->
    <div v-if="showFilters" class="filter-panel">
      <div class="filter-section">
        <h4>Status Filter</h4>
        <div class="filter-options">
          <label v-for="status in statusOptions" :key="status" class="filter-option">
            <input
              type="checkbox"
              :value="status"
              v-model="selectedStatuses"
              @change="applyFilters"
            />
            <span :class="['status-badge', `status-${status}`]">{{ status }}</span>
          </label>
        </div>
      </div>

      <div class="filter-section">
        <h4>Tags Filter</h4>
        <div class="filter-options">
          <label v-for="tag in availableTags" :key="tag" class="filter-option">
            <input
              type="checkbox"
              :value="tag"
              v-model="selectedTags"
              @change="applyFilters"
            />
            <span class="tag">{{ tag }}</span>
          </label>
        </div>
      </div>

      <div class="filter-section">
        <h4>Property Filters</h4>
        <div v-for="(propFilter, index) in propertyFilters" :key="index" class="property-filter">
          <select v-model="propFilter.property" @change="applyFilters">
            <option value="">Select property...</option>
            <option v-for="prop in availableProperties" :key="prop" :value="prop">
              {{ prop }}
            </option>
          </select>
          <select v-model="propFilter.operator" @change="applyFilters">
            <option value="equals">=</option>
            <option value="not_equals">≠</option>
            <option value="contains">contains</option>
            <option value="gt">></option>
            <option value="lt"><</option>
            <option value="gte">≥</option>
            <option value="lte">≤</option>
          </select>
          <input
            v-model="propFilter.value"
            type="text"
            placeholder="Value"
            @input="applyFilters"
          />
          <button class="remove-filter-btn" @click="removePropertyFilter(index)">×</button>
        </div>
        <button class="add-filter-btn" @click="addPropertyFilter">+ Add Property Filter</button>
      </div>

      <div class="filter-actions">
        <button class="btn btn-secondary" @click="clearFilters">Clear Filters</button>
      </div>
    </div>

    <!-- Column Selector (Table View) -->
    <div v-if="showColumnSelector && currentViewMode === 'table'" class="column-selector">
      <h4>Select Columns</h4>
      <div class="column-options">
        <label v-for="col in availableColumns" :key="col.value" class="column-option">
          <input
            type="checkbox"
            :value="col.value"
            v-model="selectedColumns"
            @change="updateVisibleColumns"
          />
          <span>{{ col.label }}</span>
        </label>
      </div>
    </div>

    <!-- Loading State -->
    <div v-if="loading" class="loading-state">
      <div class="spinner"></div>
      <p>Loading notes...</p>
    </div>

    <!-- Error State -->
    <div v-else-if="error" class="error-state">
      <p>{{ error }}</p>
      <button class="btn btn-primary" @click="loadNotes">Retry</button>
    </div>

    <!-- Empty State -->
    <div v-else-if="filteredNotes.length === 0" class="empty-state">
      <p>No notes found</p>
      <p class="text-muted">Try adjusting your filters or search query</p>
    </div>

    <!-- Table View -->
    <div v-else-if="currentViewMode === 'table'" class="table-view">
      <table class="notes-table">
        <thead>
          <tr>
            <th
              v-for="col in visibleColumns"
              :key="col.value"
              :class="{ sortable: col.sortable }"
              @click="col.sortable && handleColumnSort(col.value)"
            >
              {{ col.label }}
              <span v-if="sortBy === col.value" class="sort-indicator">
                {{ sortDirection === 'asc' ? '↑' : '↓' }}
              </span>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="note in sortedNotes"
            :key="note.id"
            class="note-row"
            @click="handleNoteClick(note.id)"
          >
            <td v-if="visibleColumns.find(c => c.value === 'title')" class="title-cell">
              {{ note.title || 'Untitled' }}
            </td>
            <td v-if="visibleColumns.find(c => c.value === 'status')" class="status-cell">
              <span :class="['status', `status-${note.status}`]">{{ note.status }}</span>
            </td>
            <td v-if="visibleColumns.find(c => c.value === 'tags')" class="tags-cell">
              <div class="tags-container">
                <span v-for="(tag, i) in (note.tags || []).slice(0, 3)" :key="i" class="tag">
                  {{ tag }}
                </span>
                <span v-if="(note.tags || []).length > 3" class="tag">
                  +{{ note.tags.length - 3 }}
                </span>
              </div>
            </td>
            <td v-if="visibleColumns.find(c => c.value === 'created')" class="date-cell">
              {{ formatDate(note.created) }}
            </td>
            <td v-if="visibleColumns.find(c => c.value === 'modified')" class="date-cell">
              {{ formatDate(note.modified) }}
            </td>
            <td v-if="visibleColumns.find(c => c.value === 'link_count')" class="number-cell">
              {{ note.link_count || 0 }}
            </td>
            <td v-for="prop in propertyColumns" :key="prop" class="property-cell">
              {{ getPropertyValue(note, prop) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Card View -->
    <div v-else-if="currentViewMode === 'card'" class="card-view">
      <div class="cards-grid">
        <div
          v-for="note in sortedNotes"
          :key="note.id"
          class="note-card card card-hover"
          @click="handleNoteClick(note.id)"
        >
          <div class="card-header">
            <h4 class="card-title">{{ note.title || 'Untitled' }}</h4>
            <span :class="['status', `status-${note.status}`]">{{ note.status }}</span>
          </div>
          <div v-if="note.tags && note.tags.length > 0" class="card-tags">
            <span v-for="(tag, i) in note.tags.slice(0, 5)" :key="i" class="tag">
              {{ tag }}
            </span>
            <span v-if="note.tags.length > 5" class="tag">+{{ note.tags.length - 5 }}</span>
          </div>
          <div v-if="hasProperties(note)" class="card-properties">
            <div v-for="(value, prop) in getNoteProperties(note)" :key="prop" class="property-item">
              <span class="property-name">{{ prop }}:</span>
              <span class="property-value">{{ formatPropertyValue(value) }}</span>
            </div>
          </div>
          <div class="card-footer">
            <span class="text-muted text-sm">{{ formatDate(note.modified) }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- Kanban View -->
    <div v-else-if="currentViewMode === 'kanban'" class="kanban-view">
      <div class="kanban-board">
        <div
          v-for="group in kanbanGroups"
          :key="group.name"
          class="kanban-column"
          @dragover.prevent
          @drop="handleDrop($event, group.name)"
        >
          <div class="kanban-column-header">
            <h4>{{ group.label }}</h4>
            <span class="kanban-count">{{ group.notes.length }}</span>
          </div>
          <div class="kanban-column-body">
            <div
              v-for="note in group.notes"
              :key="note.id"
              class="kanban-card"
              draggable="true"
              @dragstart="handleDragStart($event, note)"
              @click="handleNoteClick(note.id)"
            >
              <div class="kanban-card-header">
                <span class="kanban-card-title">{{ note.title || 'Untitled' }}</span>
                <span :class="['status', `status-${note.status}`]">{{ note.status }}</span>
              </div>
              <div v-if="note.tags && note.tags.length > 0" class="kanban-card-tags">
                <span v-for="(tag, i) in note.tags.slice(0, 3)" :key="i" class="tag">
                  {{ tag }}
                </span>
              </div>
              <div v-if="hasProperties(note)" class="kanban-card-properties">
                <div v-for="(value, prop) in getNoteProperties(note).slice(0, 2)" :key="prop" class="property-item">
                  <span class="property-name">{{ prop }}:</span>
                  <span class="property-value">{{ formatPropertyValue(value) }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { notes as notesApi } from '../api/client'

// Props
const props = defineProps({
  initialViewMode: {
    type: String,
    default: 'table'
  },
  initialSortBy: {
    type: String,
    default: 'modified'
  },
  initialSortDirection: {
    type: String,
    default: 'desc'
  }
})

// Emits
const emit = defineEmits(['note-click', 'note-move'])

// View Modes
const viewModes = [
  { value: 'table', label: 'Table', icon: '▦' },
  { value: 'card', label: 'Cards', icon: '▤' },
  { value: 'kanban', label: 'Kanban', icon: '▥' }
]

// State
const currentViewMode = ref(props.initialViewMode)
const notes = ref([])
const loading = ref(false)
const error = ref(null)

// Search
const searchQuery = ref('')

// Sorting
const sortBy = ref(props.initialSortBy)
const sortDirection = ref(props.initialSortDirection)

// Filters
const showFilters = ref(false)
const selectedStatuses = ref(['draft', 'evidence', 'canonical'])
const selectedTags = ref([])
const propertyFilters = ref([])

// Grouping (Kanban)
const groupBy = ref('status')

// Column Selector (Table)
const showColumnSelector = ref(false)
const selectedColumns = ref(['title', 'status', 'tags', 'modified'])

// Available Columns
const availableColumns = [
  { value: 'title', label: 'Title', sortable: true },
  { value: 'status', label: 'Status', sortable: true },
  { value: 'tags', label: 'Tags', sortable: false },
  { value: 'created', label: 'Created', sortable: true },
  { value: 'modified', label: 'Modified', sortable: true },
  { value: 'link_count', label: 'Links', sortable: true }
]

// Status Options
const statusOptions = ['draft', 'evidence', 'canonical']

// Computed
const visibleColumns = computed(() => {
  return availableColumns.filter(col => selectedColumns.value.includes(col.value))
})

const propertyColumns = computed(() => {
  return availableProperties.value.filter(prop => selectedColumns.value.includes(`property.${prop}`))
})

const availableTags = computed(() => {
  const tags = new Set()
  notes.value.forEach(note => {
    if (note.tags) {
      note.tags.forEach(tag => tags.add(tag))
    }
  })
  return Array.from(tags).sort()
})

const availableProperties = computed(() => {
  const props = new Set()
  notes.value.forEach(note => {
    if (note.properties) {
      Object.keys(note.properties).forEach(prop => props.add(prop))
    }
  })
  return Array.from(props).sort()
})

const filteredNotes = computed(() => {
  let result = [...notes.value]

  // Search filter
  if (searchQuery.value) {
    const query = searchQuery.value.toLowerCase()
    result = result.filter(note => {
      const titleMatch = (note.title || '').toLowerCase().includes(query)
      const contentMatch = (note.content || '').toLowerCase().includes(query)
      const tagMatch = (note.tags || []).some(tag => tag.toLowerCase().includes(query))
      return titleMatch || contentMatch || tagMatch
    })
  }

  // Status filter
  if (selectedStatuses.value.length > 0) {
    result = result.filter(note => selectedStatuses.value.includes(note.status))
  }

  // Tags filter
  if (selectedTags.value.length > 0) {
    result = result.filter(note => {
      if (!note.tags || note.tags.length === 0) return false
      return selectedTags.value.every(tag => note.tags.includes(tag))
    })
  }

  // Property filters
  propertyFilters.value.forEach(filter => {
    if (filter.property && filter.operator && filter.value) {
      result = result.filter(note => {
        const propValue = getPropertyValue(note, filter.property)
        return compareValues(propValue, filter.operator, filter.value)
      })
    }
  })

  return result
})

const sortedNotes = computed(() => {
  const result = [...filteredNotes.value]
  
  result.sort((a, b) => {
    let aValue, bValue

    if (sortBy.value.startsWith('property.')) {
      const propName = sortBy.value.replace('property.', '')
      aValue = getPropertyValue(a, propName)
      bValue = getPropertyValue(b, propName)
    } else {
      aValue = a[sortBy.value]
      bValue = b[sortBy.value]
    }

    if (aValue === undefined || aValue === null) return 1
    if (bValue === undefined || bValue === null) return -1

    if (typeof aValue === 'string' && typeof bValue === 'string') {
      return sortDirection.value === 'asc'
        ? aValue.localeCompare(bValue)
        : bValue.localeCompare(aValue)
    }

    if (typeof aValue === 'number' && typeof bValue === 'number') {
      return sortDirection.value === 'asc'
        ? aValue - bValue
        : bValue - aValue
    }

    // Handle dates
    const aDate = new Date(aValue)
    const bDate = new Date(bValue)
    if (!isNaN(aDate.getTime()) && !isNaN(bDate.getTime())) {
      return sortDirection.value === 'asc'
        ? aDate - bDate
        : bDate - aDate
    }

    return 0
  })

  return result
})

const kanbanGroups = computed(() => {
  const groups = {}

  if (groupBy.value === 'status') {
    statusOptions.forEach(status => {
      groups[status] = {
        name: status,
        label: status.charAt(0).toUpperCase() + status.slice(1),
        notes: sortedNotes.value.filter(note => note.status === status)
      }
    })
  } else {
    // Group by property
    const propName = groupBy.value.replace('property.', '')
    const uniqueValues = new Set()
    sortedNotes.value.forEach(note => {
      const value = getPropertyValue(note, propName)
      if (value !== undefined && value !== null && value !== '') {
        uniqueValues.add(value)
      }
    })

    uniqueValues.forEach(value => {
      groups[value] = {
        name: value,
        label: String(value),
        notes: sortedNotes.value.filter(note => getPropertyValue(note, propName) === value)
      }
    })

    // Add "Uncategorized" group
    groups['uncategorized'] = {
      name: 'uncategorized',
      label: 'Uncategorized',
      notes: sortedNotes.value.filter(note => {
        const value = getPropertyValue(note, propName)
        return value === undefined || value === null || value === ''
      })
    }
  }

  return Object.values(groups).filter(group => group.notes.length > 0)
})

const activeFilterCount = computed(() => {
  let count = 0
  if (selectedStatuses.value.length !== statusOptions.length) count++
  if (selectedTags.value.length > 0) count++
  if (propertyFilters.value.length > 0) count++
  if (searchQuery.value) count++
  return count
})

// Methods
async function loadNotes() {
  loading.value = true
  error.value = null
  try {
    const data = await notesApi.list()
    notes.value = data || []
  } catch (err) {
    error.value = err.message || 'Failed to load notes'
    console.error('Failed to load notes:', err)
  } finally {
    loading.value = false
  }
}

function handleNoteClick(noteId) {
  emit('note-click', noteId)
}

function handleColumnSort(column) {
  if (sortBy.value === column) {
    toggleSortDirection()
  } else {
    sortBy.value = column
    sortDirection.value = 'asc'
  }
}

function toggleSortDirection() {
  sortDirection.value = sortDirection.value === 'asc' ? 'desc' : 'asc'
}

function applyFilters() {
  // Filters are applied via computed properties
}

function applySort() {
  // Sort is applied via computed properties
}

function applyGrouping() {
  // Grouping is applied via computed properties
}

function clearFilters() {
  searchQuery.value = ''
  selectedStatuses.value = [...statusOptions]
  selectedTags.value = []
  propertyFilters.value = []
}

function addPropertyFilter() {
  propertyFilters.value.push({
    property: '',
    operator: 'equals',
    value: ''
  })
}

function removePropertyFilter(index) {
  propertyFilters.value.splice(index, 1)
}

function updateVisibleColumns() {
  // Update available columns to include properties
  availableProperties.value.forEach(prop => {
    if (!availableColumns.find(c => c.value === `property.${prop}`)) {
      availableColumns.push({
        value: `property.${prop}`,
        label: prop,
        sortable: true
      })
    }
  })
}

function getPropertyValue(note, propertyName) {
  if (!note.properties) return undefined
  return note.properties[propertyName]
}

function getNoteProperties(note) {
  return note.properties || {}
}

function hasProperties(note) {
  return note.properties && Object.keys(note.properties).length > 0
}

function formatPropertyValue(value) {
  if (value === undefined || value === null) return ''
  if (typeof value === 'boolean') return value ? 'Yes' : 'No'
  if (Array.isArray(value)) return value.join(', ')
  return String(value)
}

function compareValues(value, operator, compareValue) {
  const val = String(value || '').toLowerCase()
  const comp = String(compareValue || '').toLowerCase()

  switch (operator) {
    case 'equals':
      return val === comp
    case 'not_equals':
      return val !== comp
    case 'contains':
      return val.includes(comp)
    case 'gt':
      return parseFloat(val) > parseFloat(comp)
    case 'lt':
      return parseFloat(val) < parseFloat(comp)
    case 'gte':
      return parseFloat(val) >= parseFloat(comp)
    case 'lte':
      return parseFloat(val) <= parseFloat(comp)
    default:
      return false
  }
}

function formatDate(dateString) {
  if (!dateString) return ''
  const date = new Date(dateString)
  if (isNaN(date.getTime())) return dateString
  return date.toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric'
  })
}

// Drag and Drop for Kanban
let draggedNote = null

function handleDragStart(event, note) {
  draggedNote = note
  event.dataTransfer.effectAllowed = 'move'
  event.target.classList.add('dragging')
}

function handleDrop(event, groupName) {
  event.preventDefault()
  if (!draggedNote) return

  // Remove dragging class
  document.querySelectorAll('.kanban-card').forEach(el => {
    el.classList.remove('dragging')
  })

  // Update note based on group
  if (groupBy.value === 'status') {
    // Update status
    const updatedNote = { ...draggedNote, status: groupName }
    emit('note-move', updatedNote)
  } else {
    // Update property
    const propName = groupBy.value.replace('property.', '')
    const updatedNote = {
      ...draggedNote,
      properties: {
        ...draggedNote.properties,
        [propName]: groupName === 'uncategorized' ? null : groupName
      }
    }
    emit('note-move', updatedNote)
  }

  draggedNote = null
}

// Lifecycle
onMounted(() => {
  loadNotes()
})

// Watch for changes to update columns
watch(availableProperties, () => {
  updateVisibleColumns()
}, { immediate: true })
</script>

<style scoped>
.base-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

/* View Controls */
.view-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--bg-tertiary);
  gap: var(--spacing-md);
  flex-wrap: wrap;
}

.view-controls-left,
.view-controls-right {
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  flex-wrap: wrap;
}

/* View Mode Switcher */
.view-mode-switcher {
  display: flex;
  gap: var(--spacing-xs);
  background: var(--bg-tertiary);
  padding: var(--spacing-xs);
  border-radius: var(--radius-md);
}

.view-mode-btn {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.view-mode-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.view-mode-btn.active {
  background: var(--accent-primary);
  color: white;
}

.view-mode-icon {
  font-size: 1rem;
}

.view-mode-label {
  font-size: 0.875rem;
}

/* Search Box */
.search-box {
  position: relative;
  flex: 1;
  min-width: 200px;
  max-width: 400px;
}

.search-box input {
  width: 100%;
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.875rem;
  transition: border-color var(--transition-fast);
}

.search-box input:focus {
  outline: none;
  border-color: var(--accent-primary);
}

/* Sort Controls */
.sort-controls {
  display: flex;
  gap: var(--spacing-xs);
}

.sort-controls select {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.875rem;
  cursor: pointer;
}

.sort-direction-btn {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.sort-direction-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.sort-direction-btn.active {
  background: var(--accent-primary);
  color: white;
}

/* Filter Toggle Button */
.filter-toggle-btn {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.filter-toggle-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.filter-toggle-btn.active {
  background: var(--accent-primary);
  color: white;
}

.filter-badge {
  background: var(--accent-danger);
  color: white;
  font-size: 0.75rem;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
}

/* Column Selector Button */
.column-selector-btn {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.column-selector-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

/* Group Selector */
.group-selector {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.875rem;
  cursor: pointer;
}

/* Refresh Button */
.refresh-btn {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: 1rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.refresh-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.refresh-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* Filter Panel */
.filter-panel {
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--bg-tertiary);
  max-height: 400px;
  overflow-y: auto;
}

.filter-section {
  margin-bottom: var(--spacing-md);
}

.filter-section h4 {
  margin: 0 0 var(--spacing-sm) 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
}

.filter-options {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-sm);
}

.filter-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background-color var(--transition-fast);
}

.filter-option:hover {
  background: var(--bg-hover);
}

.filter-option input[type="checkbox"] {
  width: auto;
  cursor: pointer;
}

.property-filter {
  display: flex;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.property-filter select,
.property-filter input {
  flex: 1;
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.remove-filter-btn {
  padding: var(--spacing-sm);
  background: var(--accent-danger);
  border: none;
  border-radius: var(--radius-sm);
  color: white;
  font-size: 1rem;
  cursor: pointer;
  transition: opacity var(--transition-fast);
}

.remove-filter-btn:hover {
  opacity: 0.8;
}

.add-filter-btn {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--accent-success);
  border: none;
  border-radius: var(--radius-md);
  color: white;
  font-size: 0.875rem;
  cursor: pointer;
  transition: opacity var(--transition-fast);
}

.add-filter-btn:hover {
  opacity: 0.8;
}

.filter-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--spacing-md);
}

/* Column Selector */
.column-selector {
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--bg-tertiary);
}

.column-selector h4 {
  margin: 0 0 var(--spacing-sm) 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
}

.column-options {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-sm);
}

.column-option {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background-color var(--transition-fast);
}

.column-option:hover {
  background: var(--bg-hover);
}

.column-option input[type="checkbox"] {
  width: auto;
  cursor: pointer;
}

/* Loading State */
.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: var(--spacing-md);
}

.spinner {
  width: 40px;
  height: 40px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Error State */
.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: var(--spacing-md);
}

/* Empty State */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: var(--spacing-sm);
  text-align: center;
}

/* Table View */
.table-view {
  flex: 1;
  overflow: auto;
}

.notes-table {
  width: 100%;
  border-collapse: collapse;
}

.notes-table thead {
  position: sticky;
  top: 0;
  background: var(--bg-secondary);
  z-index: 10;
}

.notes-table th {
  padding: var(--spacing-md);
  text-align: left;
  font-weight: 600;
  font-size: 0.875rem;
  color: var(--text-primary);
  border-bottom: 2px solid var(--bg-tertiary);
}

.notes-table th.sortable {
  cursor: pointer;
  transition: background-color var(--transition-fast);
}

.notes-table th.sortable:hover {
  background: var(--bg-tertiary);
}

.sort-indicator {
  margin-left: var(--spacing-xs);
  color: var(--accent-primary);
}

.notes-table tbody tr {
  border-bottom: 1px solid var(--bg-tertiary);
  transition: background-color var(--transition-fast);
}

.notes-table tbody tr:hover {
  background: var(--bg-hover);
}

.notes-table td {
  padding: var(--spacing-md);
  font-size: 0.875rem;
  color: var(--text-primary);
}

.title-cell {
  font-weight: 500;
}

.tags-cell,
.tags-container {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
}

.date-cell {
  white-space: nowrap;
}

.number-cell {
  text-align: right;
}

.property-cell {
  font-family: 'Fira Code', monospace;
  font-size: 0.8rem;
}

/* Card View */
.card-view {
  flex: 1;
  overflow: auto;
  padding: var(--spacing-md);
}

.cards-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--spacing-md);
}

.note-card {
  cursor: pointer;
  transition: transform var(--transition-fast);
}

.note-card:hover {
  transform: translateY(-2px);
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-sm);
}

.card-title {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card-tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
  margin-bottom: var(--spacing-sm);
}

.card-properties {
  margin-bottom: var(--spacing-sm);
}

.property-item {
  display: flex;
  gap: var(--spacing-xs);
  margin-bottom: var(--spacing-xs);
  font-size: 0.875rem;
}

.property-name {
  color: var(--text-secondary);
  font-weight: 500;
}

.property-value {
  color: var(--text-primary);
}

.card-footer {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--spacing-sm);
  padding-top: var(--spacing-sm);
  border-top: 1px solid var(--bg-tertiary);
}

.text-sm {
  font-size: 0.75rem;
}

/* Kanban View */
.kanban-view {
  flex: 1;
  overflow: auto;
  padding: var(--spacing-md);
}

.kanban-board {
  display: flex;
  gap: var(--spacing-md);
  height: 100%;
  min-height: 400px;
}

.kanban-column {
  flex: 1;
  min-width: 250px;
  max-width: 350px;
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.kanban-column-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  background: var(--bg-tertiary);
  border-bottom: 1px solid var(--bg-hover);
}

.kanban-column-header h4 {
  margin: 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
}

.kanban-count {
  background: var(--bg-hover);
  color: var(--text-secondary);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  font-weight: 500;
}

.kanban-column-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
}

.kanban-card {
  background: var(--bg-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  padding: var(--spacing-md);
  cursor: grab;
  transition: all var(--transition-fast);
}

.kanban-card:hover {
  border-color: var(--accent-primary);
  transform: translateY(-2px);
}

.kanban-card.dragging {
  opacity: 0.5;
  cursor: grabbing;
}

.kanban-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.kanban-card-title {
  font-size: 0.875rem;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kanban-card-tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-xs);
  margin-bottom: var(--spacing-sm);
}

.kanban-card-properties {
  margin-bottom: var(--spacing-sm);
}

/* Responsive Design */
@media (max-width: 768px) {
  .view-controls {
    flex-direction: column;
    align-items: stretch;
  }

  .view-controls-left,
  .view-controls-right {
    flex-direction: column;
    align-items: stretch;
  }

  .search-box {
    max-width: 100%;
  }

  .sort-controls {
    flex-direction: column;
  }

  .cards-grid {
    grid-template-columns: 1fr;
  }

  .kanban-board {
    flex-direction: column;
    overflow-x: auto;
  }

  .kanban-column {
    min-width: 100%;
    max-width: 100%;
  }
}
</style>
