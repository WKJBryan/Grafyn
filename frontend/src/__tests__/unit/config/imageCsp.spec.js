import { describe, expect, it } from 'vitest'
import tauriConfig from '../../../../src-tauri/tauri.conf.json'

describe('generated image content security policy', () => {
  it('admits local Blob previews without widening scripts or connections', () => {
    const directives = Object.fromEntries(tauriConfig.app.security.csp
      .split(';')
      .map(directive => directive.trim().split(/\s+/))
      .filter(parts => parts[0])
      .map(([name, ...sources]) => [name, sources]))

    expect(directives['img-src']).toContain('blob:')
    expect(directives['default-src']).toEqual(["'self'"])
    expect(directives['script-src']).toEqual(["'self'"])
    expect(directives['connect-src']).toEqual([
      "'self'",
      'https://openrouter.ai',
      'https://grafyn-updater.grafyn-updater.workers.dev',
    ])
  })
})
