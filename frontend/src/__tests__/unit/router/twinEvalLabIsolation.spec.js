import { describe, it, expect } from 'vitest'
import router from '@/router'

describe('Twin Eval lab isolation', () => {
  it('does not expose the research lab as a normal Grafyn route', () => {
    const routePaths = router.getRoutes().map(route => route.path)

    expect(routePaths).not.toContain('/twin-eval')
  })
})
