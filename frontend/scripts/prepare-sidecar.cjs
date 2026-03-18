const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const projectRoot = path.resolve(__dirname, '..')
const tauriRoot = path.join(projectRoot, 'src-tauri')

function fail(message) {
  console.error(`prepare-sidecar: ${message}`)
  process.exit(1)
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: tauriRoot,
    stdio: 'inherit',
    ...options,
  })

  if (result.error) {
    fail(result.error.message)
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
}

function capture(command, args) {
  const result = spawnSync(command, args, {
    cwd: tauriRoot,
    encoding: 'utf8',
  })

  if (result.error) {
    fail(result.error.message)
  }

  if (result.status !== 0) {
    fail(`failed to run ${command} ${args.join(' ')}`)
  }

  return result.stdout
}

function parseArgs(argv) {
  const parsed = {
    release: false,
    locked: false,
    target: process.env.CARGO_BUILD_TARGET || process.env.npm_config_target || '',
  }

  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]

    if (value === '--release') {
      parsed.release = true
      continue
    }

    if (value === '--locked') {
      parsed.locked = true
      continue
    }

    if (value === '--target') {
      const target = argv[index + 1]
      if (!target) {
        fail('missing value for --target')
      }
      parsed.target = target
      index += 1
      continue
    }

    if (value === '--help' || value === '-h') {
      console.log('Usage: node scripts/prepare-sidecar.cjs [--release] [--locked] [--target <triple>]')
      process.exit(0)
    }

    fail(`unknown argument: ${value}`)
  }

  return parsed
}

function resolveHostTarget() {
  const output = capture('rustc', ['-vV'])
  const hostLine = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.startsWith('host: '))

  if (!hostLine) {
    fail('could not determine host target from rustc -vV')
  }

  return hostLine.slice('host: '.length).trim()
}

function ensureNonEmptyFile(filePath) {
  if (!fs.existsSync(filePath)) {
    fail(`expected built binary at ${filePath}`)
  }

  const stats = fs.statSync(filePath)
  if (stats.size === 0) {
    fail(`built binary is empty: ${filePath}`)
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const target = options.target || resolveHostTarget()
  const isWindowsTarget = target.includes('windows')
  const profile = options.release ? 'release' : 'debug'
  const extension = isWindowsTarget ? '.exe' : ''

  const cargoArgs = ['build', '--bin', 'grafyn-mcp', '--no-default-features', '--features', 'mcp']
  if (options.locked) {
    cargoArgs.push('--locked')
  }
  if (options.release) {
    cargoArgs.push('--release')
  }
  cargoArgs.push('--target', target)

  console.log(`Preparing grafyn-mcp sidecar for ${target} (${profile})`)
  run('cargo', cargoArgs)

  const builtBinary = path.join(tauriRoot, 'target', target, profile, `grafyn-mcp${extension}`)
  ensureNonEmptyFile(builtBinary)

  const binariesDir = path.join(tauriRoot, 'binaries')
  fs.mkdirSync(binariesDir, { recursive: true })

  const packagedBinary = path.join(binariesDir, `grafyn-mcp-${target}${extension}`)
  fs.copyFileSync(builtBinary, packagedBinary)
  ensureNonEmptyFile(packagedBinary)

  console.log(`Prepared sidecar: ${path.relative(projectRoot, packagedBinary)}`)
}

main()
