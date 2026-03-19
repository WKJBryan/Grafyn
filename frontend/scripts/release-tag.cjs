#!/usr/bin/env node

const path = require('node:path')
const {
  assertValidVersion,
  assertVersionsAligned,
  ensureCleanGitTree,
  ensureGitBranch,
  ensureOnlyExpectedFilesChanged,
  ensureTagAbsent,
  fail,
  getGitBranch,
  projectRoot,
  releaseManifestFiles,
  repoRoot,
  runPassthrough,
  tauriRoot,
} = require('./release-utils.cjs')

function parseArgs(argv) {
  const options = {
    prepareOnly: false,
  }

  let version = ''

  for (const value of argv) {
    if (value === '--prepare-only') {
      options.prepareOnly = true
      continue
    }

    if (value === '--help' || value === '-h') {
      console.log('Usage: node scripts/release-tag.cjs <version> [--prepare-only]')
      process.exit(0)
    }

    if (!version) {
      version = value
      continue
    }

    fail(`unknown argument: ${value}`)
  }

  if (!version) {
    console.log('Usage: node scripts/release-tag.cjs <version> [--prepare-only]')
    process.exit(1)
  }

  assertValidVersion(version)
  return {
    ...options,
    version,
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const { prepareOnly, version } = options
  const currentVersion = assertVersionsAligned()
  const currentBranch = getGitBranch()

  ensureCleanGitTree()

  if (!currentBranch) {
    fail('release commands must be run from a branch, not a detached HEAD')
  }

  ensureTagAbsent(`v${version}`)

  if (version === currentVersion) {
    if (prepareOnly) {
      fail(`version ${version} is already prepared on this branch`, 'run the final tagging step from clean main instead')
    }

    ensureGitBranch('main')

    console.log(`Tagging existing release commit for v${version}`)
    runPassthrough(process.execPath, [path.join(__dirname, 'release-verify.cjs')], {
      cwd: projectRoot,
    })
    runPassthrough('git', ['tag', '-a', `v${version}`, '-m', `Grafyn v${version}`], { cwd: repoRoot })

    console.log('\nRelease tag created successfully.')
    console.log('Push when ready:')
    console.log('  git push origin v' + version)
    return
  }

  if (!prepareOnly) {
    ensureGitBranch('main')
  }

  console.log(`Preparing release v${version}`)
  runPassthrough(process.execPath, [path.join(__dirname, 'bump-version.cjs'), version], {
    cwd: projectRoot,
  })

  console.log('\nRegenerating Cargo.lock with cargo generate-lockfile')
  runPassthrough('cargo', ['generate-lockfile'], { cwd: tauriRoot })

  ensureOnlyExpectedFilesChanged(releaseManifestFiles)

  console.log('\nRunning release verification')
  runPassthrough(process.execPath, [path.join(__dirname, 'release-verify.cjs'), '--skip-git-clean-check'], {
    cwd: projectRoot,
  })

  runPassthrough('git', ['add', ...releaseManifestFiles], { cwd: repoRoot })
  runPassthrough('git', ['commit', '-m', `chore: release v${version}`], { cwd: repoRoot })

  if (prepareOnly) {
    console.log('\nRelease preparation commit created successfully.')
    console.log('Next steps:')
    console.log('  1. Push this branch and merge it through a PR.')
    console.log(`  2. On clean main, run: npm run release:tag -- ${version}`)
    console.log(`  3. Push the tag: git push origin v${version}`)
    return
  }

  runPassthrough('git', ['tag', '-a', `v${version}`, '-m', `Grafyn v${version}`], { cwd: repoRoot })

  console.log('\nRelease commit and tag created successfully.')
  console.log('Push when ready:')
  console.log('  git push origin main --follow-tags')
}

main()
