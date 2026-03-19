const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const projectRoot = path.resolve(__dirname, '..')
const repoRoot = path.resolve(projectRoot, '..')
const tauriRoot = path.join(projectRoot, 'src-tauri')

const releaseTargets = [
  'x86_64-pc-windows-msvc',
  'aarch64-pc-windows-msvc',
  'aarch64-apple-darwin',
  'x86_64-unknown-linux-gnu',
]

const releaseManifestFiles = [
  'frontend/package.json',
  'frontend/src-tauri/Cargo.toml',
  'frontend/src-tauri/Cargo.lock',
  'frontend/src-tauri/tauri.conf.json',
]

function fail(message, details = '') {
  console.error(`release: ${message}`)
  if (details) {
    console.error(details)
  }
  process.exit(1)
}

function formatCommand(command, args) {
  return [command, ...args].join(' ')
}

function spawn(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 10 * 1024 * 1024,
    stdio: 'pipe',
    ...options,
  })

  if (result.error) {
    fail(`failed to run ${formatCommand(command, args)}`, result.error.message)
  }

  return result
}

function run(command, args, options = {}) {
  const result = spawn(command, args, options)

  if (result.status !== 0) {
    const output = [result.stdout, result.stderr].filter(Boolean).join('\n').trim()
    fail(`command failed: ${formatCommand(command, args)}`, output)
  }

  return (result.stdout || '').trim()
}

function runQuiet(command, args, options = {}) {
  const result = spawn(command, args, {
    stdio: ['ignore', 'ignore', 'pipe'],
    ...options,
  })

  if (result.status !== 0) {
    const output = (result.stderr || '').trim()
    fail(`command failed: ${formatCommand(command, args)}`, output)
  }
}

function runPassthrough(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    ...options,
  })

  if (result.error) {
    fail(`failed to run ${formatCommand(command, args)}`, result.error.message)
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
}

function normalizePath(filePath) {
  return filePath.replace(/\\/g, '/')
}

function readJson(relativePath) {
  return JSON.parse(fs.readFileSync(path.join(projectRoot, relativePath), 'utf8'))
}

function readText(relativePath) {
  return fs.readFileSync(path.join(projectRoot, relativePath), 'utf8')
}

function extractCargoVersion(cargoToml) {
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)
  if (!match) {
    fail('could not find version in frontend/src-tauri/Cargo.toml')
  }
  return match[1]
}

function stripLeadingV(version) {
  return version.startsWith('v') ? version.slice(1) : version
}

function assertValidVersion(version) {
  if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
    fail(`invalid semver "${version}"`, 'expected X.Y.Z or X.Y.Z-prerelease')
  }
}

function getVersionState() {
  const packageJson = readJson('package.json')
  const tauriConfig = readJson('src-tauri/tauri.conf.json')
  const cargoToml = readText('src-tauri/Cargo.toml')
  const windowTitle = tauriConfig.tauri?.windows?.[0]?.title || ''

  return {
    packageVersion: packageJson.version,
    tauriVersion: tauriConfig.package?.version || '',
    cargoVersion: extractCargoVersion(cargoToml),
    windowTitle,
  }
}

function assertVersionsAligned() {
  const versions = getVersionState()
  const entries = [
    ['frontend/package.json', versions.packageVersion],
    ['frontend/src-tauri/Cargo.toml', versions.cargoVersion],
    ['frontend/src-tauri/tauri.conf.json', versions.tauriVersion],
  ]

  const distinctVersions = new Set(entries.map(([, version]) => version))
  if (distinctVersions.size !== 1) {
    fail(
      'version mismatch across release manifests',
      entries.map(([file, version]) => `  ${file}: ${version || '(missing)'}`).join('\n'),
    )
  }

  const version = versions.packageVersion
  assertValidVersion(version)

  if (versions.windowTitle) {
    const expectedTitle = `Grafyn v${version}`
    if (versions.windowTitle !== expectedTitle) {
      fail(
        'window title version is out of sync',
        `  expected "${expectedTitle}" in frontend/src-tauri/tauri.conf.json but found "${versions.windowTitle}"`,
      )
    }
  }

  return version
}

function listGitChanges() {
  const output = run('git', ['status', '--porcelain=v1', '--untracked-files=all'], {
    cwd: repoRoot,
  })

  if (!output) {
    return []
  }

  return output
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => {
      const rawPath = line.slice(3).trim()
      const normalized = normalizePath(rawPath.split(' -> ').pop())

      return {
        line,
        path: normalized,
        status: line.slice(0, 2),
      }
    })
}

function ensureCleanGitTree() {
  const changes = listGitChanges()
  if (changes.length > 0) {
    fail(
      'working tree must be clean before running this command',
      changes.map(({ line }) => `  ${line}`).join('\n'),
    )
  }
}

function ensureOnlyExpectedFilesChanged(expectedPaths) {
  const allowed = new Set(expectedPaths.map(normalizePath))
  const changes = listGitChanges()
  const unexpected = changes.filter(({ path: changedPath }) => !allowed.has(changedPath))

  if (unexpected.length > 0) {
    fail(
      'release preparation changed files outside the expected manifest set',
      unexpected.map(({ line }) => `  ${line}`).join('\n'),
    )
  }

  return changes
}

function ensureGitBranch(expectedBranch) {
  const currentBranch = getGitBranch()
  if (currentBranch !== expectedBranch) {
    fail(`release tags must be cut from ${expectedBranch}`, `current branch: ${currentBranch || '(detached HEAD)'}`)
  }
}

function getGitBranch() {
  return run('git', ['branch', '--show-current'], { cwd: repoRoot })
}

function ensureTagAbsent(tagName) {
  const localTag = spawn('git', ['rev-parse', '--verify', '--quiet', `refs/tags/${tagName}`], {
    cwd: repoRoot,
  })
  if (localTag.status === 0) {
    fail(`tag ${tagName} already exists locally`)
  }

  const originCheck = spawn('git', ['remote', 'get-url', 'origin'], { cwd: repoRoot })
  if (originCheck.status !== 0) {
    return
  }

  const remoteTag = spawn('git', ['ls-remote', '--tags', 'origin', `refs/tags/${tagName}`], {
    cwd: repoRoot,
  })
  if (remoteTag.status !== 0) {
    fail(
      `failed to confirm whether ${tagName} already exists on origin`,
      [remoteTag.stdout, remoteTag.stderr].filter(Boolean).join('\n').trim(),
    )
  }

  if ((remoteTag.stdout || '').trim()) {
    fail(`tag ${tagName} already exists on origin`)
  }
}

module.exports = {
  assertValidVersion,
  assertVersionsAligned,
  ensureCleanGitTree,
  ensureGitBranch,
  ensureOnlyExpectedFilesChanged,
  ensureTagAbsent,
  fail,
  getGitBranch,
  normalizePath,
  projectRoot,
  releaseManifestFiles,
  releaseTargets,
  repoRoot,
  run,
  runQuiet,
  runPassthrough,
  stripLeadingV,
  tauriRoot,
}
