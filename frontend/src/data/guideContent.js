/**
 * Structured tutorial content for the Grafyn guide system.
 * Each category groups related steps; each step can optionally anchor
 * to a DOM element via a `[data-guide="..."]` selector.
 */

export const guideCategories = [
  {
    id: 'notes',
    title: 'Notes',
    icon: '\u{1F4DD}',
    route: '/',
    steps: [
      {
        id: 'notes-create',
        title: 'Create a Note',
        content: 'Click "+ New Note" to create a note. Choose a type (atomic, hub, or evidence) and optionally assign a topic tag.',
        anchor: '[data-guide="new-note-btn"]',
        sinceVersion: '0.1.0',
      },
      {
        id: 'notes-status',
        title: 'Note Status Workflow',
        content: 'Notes progress through statuses: Draft, Evidence, and Canonical. Change the status in the editor footer to reflect maturity.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
      {
        id: 'notes-wikilinks',
        title: 'Wikilinks',
        content: 'Link notes together with [[Note Title]] syntax. Use [[Title|Display]] for custom link text. Links appear in the knowledge graph.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
      {
        id: 'notes-tags',
        title: 'Tags & Frontmatter',
        content: 'Add comma-separated tags in the editor footer. Tags power the sidebar filter and are stored in YAML frontmatter.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'graph',
    title: 'Knowledge Graph',
    icon: '\u{1F578}\u{FE0F}',
    route: '/',
    steps: [
      {
        id: 'graph-interact',
        title: 'Explore the Graph',
        content: 'The main graph shows all notes and their wikilink connections. Click any node to open the note. Drag to rearrange, scroll to zoom.',
        anchor: '[data-guide="graph-view"]',
        sinceVersion: '0.1.0',
      },
      {
        id: 'graph-mini',
        title: 'Mini Graph',
        content: 'The right sidebar shows a focused graph centered on the selected note and its immediate neighbors.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'search',
    title: 'Search',
    icon: '\u{1F50D}',
    route: '/',
    steps: [
      {
        id: 'search-bar',
        title: 'Full-Text Search',
        content: 'Use the search bar to find notes by title or content. Results are ranked using full-text search with graph-aware similarity.',
        anchor: '[data-guide="search-bar"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'sidebar',
    title: 'Sidebar Panels',
    icon: '\u{1F4CB}',
    route: '/',
    steps: [
      {
        id: 'sidebar-nav',
        title: 'Tree Navigation',
        content: 'The left sidebar shows all notes in a tree view, grouped by tags. Click to select, filter by tags below.',
        anchor: '[data-guide="sidebar-left"]',
        sinceVersion: '0.1.0',
      },
      {
        id: 'sidebar-backlinks',
        title: 'Backlinks & Mentions',
        content: 'Select a note to see its backlinks (notes that link to it) and unlinked mentions (notes that mention its title but don\'t link) in the right sidebar.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'canvas',
    title: 'Multi-LLM Canvas',
    icon: '\u{1F3A8}',
    route: '/canvas',
    steps: [
      {
        id: 'canvas-create',
        title: 'Create a Canvas',
        content: 'Click "+ New" in the sidebar to start a canvas session. Each canvas lets you compare responses from multiple AI models.',
        anchor: '[data-guide="canvas-new-btn"]',
        sinceVersion: '0.1.0',
      },
      {
        id: 'canvas-prompt',
        title: 'Send a Prompt',
        content: 'Start with + New Prompt, then type your prompt and choose one or more models. Responses stream onto the canvas in real time.',
        anchor: '[data-guide="canvas-prompt-btn"]',
        sinceVersion: '0.1.0',
      },
      {
        id: 'canvas-context',
        title: 'Semantic Note Context',
        content: 'Canvas automatically retrieves relevant notes from your vault and includes them as context for AI responses. Pin specific notes for guaranteed inclusion.',
        anchor: '[data-guide="pinned-notes-btn"]',
        sinceVersion: '0.1.1',
      },
      {
        id: 'canvas-debate',
        title: 'Model Debate',
        content: 'Start a debate to have models critique and respond to each other\'s answers, deepening the analysis.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
      {
        id: 'canvas-export',
        title: 'Save as Note',
        content: 'Export the canvas conversation as a note in your vault, preserving the multi-model discussion.',
        anchor: '[data-guide="canvas-save-btn"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'distill',
    title: 'Distillation',
    icon: '\u{2697}\u{FE0F}',
    route: '/',
    steps: [
      {
        id: 'distill-overview',
        title: 'Container to Atomic Notes',
        content: 'Distillation splits large "container" notes (evidence status) into focused "atomic" notes. Hub notes are auto-created for frequent tags.',
        anchor: null,
        sinceVersion: '0.1.0',
      },
      {
        id: 'distill-button',
        title: 'Distill a Note',
        content: 'Open an evidence-status note and click the Distill button. Choose Algorithm (fast, rule-based) or LLM (AI-powered extraction).',
        anchor: '[data-guide="distill-btn"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'links',
    title: 'Link Discovery',
    icon: '\u{1F517}',
    route: '/',
    steps: [
      {
        id: 'links-discover',
        title: 'Discover Links',
        content: 'Click "Discover Links" on any saved note to find potential connections. Uses semantic similarity and optional LLM analysis.',
        anchor: '[data-guide="discover-links-btn"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'import',
    title: 'Import Conversations',
    icon: '\u{1F4E5}',
    route: '/import',
    steps: [
      {
        id: 'import-file',
        title: 'Import from AI Platforms',
        content: 'Import conversations from ChatGPT, Claude, Grok, or Gemini. Select a JSON export file to preview and choose which conversations to import as evidence notes.',
        anchor: '[data-guide="import-file-btn"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
  {
    id: 'settings',
    title: 'Settings',
    icon: '\u{2699}\u{FE0F}',
    route: '/',
    steps: [
      {
        id: 'settings-open',
        title: 'App Settings',
        content: 'Open Settings to configure your vault path, OpenRouter API key, LLM model, theme, and MCP integration for Claude Desktop.',
        anchor: '[data-guide="settings-btn"]',
        sinceVersion: '0.1.0',
      },
    ],
  },
]

/** Flat list of all steps for easy lookup */
export const allSteps = guideCategories.flatMap(cat =>
  cat.steps.map(step => ({ ...step, categoryId: cat.id, route: cat.route }))
)

/**
 * Simple semver comparison: returns true if a > b.
 * Only handles major.minor.patch (no pre-release).
 */
function semverGt(a, b) {
  const pa = a.split('.').map(Number)
  const pb = b.split('.').map(Number)
  for (let i = 0; i < 3; i++) {
    if ((pa[i] || 0) > (pb[i] || 0)) return true
    if ((pa[i] || 0) < (pb[i] || 0)) return false
  }
  return false
}

/**
 * Returns steps added after a given version.
 * @param {string} sinceVersion - e.g. "0.1.0"
 * @returns {Array} steps with sinceVersion > sinceVersion param
 */
export function getNewSteps(sinceVersion) {
  if (!sinceVersion) return allSteps
  return allSteps.filter(step => semverGt(step.sinceVersion, sinceVersion))
}
