const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const projectRoot = path.resolve(__dirname, '..')

function fail(message) {
  console.error(`run-tauri: ${message}`)
  process.exit(1)
}

function run(command, args, cwd = projectRoot) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: 'inherit',
  })

  if (result.error) {
    fail(result.error.message)
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
}

function findOption(args, name) {
  const optionIndex = args.indexOf(name)
  if (optionIndex === -1) {
    return ''
  }

  const value = args[optionIndex + 1]
  if (!value || value.startsWith('--')) {
    fail(`missing value for ${name}`)
  }

  return value
}

function hasOption(args, name) {
  return args.includes(name)
}

function resolveTargetForBundles(target) {
  if (target) {
    return target
  }

  if (process.platform === 'win32') {
    return 'x86_64-pc-windows-msvc'
  }

  if (process.platform === 'darwin') {
    return 'aarch64-apple-darwin'
  }

  return 'x86_64-unknown-linux-gnu'
}

function localBundleTargets(target) {
  if (target.includes('windows')) {
    return ['nsis']
  }

  if (target.includes('apple-darwin')) {
    return ['dmg']
  }

  if (target.includes('linux')) {
    return ['deb', 'appimage']
  }

  return []
}

function isMobileTarget(target) {
  return target.includes('android') || target.includes('ios')
}

function main() {
  const [subcommand, ...tauriArgs] = process.argv.slice(2)
  if (!subcommand || !['build', 'dev'].includes(subcommand)) {
    fail('usage: node scripts/run-tauri.cjs <build|dev> [tauri args]')
  }

  const target = findOption(tauriArgs, '--target')
  const config = findOption(tauriArgs, '--config')

  const prepareArgs = []
  if (subcommand === 'build') {
    prepareArgs.push('--release')
  }
  if (target) {
    prepareArgs.push('--target', target)
  }

  if (
    subcommand === 'build' &&
    !process.env.TAURI_SIGNING_PRIVATE_KEY &&
    !hasOption(tauriArgs, '--bundles') &&
    !config &&
    !isMobileTarget(target)
  ) {
    const bundleTargets = localBundleTargets(resolveTargetForBundles(target))
    if (bundleTargets.length > 0) {
      console.log(
        `TAURI_SIGNING_PRIVATE_KEY is not set; building local installer bundles only (${bundleTargets.join(', ')})`,
      )
      const desktopConfig = JSON.parse(
        fs.readFileSync(path.join(projectRoot, 'src-tauri', 'tauri.desktop.conf.json'), 'utf8'),
      )
      desktopConfig.bundle.targets = bundleTargets
      desktopConfig.bundle.createUpdaterArtifacts = false
      tauriArgs.push('--config', JSON.stringify(desktopConfig))
    }
  }

  if (!config && !hasOption(tauriArgs, '--config') && !isMobileTarget(target)) {
    tauriArgs.push('--config', 'src-tauri/tauri.desktop.conf.json')
  }

  if (!isMobileTarget(target)) {
    run(process.execPath, [path.join(__dirname, 'prepare-sidecar.cjs'), ...prepareArgs])
  }
  const tauriEntrypoint = path.join(
    projectRoot,
    'node_modules',
    '@tauri-apps',
    'cli',
    'tauri.js',
  )
  run(process.execPath, [tauriEntrypoint, subcommand, ...tauriArgs])
}

main()
