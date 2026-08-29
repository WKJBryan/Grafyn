const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '..', '..')
const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'grafyn-license-alignment-'))

const fixtureFiles = [
  'LICENSE',
  'LICENSES/MPL-2.0.txt',
  'LICENSES/Apache-2.0.txt',
  'CONTRIBUTING.md',
  'TRADEMARKS.md',
  'RELICENSING.md',
  'README.md',
  'frontend/package.json',
  'frontend/package-lock.json',
  'frontend/scripts/check-license-alignment.cjs',
  'frontend/src-tauri/Cargo.toml',
  'frontend/src-tauri/Cargo.lock',
  'e2e/package.json',
  'e2e/package-lock.json',
]

try {
  for (const relativePath of fixtureFiles) {
    const fixturePath = path.join(fixtureRoot, relativePath)
    fs.mkdirSync(path.dirname(fixturePath), { recursive: true })
    fs.copyFileSync(path.join(root, relativePath), fixturePath)
  }

  const contributingPath = path.join(fixtureRoot, 'CONTRIBUTING.md')
  const contributing = fs.readFileSync(contributingPath, 'utf8')
  const mutated = contributing.replace(
    /    maintained indefinitely and may be redistributed consistent with\r?\n    this project or the open source license\(s\) involved\./,
    '    maintained temporarily.',
  )
  assert.notEqual(mutated, contributing, 'test fixture must alter DCO clause (d)')
  fs.writeFileSync(contributingPath, mutated)

  const result = spawnSync('npm run check:licenses', {
    cwd: path.join(fixtureRoot, 'frontend'),
    encoding: 'utf8',
    shell: true,
  })
  const output = `${result.stdout || ''}${result.stderr || ''}`

  assert.equal(result.status, 1, `altered DCO text must fail check:licenses\n${output}`)
  assert.match(output, /complete unmodified official DCO 1\.1 text/)
  console.log('License alignment mutation test passed.')
} finally {
  fs.rmSync(fixtureRoot, { recursive: true, force: true })
}
