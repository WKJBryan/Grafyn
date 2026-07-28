import { describe, it, expect } from 'vitest'
import {
  DEFAULT_QUANTIZATION,
  TWIN_EVAL_MODEL_REGISTRY,
  buildSetupPlan
} from '../../../../scripts/twin-eval-model-registry.mjs'

describe('twin eval model setup registry', () => {
  it('pins the five planned model variants to one quantization target', () => {
    expect(DEFAULT_QUANTIZATION).toBe('Q4_K_M')
    expect(TWIN_EVAL_MODEL_REGISTRY.map(model => model.key)).toEqual([
      'gemma4-e2b-base',
      'gemma4-e2b-it',
      'sealion-v4-5-e2b-it',
      'qwen3-6-27b',
      'qwen-sealion-v4-5-27b-it'
    ])
    expect(TWIN_EVAL_MODEL_REGISTRY.every(model => model.quantization === 'Q4_K_M')).toBe(true)
  })

  it('builds a dry-run setup plan without requiring an HF token', () => {
    const plan = buildSetupPlan({ execute: false, selectedKeys: ['qwen-sealion-v4-5-27b-it'] })

    expect(plan.execute).toBe(false)
    expect(plan.models).toHaveLength(1)
    expect(plan.models[0]).toMatchObject({
      key: 'qwen-sealion-v4-5-27b-it',
      sourceRepo: 'aisingapore/Qwen-SEA-LION-v4.5-27B-IT',
      quantization: 'Q4_K_M',
      ollamaTag: 'qwen-sealion-v4.5:27b-q4_k_m'
    })
  })
})
