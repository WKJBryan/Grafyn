import { createRequire } from 'node:module'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const require = createRequire(import.meta.url)
const { releaseManifestFiles } = require(resolve(process.cwd(), 'scripts/release-utils.cjs'))

describe('release preparation manifests', () => {
  it('recognizes and stages the desktop Tauri config changed by the version bump', () => {
    const releaseTag = readFileSync(resolve(process.cwd(), 'scripts/release-tag.cjs'), 'utf8')

    expect(releaseManifestFiles).toContain('frontend/src-tauri/tauri.desktop.conf.json')
    expect(releaseTag).toContain('ensureOnlyExpectedFilesChanged(releaseManifestFiles)')
    expect(releaseTag).toContain("runPassthrough('git', ['add', ...releaseManifestFiles]")
  })
})
