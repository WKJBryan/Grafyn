import { describe, it, expect } from 'vitest'
import router from '@/router'
import {
  assertTwinEvalIsolation,
  resolveViteInputs,
} from '../../../../scripts/twin-eval-isolation.mjs'

const projectRoot = 'C:/workspace/frontend'

describe('Twin Eval lab isolation', () => {
  it('does not expose the research lab as a normal Grafyn route', () => {
    const routePaths = router.getRoutes().map(route => route.path)

    expect(routePaths).not.toContain('/twin-eval')
  })

  it('omits the lab HTML entry from a normal build', () => {
    expect(resolveViteInputs(projectRoot, false)).toEqual({
      main: expect.stringMatching(/index\.html$/),
    })
  })

  it('adds the lab HTML entry only for an explicit lab build', () => {
    expect(resolveViteInputs(projectRoot, true)).toEqual({
      main: expect.stringMatching(/index\.html$/),
      lab: expect.stringMatching(/lab\.html$/),
    })
  })

  it('rejects Twin Eval commands or assets in a normal build bundle', () => {
    expect(() => assertTwinEvalIsolation({
      'assets/main.js': {
        type: 'chunk',
        code: "invoke('export_twin_eval_results')",
      },
    })).toThrow(/Twin Eval marker/)

    expect(() => assertTwinEvalIsolation({
      'lab.html': { type: 'asset', source: '<html></html>' },
    })).toThrow(/lab HTML entry/)
  })
})
