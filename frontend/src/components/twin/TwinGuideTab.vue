<template>
  <section class="tab-panel guide-panel">
    <div class="panel-header">
      <div>
        <h2>How To Use</h2>
        <span>Button guide and short task walkthroughs</span>
      </div>
    </div>
    <section class="workspace-band guide-walkthrough">
      <h3>Fast Decision Session</h3>
      <ol>
        <li>Open Canvas and click <strong>+ New</strong>.</li>
        <li>Choose <strong>Decision</strong>, then write or paste the decision.</li>
        <li>Add options, stakes, leaning, and follow-up only when useful.</li>
        <li>Click <strong>Create Reflection Card</strong>.</li>
        <li>Use one feedback button if the card is useful or wrong.</li>
      </ol>
    </section>
    <section
      v-for="group in tutorialButtonGroups"
      :key="group.title"
      class="workspace-band guide-group"
    >
      <h3>{{ group.title }}</h3>
      <dl>
        <template
          v-for="item in group.items"
          :key="item.name"
        >
          <dt>{{ item.name }}</dt>
          <dd>{{ item.description }}</dd>
        </template>
      </dl>
    </section>
  </section>
</template>

<script setup>
// Extracted verbatim from TwinReviewView.vue (Task 4.3). Pure static content —
// no store dependency.
const tutorialButtonGroups = [
  {
    title: 'Canvas Buttons',
    items: [
      { name: '+ New', description: 'Create a normal prompt or Decision Mirror session.' },
      { name: 'Decision', description: 'Switches the prompt into Decision Mirror mode.' },
      { name: 'Options', description: 'One option per line, useful for comparisons.' },
      { name: 'Stakes', description: 'What changes if the decision goes right or wrong.' },
      { name: 'Initial Leaning', description: 'What the user currently thinks they may choose.' },
      { name: 'Follow-up', description: 'Optional date for outcome review.' },
      { name: 'Create Reflection Card', description: 'Generates the Decision Mirror answer.' },
      { name: 'Useful', description: 'Marks the reflection as useful evidence.' },
      { name: 'Not Me', description: 'Records that the mirror misrepresented the user.' },
      { name: 'Save Insight', description: 'Saves the reflection as a reasoning-pattern insight.' },
      { name: 'Reject Pattern', description: 'Rejects the inferred pattern behind the reflection.' },
      { name: 'Open in Twin', description: 'Jumps to the decision record in Twin Workspace.' }
    ]
  },
  {
    title: 'Twin Workspace Buttons',
    items: [
      { name: 'Run Records', description: 'Infer candidate user records from Canvas traces and feedback.' },
      { name: 'Run Constitution', description: 'Infer Constitution items and Action Gaps from records and decisions.' },
      { name: 'Overview', description: 'Shows health summary, recent decisions, action gaps, and pending review.' },
      { name: 'Constitution', description: 'Review values, tastes, constraints, somatic cues, and reasoning principles.' },
      { name: 'Action Gaps', description: 'Review gaps between stated intent and revealed behavior.' },
      { name: 'Decisions', description: 'Review Reflection Cards and record outcomes.' },
      { name: 'Memory Review', description: 'Review clustered memory digest items.' },
      { name: 'Setup', description: 'Progressively seed values, tastes, constraints, somatic cues, and action tendencies.' },
      { name: 'Keep', description: 'Approve a memory or constitution item for future context.' },
      { name: 'Soften', description: 'Keep it as tentative or lower confidence.' },
      { name: 'Not Me', description: 'Mark it as inaccurate.' },
      { name: 'Private', description: 'Exclude it from live twin context.' },
      { name: 'No Train', description: 'Preserve locally but exclude from model, training, and export use.' },
      { name: 'Reject', description: 'Remove it from future twin context.' },
      { name: 'Save Setup', description: 'Turns setup entries into evidence-backed Constitution seed items.' }
    ]
  },
  {
    title: 'Config Buttons',
    items: [
      { name: 'Preset', description: 'Choose Balanced, Stricter Evidence, Find Blind Spots, or Push Next Action.' },
      { name: 'Advanced', description: 'Reveals scoring and retrieval sliders.' },
      { name: 'Reset', description: 'Restores default Decision Mirror settings.' },
      { name: 'Export Benchmark', description: 'Exports decision episodes, Reflection Cards, scores, evidence packets, and outcomes.' }
    ]
  }
]
</script>
