export const DEFAULT_QUANTIZATION = 'Q4_K_M'
export const DEFAULT_MODEL_DIR = 'twin-eval-models'

export const TWIN_EVAL_MODEL_REGISTRY = [
  {
    key: 'gemma4-e2b-base',
    label: 'Gemma 4 E2B Base',
    sourceRepo: 'google/gemma-4-E2B',
    ollamaTag: 'gemma4:e2b-base-q4_k_m',
    quantization: DEFAULT_QUANTIZATION,
    family: 'Gemma 4 E2B',
    stage: 'base/pre-trained',
    preferredGguf: null
  },
  {
    key: 'gemma4-e2b-it',
    label: 'Gemma 4 E2B IT',
    sourceRepo: 'google/gemma-4-E2B-it',
    ollamaTag: 'gemma4:e2b-q4_k_m',
    quantization: DEFAULT_QUANTIZATION,
    family: 'Gemma 4 E2B',
    stage: 'instruction/post-trained',
    preferredGguf: null
  },
  {
    key: 'sealion-v4-5-e2b-it',
    label: 'Gemma SEA-LION v4.5 E2B IT',
    sourceRepo: 'aisingapore/Gemma-SEA-LION-v4.5-E2B-IT',
    ollamaTag: 'gemma-sealion-v4.5:e2b-it-q4_k_m',
    quantization: DEFAULT_QUANTIZATION,
    family: 'Gemma 4 E2B',
    stage: 'SEA regional post-training/instruction',
    preferredGguf: null
  },
  {
    key: 'qwen3-6-27b',
    label: 'Qwen3.6 27B',
    sourceRepo: 'Qwen/Qwen3.6-27B',
    ollamaTag: 'qwen3.6:27b-q4_k_m',
    quantization: DEFAULT_QUANTIZATION,
    family: 'Qwen3.6',
    stage: 'pre-training plus post-training',
    preferredGguf: null
  },
  {
    key: 'qwen-sealion-v4-5-27b-it',
    label: 'Qwen SEA-LION v4.5 27B IT',
    sourceRepo: 'aisingapore/Qwen-SEA-LION-v4.5-27B-IT',
    ollamaTag: 'qwen-sealion-v4.5:27b-q4_k_m',
    quantization: DEFAULT_QUANTIZATION,
    family: 'Qwen3.6',
    stage: 'SEA regional post-training/instruction',
    preferredGguf: null
  }
]

export function buildSetupPlan({ execute = false, selectedKeys = [] } = {}) {
  const selected = selectedKeys.length
    ? TWIN_EVAL_MODEL_REGISTRY.filter(model => selectedKeys.includes(model.key))
    : TWIN_EVAL_MODEL_REGISTRY

  return {
    execute,
    quantization: DEFAULT_QUANTIZATION,
    modelDir: DEFAULT_MODEL_DIR,
    models: selected.map(model => ({
      ...model,
      hfLocalDir: `${DEFAULT_MODEL_DIR}/hf/${model.key}`,
      rawGgufPath: `${DEFAULT_MODEL_DIR}/gguf/${model.key}.bf16.gguf`,
      quantizedGgufPath: `${DEFAULT_MODEL_DIR}/gguf/${model.key}.${DEFAULT_QUANTIZATION}.gguf`,
      modelfilePath: `${DEFAULT_MODEL_DIR}/ollama/${model.key}.Modelfile`,
      manifestPath: `${DEFAULT_MODEL_DIR}/manifest.json`,
      acquisition: model.preferredGguf ? 'public_gguf' : 'official_hf_then_local_quantize'
    }))
  }
}
