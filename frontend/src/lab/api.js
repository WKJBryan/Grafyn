import { getTransport } from '@/api/transport'

const invoke = (...args) => getTransport().invoke(...args)

export const twinEval = {
  getModelMatrix: () => invoke('get_twin_eval_model_matrix', {}),

  previewInput: (rawQuestion, answerKey = null) =>
    invoke('preview_twin_eval_input', { rawQuestion, answerKey }),

  previewContext: (request) => invoke('preview_twin_eval_context', { request }),

  runLab: (request) => invoke('run_twin_eval_lab', { request }),

  exportResults: (results) => invoke('export_twin_eval_results', { results }),
}
