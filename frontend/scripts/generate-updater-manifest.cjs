#!/usr/bin/env node

const fs = require('node:fs')
const path = require('node:path')
const { assertValidVersion, fail, stripLeadingV } = require('./release-utils.cjs')

const installerChecks = [
  { label: 'Windows x64 installer', pattern: /_x64-setup\.exe$/ },
  { label: 'Windows ARM64 installer', pattern: /_arm64-setup\.exe$/ },
  { label: 'macOS DMG', pattern: /_aarch64\.dmg$/i },
  { label: 'Linux DEB', pattern: /_amd64\.deb$/i },
  { label: 'Linux AppImage', pattern: /_amd64\.AppImage$/ },
]

const updaterChecks = [
  {
    canonical: 'darwin-aarch64-app',
    aliases: ['darwin-aarch64'],
    pattern: /\.app\.tar\.gz$/,
  },
  {
    canonical: 'linux-x86_64-appimage',
    aliases: ['linux-x86_64'],
    pattern: /\.AppImage\.tar\.gz$/,
  },
  {
    canonical: 'windows-x86_64-nsis',
    aliases: ['windows-x86_64'],
    pattern: /_x64-setup\.nsis\.zip$/,
  },
  {
    canonical: 'windows-aarch64-nsis',
    aliases: ['windows-aarch64'],
    pattern: /_arm64-setup\.nsis\.zip$/,
  },
]

function parseArgs(argv) {
  const options = {
    pubDate: new Date().toISOString(),
    notes: '',
  }

  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]

    if (value === '--assets-dir') {
      options.assetsDir = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--version') {
      options.version = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--worker-url') {
      options.workerUrl = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--pub-date') {
      options.pubDate = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--notes-file') {
      options.notesFile = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--out') {
      options.out = argv[index + 1]
      index += 1
      continue
    }

    if (value === '--help' || value === '-h') {
      console.log('Usage: node scripts/generate-updater-manifest.cjs --assets-dir <dir> --version <version> --worker-url <url> --out <file> [--pub-date <iso>] [--notes-file <file>]')
      process.exit(0)
    }

    fail(`unknown argument: ${value}`)
  }

  if (!options.assetsDir || !options.version || !options.workerUrl || !options.out) {
    fail('missing required arguments', 'expected --assets-dir, --version, --worker-url, and --out')
  }

  return options
}

function listAssetFiles(assetsDir) {
  if (!fs.existsSync(assetsDir)) {
    fail(`assets directory does not exist: ${assetsDir}`)
  }

  return fs
    .readdirSync(assetsDir, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
}

function findSingleMatch(files, pattern, label) {
  const matches = files.filter((file) => pattern.test(file))

  if (matches.length === 0) {
    fail(`missing required asset for ${label}`)
  }

  if (matches.length > 1) {
    fail(`found multiple candidate assets for ${label}`, matches.map((match) => `  ${match}`).join('\n'))
  }

  return matches[0]
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const version = stripLeadingV(options.version)

  assertValidVersion(version)

  if (options.notesFile) {
    if (!fs.existsSync(options.notesFile)) {
      fail(`notes file does not exist: ${options.notesFile}`)
    }
    options.notes = fs.readFileSync(options.notesFile, 'utf8').trim()
  }

  const workerUrl = options.workerUrl.replace(/\/+$/, '')
  const versionTag = `v${version}`
  const assetsDir = path.resolve(options.assetsDir)
  const outputFile = path.resolve(options.out)
  const assetFiles = listAssetFiles(assetsDir)
  const nonSignatureFiles = assetFiles.filter((file) => !file.endsWith('.sig'))

  for (const check of installerChecks) {
    findSingleMatch(nonSignatureFiles, check.pattern, check.label)
  }

  const platforms = {}

  for (const check of updaterChecks) {
    const assetName = findSingleMatch(nonSignatureFiles, check.pattern, check.canonical)
    const signatureName = `${assetName}.sig`
    const signaturePath = path.join(assetsDir, signatureName)

    if (!fs.existsSync(signaturePath)) {
      fail(`missing updater signature for ${assetName}`)
    }

    const signature = fs.readFileSync(signaturePath, 'utf8').trim()
    if (!signature) {
      fail(`signature file is empty: ${signatureName}`)
    }

    const entry = {
      signature,
      url: `${workerUrl}/download/${versionTag}/${encodeURIComponent(assetName)}`,
    }

    platforms[check.canonical] = entry
    for (const alias of check.aliases) {
      platforms[alias] = entry
    }
  }

  const manifest = {
    version,
    notes: options.notes || `Release v${version}`,
    pub_date: options.pubDate,
    platforms,
  }

  fs.mkdirSync(path.dirname(outputFile), { recursive: true })
  fs.writeFileSync(outputFile, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')

  console.log(`Generated updater manifest at ${outputFile}`)
}

main()
