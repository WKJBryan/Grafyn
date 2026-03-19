#!/usr/bin/env node

const path = require('node:path')
const {
  assertVersionsAligned,
  ensureCleanGitTree,
  fail,
  releaseTargets,
  runQuiet,
  tauriRoot,
} = require('./release-utils.cjs')

function parseArgs(argv) {
  const options = {
    skipGitCleanCheck: false,
  }

  for (const value of argv) {
    if (value === '--skip-git-clean-check') {
      options.skipGitCleanCheck = true
      continue
    }

    if (value === '--help' || value === '-h') {
      console.log('Usage: node scripts/release-verify.cjs [--skip-git-clean-check]')
      process.exit(0)
    }

    fail(`unknown argument: ${value}`)
  }

  return options
}

function validateCargoGraph(target, label, featureArgs) {
  const cargoArgs = [
    'metadata',
    '--format-version',
    '1',
    '--no-deps',
    '--locked',
    '--manifest-path',
    path.join(tauriRoot, 'Cargo.toml'),
    '--filter-platform',
    target,
    ...featureArgs,
  ]

  console.log(`- ${label} (${target})`)
  runQuiet('cargo', cargoArgs, { cwd: tauriRoot })
}

function main() {
  const options = parseArgs(process.argv.slice(2))

  if (!options.skipGitCleanCheck) {
    ensureCleanGitTree()
  }

  const version = assertVersionsAligned()
  let graphCount = 0

  console.log(`Verified release manifest versions at ${version}`)
  console.log('Validating Cargo.lock against release targets:')

  for (const target of releaseTargets) {
    validateCargoGraph(target, 'desktop app', [])
    graphCount += 1

    validateCargoGraph(target, 'grafyn-mcp sidecar', ['--no-default-features', '--features', 'mcp'])
    graphCount += 1
  }

  console.log(`Release verification passed for ${graphCount} Cargo graphs across ${releaseTargets.length} targets.`)
}

main()
