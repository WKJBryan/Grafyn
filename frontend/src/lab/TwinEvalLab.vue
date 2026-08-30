<template>
  <div class="lab-shell">
    <header class="lab-header">
      <div>
        <h1>Grafyn Twin Eval Lab</h1>
        <p>Compare base, instruction-tuned, and SEA-LION models using retrieval + constitution context.</p>
      </div>
      <button
        class="btn secondary"
        :disabled="loadingMatrix"
        @click="loadModelMatrix"
      >
        {{ loadingMatrix ? 'Refreshing...' : 'Refresh Models' }}
      </button>
    </header>

    <main class="lab-main">
      <!-- Input panel -->
      <section class="panel input-panel">
        <div class="panel-title">
          <h2>Question</h2>
          <span>Paste MCQ or freeform scenario</span>
        </div>

        <textarea
          v-model="rawQuestion"
          data-test="lab-question"
          class="question-input"
          spellcheck="false"
          placeholder="What is the most accurate description?&#10;&#10;A) ...&#10;B) ...&#10;C) ..."
        />

        <div class="control-row">
          <label>
            Selected context mode
            <select
              v-model="contextMode"
              data-test="context-mode"
            >
              <option value="system_only">System only</option>
              <option value="retrieval_only">Retrieval only</option>
              <option value="constitution_only">Constitution only</option>
              <option value="retrieval_and_constitution">Retrieval and constitution</option>
            </select>
          </label>
          <label>
            Answer key (optional)
            <input
              v-model="answerKey"
              class="short-input"
              maxlength="4"
              placeholder="A"
            >
          </label>
          <label>
            Temperature
            <input
              v-model.number="temperature"
              type="number"
              min="0"
              max="2"
              step="0.05"
              class="short-input"
            >
          </label>
          <label>
            Top-p
            <input
              v-model.number="topP"
              type="number"
              min="0"
              max="1"
              step="0.05"
              class="short-input"
            >
          </label>
          <label>
            <input
              v-model="showReasoningTrace"
              data-test="show-trace"
              type="checkbox"
            >
            Show reasoning trace
          </label>
          <label>
            <input
              v-model="structuredOutput"
              data-test="structured-output"
              type="checkbox"
            >
            Structured JSON output
          </label>
        </div>

        <label>
          System prompt
          <textarea
            v-model="systemPrompt"
            data-test="system-prompt"
            :disabled="contextMode !== 'system_only'"
            class="system-prompt-input"
          />
        </label>

        <div class="actions">
          <button
            data-test="preview-input"
            class="btn secondary"
            :disabled="!rawQuestion.trim()"
            @click="previewInput"
          >
            Preview Input
          </button>
          <button
            data-test="preview-context"
            class="btn secondary"
            :disabled="!rawQuestion.trim() || previewingContext"
            @click="previewContext"
          >
            {{ previewingContext ? 'Previewing...' : 'Preview Context' }}
          </button>
          <button
            data-test="run-lab"
            class="btn primary"
            :disabled="!canRun"
            @click="runLab"
          >
            {{ running ? 'Running...' : 'Run Models' }}
          </button>
        </div>
        <div
          v-if="inputPreview"
          class="input-preview"
        >
          <p
            v-for="option in inputPreview.options"
            :key="option.key"
          >
            {{ option.key }}. {{ option.text }}
          </p>
          <p>{{ inputPreview.answer_key ? `Answer key: ${inputPreview.answer_key}` : 'No answer key' }}</p>
        </div>
        <p
          v-if="errorMessage"
          class="error"
        >
          {{ errorMessage }}
        </p>
      </section>

      <!-- Models panel -->
      <section class="panel models-panel">
        <div class="panel-title">
          <h2>Models</h2>
          <span>{{ selectedModelKeys.length }} selected</span>
        </div>
        <div class="model-list">
          <label
            v-for="model in modelMatrix"
            :key="model.key"
            class="model-row"
            :class="{ blocked: !model.runner_ready }"
          >
            <input
              v-model="selectedModelKeys"
              type="checkbox"
              :value="model.key"
              :disabled="!model.runner_ready"
            >
            <span>
              <strong>{{ model.label }}</strong>
              <small>{{ model.quantization }} · {{ model.runner_model_id }}</small>
            </span>
            <em>{{ model.runner_ready ? 'ready' : 'not installed' }}</em>
          </label>
        </div>
      </section>

      <!-- Context preview panel -->
      <section
        v-if="contextPacket"
        class="panel context-panel"
      >
        <div class="panel-title">
          <h2>Context Preview</h2>
          <span>{{ contextPacket.private_store_accessed ? 'Private store accessed' : 'No private store access' }}</span>
        </div>
        <div
          v-for="group in contextGroups"
          :key="group.label"
          class="context-group"
        >
          <h3>{{ group.label }} ({{ group.items.length }})</h3>
          <p
            v-if="!group.items.length"
            class="muted"
          >
            None retrieved.
          </p>
          <ul v-else>
            <li
              v-for="item in group.items"
              :key="`${item.source_type}-${item.id}`"
            >
              <strong>{{ item.label }}</strong>
              <span class="muted">{{ item.snippet }}</span>
            </li>
          </ul>
        </div>
      </section>

      <!-- Results area — grouped by question run -->
      <section
        v-if="allRuns.length"
        class="panel results-panel"
      >
        <div class="panel-title">
          <h2>Results</h2>
          <div class="actions">
            <button
              data-test="export-json"
              class="btn secondary"
              @click="downloadExport('json')"
            >
              Export JSON
            </button>
            <button
              class="btn secondary"
              @click="downloadExport('csv')"
            >
              Export CSV
            </button>
            <button
              class="btn danger"
              @click="clearAll"
            >
              Clear All
            </button>
          </div>
        </div>

        <div
          v-for="(run, runIdx) in allRuns"
          :key="run.question.id"
          class="run-block"
        >
          <div class="run-header">
            <span class="run-label">Q{{ runIdx + 1 }}</span>
            <p class="run-question">
              {{ run.question.question }}
            </p>
            <span
              v-if="run.question.answer_key"
              class="answer-badge"
            >Key: {{ run.question.answer_key }}</span>
            <span
              v-if="run.pendingKeys.size"
              class="running-badge"
            >{{ run.pendingKeys.size }} running…</span>
          </div>

          <table class="results-table">
            <thead>
              <tr>
                <th class="col-model">
                  Model
                </th>
                <th class="col-stage">
                  Stage
                </th>
                <th class="col-region">
                  Region
                </th>
                <th class="col-pick">
                  Pick
                </th>
                <th class="col-score">
                  Score
                </th>
                <th class="col-response">
                  Response
                </th>
              </tr>
            </thead>
            <tbody>
              <!-- Pending rows shown while a model is still generating -->
              <tr
                v-for="key in [...run.pendingKeys]"
                :key="`pending-${key}`"
                class="row-pending"
              >
                <td class="col-model">
                  <strong>{{ modelLabel(key) }}</strong>
                </td>
                <td class="col-stage muted">
                  {{ modelMeta(key, 'training_stage') }}
                </td>
                <td class="col-region muted">
                  {{ modelMeta(key, 'regional_context') }}
                </td>
                <td class="col-pick">
                  <span class="spinner" />
                </td>
                <td class="col-score muted">
                  —
                </td>
                <td class="col-response muted generating-text">
                  Generating…
                </td>
              </tr>
              <template
                v-for="result in run.results"
                :key="result.model_key"
              >
                <tr :class="{ 'row-error': !!result.error }">
                  <td class="col-model">
                    <strong>{{ modelLabel(result.model_key) }}</strong>
                  </td>
                  <td class="col-stage muted">
                    {{ modelMeta(result.model_key, 'training_stage') }}
                  </td>
                  <td class="col-region muted">
                    {{ modelMeta(result.model_key, 'regional_context') }}
                  </td>
                  <td class="col-pick">
                    <span
                      v-if="result.error"
                      class="tag error-tag"
                    >err</span>
                    <span
                      v-else-if="result.selected_option"
                      class="tag option-tag"
                    >{{ result.selected_option }}</span>
                    <span
                      v-else
                      class="tag outside-tag"
                    >free</span>
                  </td>
                  <td class="col-score">
                    <span
                      v-if="result.correctness_score === 1"
                      class="tag correct-tag"
                    >✓</span>
                    <span
                      v-else-if="result.correctness_score === 0"
                      class="tag wrong-tag"
                    >✗</span>
                    <span
                      v-else
                      class="muted"
                    >—</span>
                  </td>
                  <td class="col-response">
                    <p
                      v-if="result.error"
                      class="result-error"
                    >
                      {{ result.error }}
                    </p>
                    <p
                      v-else
                      class="result-answer"
                    >
                      {{ result.final_answer || result.outside_options_answer || '(no answer)' }}
                    </p>
                    <button
                      v-if="result.model_trace"
                      class="trace-toggle"
                      @click="toggleTrace(runIdx, result.model_key)"
                    >
                      {{ isTraceOpen(runIdx, result.model_key) ? '▲ Hide thinking' : '▼ Show thinking' }}
                    </button>
                    <span
                      v-else-if="result.sycophancy_flag"
                      class="tag syco-tag"
                    >sycophancy detected</span>
                  </td>
                </tr>
                <tr
                  v-if="result.model_trace && (showReasoningTrace || isTraceOpen(runIdx, result.model_key))"
                  class="trace-row"
                >
                  <td colspan="6">
                    <pre class="trace-block">{{ result.model_trace }}</pre>
                  </td>
                </tr>
              </template>
            </tbody>
          </table>

          <p
            v-if="run.skipped.length"
            class="muted skipped-note"
          >
            Skipped: {{ run.skipped.map(s => s.key + ' (' + s.reason + ')').join(', ') }}
          </p>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { twinEval } from './api'

const rawQuestion = ref('')
const answerKey = ref('')
const temperature = ref(0.2)
const topP = ref(0.95)
const contextMode = ref('system_only')
const systemPrompt = ref('')
const showReasoningTrace = ref(false)
const structuredOutput = ref(false)
const modelMatrix = ref([])
const selectedModelKeys = ref([])
const inputPreview = ref(null)
const contextPacket = ref(null)
const allRuns = ref([])
const expandedTraces = ref(new Set())
const errorMessage = ref('')
const loadingMatrix = ref(false)
const previewingContext = ref(false)
const running = ref(false)

const canRun = computed(
  () => rawQuestion.value.trim().length > 0 && selectedModelKeys.value.length > 0 && !running.value
)

const contextGroups = computed(() => {
  if (!contextPacket.value) return []
  return [
    { label: 'Retrieval notes', items: contextPacket.value.retrieval_items || [] },
    { label: 'Constitution items', items: contextPacket.value.constitution_items || [] },
    { label: 'Action gaps', items: contextPacket.value.action_gaps || [] }
  ]
})

onMounted(() => {
  void loadModelMatrix()
})

async function loadModelMatrix() {
  loadingMatrix.value = true
  errorMessage.value = ''
  try {
    modelMatrix.value = await twinEval.getModelMatrix()
    selectedModelKeys.value = modelMatrix.value
      .filter(model => model.runner_ready)
      .map(model => model.key)
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    loadingMatrix.value = false
  }
}

async function previewContext() {
  previewingContext.value = true
  errorMessage.value = ''
  try {
    contextPacket.value = await twinEval.previewContext(buildRequest())
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    previewingContext.value = false
  }
}

async function previewInput() {
  errorMessage.value = ''
  try {
    inputPreview.value = await twinEval.previewInput(
      rawQuestion.value,
      answerKey.value.trim().toUpperCase() || null
    )
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function runLab() {
  running.value = true
  errorMessage.value = ''
  try {
    const report = await twinEval.runLab(buildRequest())
    contextPacket.value = report.context_packet
    allRuns.value.push({
      question: report.question,
      context_packet: report.context_packet,
      results: report.results,
      skipped: report.skipped_models || [],
      pendingKeys: new Set()
    })
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    running.value = false
  }
}

async function downloadExport(format) {
  const allResults = allRuns.value.flatMap(run => run.results)
  const payload = await twinEval.exportResults(allResults)
  if (typeof URL === 'undefined' || typeof URL.createObjectURL !== 'function') return
  const content = format === 'csv' ? payload.csv : payload.json
  const filename = format === 'csv' ? 'twin-eval-results.csv' : 'twin-eval-results.json'
  const blob = new Blob([content], { type: format === 'csv' ? 'text/csv' : 'application/json' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  URL.revokeObjectURL(url)
}

function clearAll() {
  allRuns.value = []
  expandedTraces.value = new Set()
}

function buildRequest() {
  return {
    raw_question: rawQuestion.value,
    answer_key: answerKey.value.trim().toUpperCase() || null,
    model_keys: selectedModelKeys.value,
    context_mode: contextMode.value,
    temperature: temperature.value,
    top_p: topP.value,
    system_prompt: contextMode.value === 'system_only' ? systemPrompt.value || null : null,
    structured_output: structuredOutput.value,
    show_reasoning_trace: showReasoningTrace.value
  }
}

function toggleTrace(runIdx, modelKey) {
  const k = `${runIdx}-${modelKey}`
  const next = new Set(expandedTraces.value)
  if (next.has(k)) {
    next.delete(k)
  } else {
    next.add(k)
  }
  expandedTraces.value = next
}

function isTraceOpen(runIdx, modelKey) {
  return expandedTraces.value.has(`${runIdx}-${modelKey}`)
}

function modelLabel(modelKey) {
  return modelMatrix.value.find(m => m.key === modelKey)?.label || modelKey
}

function modelMeta(modelKey, field) {
  return modelMatrix.value.find(m => m.key === modelKey)?.[field] || ''
}
</script>

<style scoped>
.lab-shell {
  height: 100vh;
  overflow-y: auto;
  overflow-x: hidden;
  background: #f5f6f8;
  color: #1b2430;
}

.lab-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 24px;
  padding: 20px 28px;
  background: #ffffff;
  border-bottom: 1px solid #d8dee8;
}

.lab-header h1 {
  margin: 0;
  font-size: 1.2rem;
}

.lab-header p {
  margin: 6px 0 0;
  color: #5e6a78;
  font-size: 0.88rem;
}

.lab-main {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  padding: 20px 28px 40px;
}

.panel {
  background: #ffffff;
  border: 1px solid #d8dee8;
  border-radius: 8px;
  padding: 16px;
  min-width: 0;
}

.input-panel,
.results-panel {
  grid-column: span 2;
}

.panel-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}

.panel-title h2 {
  margin: 0;
  font-size: 1rem;
}

.panel-title span,
.muted {
  color: #657386;
  font-size: 0.85rem;
}

.question-input {
  width: 100%;
  min-height: 130px;
  resize: vertical;
  border: 1px solid #c8d0dc;
  border-radius: 6px;
  padding: 10px 12px;
  font: 13px/1.5 Consolas, Monaco, monospace;
}

.control-row,
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 12px;
  margin-top: 12px;
}

label {
  display: grid;
  gap: 5px;
  color: #3b4656;
  font-weight: 600;
  font-size: 0.85rem;
}

input {
  border: 1px solid #c8d0dc;
  border-radius: 6px;
  padding: 7px 10px;
  background: #ffffff;
  font-size: 0.9rem;
}

select,
.system-prompt-input {
  border: 1px solid #c8d0dc;
  border-radius: 6px;
  padding: 7px 10px;
  background: #ffffff;
  font: inherit;
}

.system-prompt-input {
  min-height: 72px;
  resize: vertical;
}

.input-preview p {
  margin: 6px 0 0;
}

.short-input {
  width: 80px;
}

.btn {
  border: 1px solid #bfcad8;
  border-radius: 6px;
  padding: 8px 14px;
  font-weight: 700;
  font-size: 0.88rem;
  cursor: pointer;
  white-space: nowrap;
}

.btn:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.btn.primary {
  border-color: #205f8f;
  background: #205f8f;
  color: #ffffff;
}

.btn.secondary {
  background: #ffffff;
  color: #1f2a37;
}

.btn.danger {
  border-color: #c0143c;
  background: #ffffff;
  color: #c0143c;
}

.error {
  color: #a1143a;
  font-size: 0.88rem;
  margin-top: 8px;
}

/* Models panel */
.model-list {
  display: grid;
  gap: 8px;
  max-height: 340px;
  overflow-y: auto;
}

.model-row {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 10px;
  border: 1px solid #e0e6f0;
  border-radius: 7px;
  padding: 9px 11px;
  background: #fbfcfd;
  cursor: pointer;
}

.model-row.blocked {
  opacity: 0.65;
}

.model-row span {
  display: grid;
  gap: 2px;
}

.model-row strong {
  font-size: 0.9rem;
}

.model-row small {
  color: #7a8799;
  font-size: 0.8rem;
}

.model-row em {
  font-style: normal;
  font-size: 0.8rem;
  color: #516071;
}

/* Context panel */
.context-panel {
  max-height: 400px;
  overflow-y: auto;
}

.context-group {
  margin-top: 12px;
}

.context-group h3 {
  font-size: 0.88rem;
  font-weight: 700;
  margin: 0 0 6px;
}

.context-group ul {
  display: grid;
  gap: 6px;
  padding-left: 16px;
}

.context-group li {
  display: grid;
  gap: 2px;
  font-size: 0.85rem;
}

/* Results table */
.run-block {
  margin-bottom: 24px;
}

.run-block:last-child {
  margin-bottom: 0;
}

.run-header {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 8px;
  padding-bottom: 6px;
  border-bottom: 2px solid #e2e8f0;
}

.run-label {
  font-weight: 800;
  font-size: 0.9rem;
  color: #205f8f;
  white-space: nowrap;
}

.run-question {
  flex: 1;
  margin: 0;
  font-size: 0.92rem;
  font-weight: 600;
  overflow-wrap: anywhere;
}

.answer-badge {
  font-size: 0.8rem;
  background: #e8f0fe;
  color: #1a56db;
  border-radius: 4px;
  padding: 2px 7px;
  white-space: nowrap;
}

.results-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.88rem;
}

.results-table th {
  text-align: left;
  font-size: 0.78rem;
  font-weight: 700;
  color: #516071;
  padding: 6px 8px;
  border-bottom: 1px solid #d8dee8;
  white-space: nowrap;
}

.results-table td {
  padding: 8px 8px;
  border-bottom: 1px solid #eef1f5;
  vertical-align: top;
}

.results-table tr.row-error td {
  background: #fff8f8;
}

.col-model { width: 160px; }
.col-stage { width: 160px; font-size: 0.8rem; }
.col-region { width: 140px; font-size: 0.8rem; }
.col-pick { width: 50px; text-align: center; }
.col-score { width: 46px; text-align: center; }
.col-response { min-width: 0; }

.tag {
  display: inline-block;
  border-radius: 4px;
  padding: 2px 7px;
  font-size: 0.78rem;
  font-weight: 700;
}

.option-tag { background: #eef2ff; color: #3730a3; }
.outside-tag { background: #f3f4f6; color: #374151; }
.error-tag { background: #fee2e2; color: #991b1b; }
.correct-tag { background: #dcfce7; color: #166534; }
.wrong-tag { background: #fee2e2; color: #991b1b; }
.syco-tag { background: #fef9c3; color: #854d0e; font-size: 0.75rem; }

.result-answer {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  line-height: 1.5;
}

.result-error {
  margin: 0;
  color: #a1143a;
  font-size: 0.85rem;
}

.trace-toggle {
  display: inline-block;
  margin-top: 6px;
  font-size: 0.78rem;
  color: #205f8f;
  background: none;
  border: none;
  cursor: pointer;
  padding: 0;
  text-decoration: underline;
}

.trace-row td {
  padding: 0 8px 10px;
  border-bottom: 1px solid #eef1f5;
}

.trace-block {
  margin: 0;
  padding: 10px 12px;
  border-radius: 6px;
  background: #f1f4f8;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 0.82rem;
  line-height: 1.5;
  max-height: 280px;
  overflow-y: auto;
}

.skipped-note {
  margin-top: 6px;
  font-size: 0.82rem;
}

.running-badge {
  font-size: 0.78rem;
  background: #fff8e1;
  color: #92400e;
  border-radius: 4px;
  padding: 2px 7px;
  white-space: nowrap;
}

.row-pending td {
  background: #fafbfc;
  color: #8a96a4;
}

.generating-text {
  font-style: italic;
}

.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid #d1d5db;
  border-top-color: #205f8f;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 860px) {
  .lab-main {
    grid-template-columns: 1fr;
  }

  .input-panel,
  .results-panel {
    grid-column: auto;
  }
}
</style>
