import { resolve } from 'node:path'

export function resolveViteInputs(projectRoot, includeTwinEvalLab = false) {
  const inputs = {
    main: resolve(projectRoot, 'index.html'),
  }
  if (includeTwinEvalLab) {
    inputs.lab = resolve(projectRoot, 'lab.html')
  }
  return inputs
}

export function assertTwinEvalIsolation(bundle) {
  const forbiddenMarkers = [
    'TwinEvalLab',
    'run_twin_eval_lab',
    'export_twin_eval_results',
  ]

  for (const [fileName, output] of Object.entries(bundle)) {
    if (fileName === 'lab.html') {
      throw new Error('Normal Grafyn build emitted the Twin Eval lab HTML entry')
    }

    const content = output.type === 'asset' ? String(output.source) : output.code
    const marker = forbiddenMarkers.find((value) => content.includes(value))
    if (marker) {
      throw new Error(`Normal Grafyn build emitted Twin Eval marker ${marker} in ${fileName}`)
    }
  }
}
