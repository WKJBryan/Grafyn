<template>
  <div
    class="graph-settings"
    :class="{ collapsed: isCollapsed }"
  >
    <button
      class="toggle-btn"
      :title="isCollapsed ? 'Show Settings' : 'Hide Settings'"
      @click="isCollapsed = !isCollapsed"
    >
      <span v-if="isCollapsed">⚙</span>
      <span v-else>×</span>
    </button>
    
    <div
      v-show="!isCollapsed"
      class="settings-content"
    >
      <!-- Filters Section -->
      <div class="settings-section">
        <button
          class="section-header"
          @click="filtersOpen = !filtersOpen"
        >
          <span
            class="chevron"
            :class="{ open: filtersOpen }"
          >›</span>
          Filters
        </button>
        <div
          v-show="filtersOpen"
          class="section-body"
        >
          <div class="filter-group">
            <label class="checkbox-label">
              <input
                v-model="filters.showNotes"
                type="checkbox"
                @change="emitFilters"
              >
              <span class="pill note">Note</span>
            </label>
            <label class="checkbox-label">
              <input
                v-model="filters.showHubs"
                type="checkbox"
                @change="emitFilters"
              >
              <span class="pill hub">Hub</span>
            </label>
            <label class="checkbox-label">
              <input
                v-model="filters.showGeneral"
                type="checkbox"
                @change="emitFilters"
              >
              <span class="pill general">General</span>
            </label>
          </div>
          <div class="search-filter">
            <input 
              v-model="filters.search" 
              type="text" 
              placeholder="Filter by name..."
              class="search-input"
              @input="emitFilters"
            >
          </div>
        </div>
      </div>
      
      <!-- Groups Section -->
      <div class="settings-section">
        <button
          class="section-header"
          @click="groupsOpen = !groupsOpen"
        >
          <span
            class="chevron"
            :class="{ open: groupsOpen }"
          >›</span>
          Groups
        </button>
        <div
          v-show="groupsOpen"
          class="section-body"
        >
          <div class="groups-legend">
            <div
              v-for="(color, type) in groupColors"
              :key="type"
              class="legend-item"
            >
              <span
                class="color-dot"
                :style="{ background: color }"
              />
              <span class="legend-label">{{ type }}</span>
            </div>
          </div>
        </div>
      </div>
      
      <!-- Display Section -->
      <div class="settings-section">
        <button
          class="section-header"
          @click="displayOpen = !displayOpen"
        >
          <span
            class="chevron"
            :class="{ open: displayOpen }"
          >›</span>
          Display
        </button>
        <div
          v-show="displayOpen"
          class="section-body"
        >
          <div class="setting-row">
            <label>Arrows</label>
            <label class="toggle-switch">
              <input
                v-model="display.arrows"
                type="checkbox"
                @change="emitDisplay"
              >
              <span class="toggle-slider" />
            </label>
          </div>
          <div class="setting-row">
            <label>Text fade threshold</label>
            <input 
              v-model.number="display.textFade" 
              type="range" 
              min="0"
              max="100"
              step="5"
              class="slider"
              @input="emitDisplay"
            >
          </div>
          <div class="setting-row">
            <label>Node size</label>
            <input 
              v-model.number="display.nodeSize" 
              type="range" 
              min="1"
              max="30"
              step="1"
              class="slider"
              @input="emitDisplay"
            >
          </div>
          <div class="setting-row">
            <label>Link thickness</label>
            <input 
              v-model.number="display.linkThickness" 
              type="range" 
              min="0.5"
              max="5"
              step="0.5"
              class="slider"
              @input="emitDisplay"
            >
          </div>
          <button
            class="animate-btn"
            @click="$emit('animate')"
          >
            Animate
          </button>
        </div>
      </div>
      
      <!-- Forces Section -->
      <div class="settings-section">
        <button
          class="section-header"
          @click="forcesOpen = !forcesOpen"
        >
          <span
            class="chevron"
            :class="{ open: forcesOpen }"
          >›</span>
          Forces
        </button>
        <div
          v-show="forcesOpen"
          class="section-body"
        >
          <div class="setting-row">
            <label>Center force</label>
            <input 
              v-model.number="forces.center" 
              type="range" 
              min="0"
              max="1"
              step="0.05"
              class="slider"
              @input="emitForces"
            >
          </div>
          <div class="setting-row">
            <label>Repel force</label>
            <input 
              v-model.number="forces.repel" 
              type="range" 
              min="-1000"
              max="0"
              step="50"
              class="slider"
              @input="emitForces"
            >
          </div>
          <div class="setting-row">
            <label>Link force</label>
            <input 
              v-model.number="forces.link" 
              type="range" 
              min="0"
              max="2"
              step="0.1"
              class="slider"
              @input="emitForces"
            >
          </div>
          <div class="setting-row">
            <label>Link distance</label>
            <input 
              v-model.number="forces.distance" 
              type="range" 
              min="30"
              max="300"
              step="10"
              class="slider"
              @input="emitForces"
            >
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive } from 'vue'

const emit = defineEmits(['update:filters', 'update:display', 'update:forces', 'animate'])

const isCollapsed = ref(true)
const filtersOpen = ref(true)
const groupsOpen = ref(true)
const displayOpen = ref(false)
const forcesOpen = ref(false)

const filters = reactive({
  showNotes: true,
  showHubs: true,
  search: ''
})

const display = reactive({
  arrows: true,
  textFade: 50,
  nodeSize: 8,
  linkThickness: 1
})

const forces = reactive({
  center: 0.5,
  repel: -300,
  link: 1,
  distance: 100
})

const groupColors = {
  'Topic hub': '#f59e0b',
  'Note': '#6b7280'
}

function emitFilters() {
  emit('update:filters', { ...filters })
}

function emitDisplay() {
  emit('update:display', { ...display })
}

function emitForces() {
  emit('update:forces', { ...forces })
}

// Emit initial values
emitFilters()
emitDisplay()
emitForces()
</script>

<style scoped>
.graph-settings {
  position: absolute;
  top: 50px;
  right: 12px;
  width: 260px;
  background: var(--bg-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  z-index: 100;
  transition: width 0.2s ease, opacity 0.2s ease;
  max-height: min(400px, calc(100% - 180px));
  overflow-y: auto;
}

.graph-settings.collapsed {
  width: 44px;
  min-height: 44px;
  background: transparent;
  border: none;
  box-shadow: none;
  overflow: visible;
}

.toggle-btn {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 28px;
  height: 28px;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  transition: all 0.15s ease;
}

.toggle-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.settings-content {
  padding: 44px 12px 12px;
}

.settings-section {
  margin-bottom: 8px;
}

.section-header {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 0;
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: 0.8rem;
  font-weight: 600;
  cursor: pointer;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  position: relative;
  z-index: 1;
  min-height: 32px;
}

.section-header:hover {
  color: var(--text-primary);
}

.chevron {
  font-size: 12px;
  transition: transform 0.2s ease;
}

.chevron.open {
  transform: rotate(90deg);
}

.section-body {
  padding: 8px 0 8px 16px;
}

.filter-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 10px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 0.8rem;
}

.checkbox-label input {
  width: 14px;
  height: 14px;
  accent-color: var(--accent-primary);
}

.pill {
  padding: 2px 8px;
  border-radius: 12px;
  font-size: 0.7rem;
  font-weight: 500;
}

.pill.note { background: rgba(107, 114, 128, 0.2); color: #6b7280; }
.pill.hub { background: rgba(245, 158, 11, 0.2); color: #f59e0b; }

.search-input {
  width: 100%;
  padding: 6px 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8rem;
}

.search-input::placeholder {
  color: var(--text-muted);
}

.groups-legend {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.legend-item {
  display: flex;
  align-items: center;
  gap: 8px;
}

.color-dot {
  width: 12px;
  height: 12px;
  border-radius: 50%;
}

.legend-label {
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}

.setting-row label {
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.slider {
  width: 100px;
  height: 4px;
  accent-color: var(--accent-primary);
}

.toggle-switch {
  position: relative;
  width: 36px;
  height: 20px;
}

.toggle-switch input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  inset: 0;
  background: var(--bg-tertiary);
  border-radius: 20px;
  transition: 0.3s;
}

.toggle-slider::before {
  content: '';
  position: absolute;
  height: 14px;
  width: 14px;
  left: 3px;
  bottom: 3px;
  background: white;
  border-radius: 50%;
  transition: 0.3s;
}

.toggle-switch input:checked + .toggle-slider {
  background: var(--accent-primary);
}

.toggle-switch input:checked + .toggle-slider::before {
  transform: translateX(16px);
}

.animate-btn {
  width: 100%;
  padding: 8px 16px;
  background: var(--accent-primary);
  border: none;
  border-radius: var(--radius-md);
  color: white;
  font-weight: 600;
  font-size: 0.85rem;
  cursor: pointer;
  transition: all 0.15s ease;
}

.animate-btn:hover {
  filter: brightness(1.1);
  transform: translateY(-1px);
}

/* Scrollbar styling */
.graph-settings::-webkit-scrollbar {
  width: 6px;
}

.graph-settings::-webkit-scrollbar-track {
  background: transparent;
}

.graph-settings::-webkit-scrollbar-thumb {
  background: var(--bg-tertiary);
  border-radius: 3px;
}
</style>
