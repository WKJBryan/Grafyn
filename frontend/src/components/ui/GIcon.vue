<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    class="g-icon"
    :class="iconClass"
  >
    <template
      v-for="(el, i) in elements"
      :key="i"
    >
      <path
        v-if="el.type === 'path'"
        :d="el.d"
      />
      <circle
        v-else-if="el.type === 'circle'"
        :cx="el.cx"
        :cy="el.cy"
        :r="el.r"
      />
      <line
        v-else-if="el.type === 'line'"
        :x1="el.x1"
        :y1="el.y1"
        :x2="el.x2"
        :y2="el.y2"
      />
      <rect
        v-else-if="el.type === 'rect'"
        :x="el.x"
        :y="el.y"
        :width="el.width"
        :height="el.height"
        :rx="el.rx"
      />
      <polyline
        v-else-if="el.type === 'polyline'"
        :points="el.points"
      />
      <polygon
        v-else-if="el.type === 'polygon'"
        :points="el.points"
      />
    </template>
  </svg>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  name: {
    type: String,
    required: true
  },
  size: {
    type: [Number, String],
    default: 16
  },
  iconClass: {
    type: String,
    default: ''
  }
})

// Lucide icon definitions — each icon is an array of SVG elements
const ICONS = {
  'message-square': [
    { type: 'path', d: 'M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z' }
  ],
  'git-branch': [
    { type: 'line', x1: 6, y1: 3, x2: 6, y2: 15 },
    { type: 'circle', cx: 18, cy: 6, r: 3 },
    { type: 'circle', cx: 6, cy: 18, r: 3 },
    { type: 'path', d: 'M18 9a9 9 0 0 1-9 9' }
  ],
  'swords': [
    { type: 'path', d: 'M14.5 17.5 3 6V3h3l11.5 11.5' },
    { type: 'path', d: 'M13 19l6-6' },
    { type: 'path', d: 'M16 16l4 4' },
    { type: 'path', d: 'M19 21l2-2' },
    { type: 'path', d: 'M9.5 6.5 21 18v3h-3L6.5 9.5' },
    { type: 'path', d: 'M11 5l-6 6' },
    { type: 'path', d: 'M8 8 4 4' },
    { type: 'path', d: 'M5 3 3 5' }
  ],
  'layout-grid': [
    { type: 'rect', x: 3, y: 3, width: 7, height: 7, rx: 1 },
    { type: 'rect', x: 14, y: 3, width: 7, height: 7, rx: 1 },
    { type: 'rect', x: 3, y: 14, width: 7, height: 7, rx: 1 },
    { type: 'rect', x: 14, y: 14, width: 7, height: 7, rx: 1 }
  ],
  'chevron-down': [
    { type: 'path', d: 'M6 9l6 6 6-6' }
  ],
  'chevron-up': [
    { type: 'path', d: 'M18 15l-6-6-6 6' }
  ],
  'chevron-right': [
    { type: 'path', d: 'M9 18l6-6-6-6' }
  ],
  'tree-pine': [
    { type: 'path', d: 'M17 14l3 3.3a1 1 0 0 1-.7 1.7H4.7a1 1 0 0 1-.7-1.7L7 14' },
    { type: 'path', d: 'M17 9l2.3 2.3a1 1 0 0 1-.7 1.7H5.4a1 1 0 0 1-.7-1.7L7 9' },
    { type: 'path', d: 'M15.3 4.3a1 1 0 0 0-.6-.3H9.3a1 1 0 0 0-.6.3L6 7h12l-2.7-2.7z' },
    { type: 'line', x1: 12, y1: 19, x2: 12, y2: 22 }
  ],
  'orbit': [
    { type: 'circle', cx: 12, cy: 12, r: 3 },
    { type: 'circle', cx: 19, cy: 5, r: 2 },
    { type: 'circle', cx: 5, cy: 19, r: 2 },
    { type: 'path', d: 'M10.4 21.9a10 10 0 0 0 9.941-15.416' },
    { type: 'path', d: 'M13.5 2.1a10 10 0 0 0-9.841 15.416' }
  ],
  'circle': [
    { type: 'circle', cx: 12, cy: 12, r: 10 }
  ],
  'pin': [
    { type: 'line', x1: 12, y1: 17, x2: 12, y2: 22 },
    { type: 'path', d: 'M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z' }
  ],
  'globe': [
    { type: 'circle', cx: 12, cy: 12, r: 10 },
    { type: 'path', d: 'M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20' },
    { type: 'path', d: 'M2 12h20' }
  ],
  'check': [
    { type: 'path', d: 'M20 6L9 17l-5-5' }
  ],
  'check-circle': [
    { type: 'path', d: 'M22 11.08V12a10 10 0 1 1-5.93-9.14' },
    { type: 'path', d: 'M22 4L12 14.01l-3-3' }
  ],
  'alert-circle': [
    { type: 'circle', cx: 12, cy: 12, r: 10 },
    { type: 'line', x1: 12, y1: 8, x2: 12, y2: 12 },
    { type: 'line', x1: 12, y1: 16, x2: 12.01, y2: 16 }
  ],
  'alert-triangle': [
    { type: 'path', d: 'M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z' },
    { type: 'line', x1: 12, y1: 9, x2: 12, y2: 13 },
    { type: 'line', x1: 12, y1: 17, x2: 12.01, y2: 17 }
  ],
  'loader': [
    { type: 'line', x1: 12, y1: 2, x2: 12, y2: 6 },
    { type: 'line', x1: 12, y1: 18, x2: 12, y2: 22 },
    { type: 'line', x1: 4.93, y1: 4.93, x2: 7.76, y2: 7.76 },
    { type: 'line', x1: 16.24, y1: 16.24, x2: 19.07, y2: 19.07 },
    { type: 'line', x1: 2, y1: 12, x2: 6, y2: 12 },
    { type: 'line', x1: 18, y1: 12, x2: 22, y2: 12 },
    { type: 'line', x1: 4.93, y1: 19.07, x2: 7.76, y2: 16.24 },
    { type: 'line', x1: 16.24, y1: 7.76, x2: 19.07, y2: 4.93 }
  ],
  'refresh-cw': [
    { type: 'polyline', points: '23 4 23 10 17 10' },
    { type: 'polyline', points: '1 20 1 14 7 14' },
    { type: 'path', d: 'M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15' }
  ],
  'x': [
    { type: 'line', x1: 18, y1: 6, x2: 6, y2: 18 },
    { type: 'line', x1: 6, y1: 6, x2: 18, y2: 18 }
  ],
  'plus': [
    { type: 'line', x1: 12, y1: 5, x2: 12, y2: 19 },
    { type: 'line', x1: 5, y1: 12, x2: 19, y2: 12 }
  ],
  'save': [
    { type: 'path', d: 'M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z' },
    { type: 'polyline', points: '17 21 17 13 7 13 7 21' },
    { type: 'polyline', points: '7 3 7 8 15 8' }
  ],
  'maximize-2': [
    { type: 'polyline', points: '15 3 21 3 21 9' },
    { type: 'polyline', points: '9 21 3 21 3 15' },
    { type: 'line', x1: 21, y1: 3, x2: 14, y2: 10 },
    { type: 'line', x1: 3, y1: 21, x2: 10, y2: 14 }
  ],
  'zoom-in': [
    { type: 'circle', cx: 11, cy: 11, r: 8 },
    { type: 'line', x1: 21, y1: 21, x2: 16.65, y2: 16.65 },
    { type: 'line', x1: 11, y1: 8, x2: 11, y2: 14 },
    { type: 'line', x1: 8, y1: 11, x2: 14, y2: 11 }
  ],
  'zoom-out': [
    { type: 'circle', cx: 11, cy: 11, r: 8 },
    { type: 'line', x1: 21, y1: 21, x2: 16.65, y2: 16.65 },
    { type: 'line', x1: 8, y1: 11, x2: 14, y2: 11 }
  ],
  'search': [
    { type: 'circle', cx: 11, cy: 11, r: 8 },
    { type: 'line', x1: 21, y1: 21, x2: 16.65, y2: 16.65 }
  ],
  'settings': [
    { type: 'circle', cx: 12, cy: 12, r: 3 },
    { type: 'path', d: 'M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z' }
  ],
  'moon': [
    { type: 'path', d: 'M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z' }
  ],
  'sun': [
    { type: 'circle', cx: 12, cy: 12, r: 5 },
    { type: 'line', x1: 12, y1: 1, x2: 12, y2: 3 },
    { type: 'line', x1: 12, y1: 21, x2: 12, y2: 23 },
    { type: 'line', x1: 4.22, y1: 4.22, x2: 5.64, y2: 5.64 },
    { type: 'line', x1: 18.36, y1: 18.36, x2: 19.78, y2: 19.78 },
    { type: 'line', x1: 1, y1: 12, x2: 3, y2: 12 },
    { type: 'line', x1: 21, y1: 12, x2: 23, y2: 12 },
    { type: 'line', x1: 4.22, y1: 19.78, x2: 5.64, y2: 18.36 },
    { type: 'line', x1: 18.36, y1: 5.64, x2: 19.78, y2: 4.22 }
  ],
  'help-circle': [
    { type: 'circle', cx: 12, cy: 12, r: 10 },
    { type: 'path', d: 'M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3' },
    { type: 'line', x1: 12, y1: 17, x2: 12.01, y2: 17 }
  ],
  'star': [
    { type: 'polygon', points: '12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2' }
  ],
  'link': [
    { type: 'path', d: 'M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71' },
    { type: 'path', d: 'M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71' }
  ],
  'minus': [
    { type: 'line', x1: 5, y1: 12, x2: 19, y2: 12 }
  ],
  'grafyn': [
    // Network nodes (upper-right)
    { type: 'circle', cx: 16, cy: 4, r: 1.5 },
    { type: 'circle', cx: 21, cy: 7, r: 1.2 },
    { type: 'circle', cx: 17, cy: 10, r: 1.5 },
    // Network edges
    { type: 'line', x1: 16, y1: 4, x2: 21, y2: 7 },
    { type: 'line', x1: 16, y1: 4, x2: 17, y2: 10 },
    { type: 'line', x1: 21, y1: 7, x2: 17, y2: 10 },
    // Beam connecting network to crystal
    { type: 'line', x1: 17, y1: 10, x2: 10, y2: 13 },
    // Crystal gem (lower-left)
    { type: 'polygon', points: '10,13 13,16 11,20 7,21 3,18 5,14' },
    // Crystal facet lines from center
    { type: 'line', x1: 8, y1: 17, x2: 10, y2: 13 },
    { type: 'line', x1: 8, y1: 17, x2: 11, y2: 20 },
    { type: 'line', x1: 8, y1: 17, x2: 3, y2: 18 }
  ]
}

const elements = computed(() => {
  return ICONS[props.name] || [
    // Fallback: question mark in circle
    { type: 'circle', cx: 12, cy: 12, r: 10 },
    { type: 'path', d: 'M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3' },
    { type: 'line', x1: 12, y1: 17, x2: 12.01, y2: 17 }
  ]
})
</script>
