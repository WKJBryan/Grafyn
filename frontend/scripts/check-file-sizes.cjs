#!/usr/bin/env node

// Size tripwire: fails when a source file grows past a line-count threshold.
// Catches the "one file does everything" drift this refactor split apart
// (commands/canvas.rs, services/twin_store.rs) before it recurs. See
// CLAUDE.md "Working conventions" for the ~1,500-line authoring target —
// this threshold (2,500) is a hard CI failure with headroom above that.

const fs = require('node:fs')
const path = require('node:path')

const projectRoot = path.resolve(__dirname, '..')

const DEFAULT_MAX_LINES = 2500

const SCAN_ROOTS = [
  { dir: path.join(projectRoot, 'src-tauri', 'src'), extensions: ['.rs'] },
  { dir: path.join(projectRoot, 'src'), extensions: ['.vue', '.js'] },
]

// Path-convention exclusions only — we do not parse Rust to find
// `#[cfg(test)]`-only files, and we do not parse JS to find describe/it
// blocks. A file is excluded solely by where it lives or how it's named.
function isExcluded(relativePath) {
  const segments = relativePath.split('/')

  if (segments.includes('__tests__')) {
    return true
  }

  const basename = path.basename(relativePath)

  if (/\.(spec|test)\.js$/.test(basename)) {
    return true
  }

  // Rust test-only files by naming convention (e.g. test_support.rs,
  // foo_test.rs) — not files that merely contain an inline `mod tests {}`.
  if (relativePath.endsWith('.rs') && /(^|_)test(s)?(_|\.rs$)/.test(basename)) {
    return true
  }

  return false
}

function walk(dir, extensions, results) {
  const entries = fs.readdirSync(dir, { withFileTypes: true })

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name)

    if (entry.isDirectory()) {
      walk(fullPath, extensions, results)
      continue
    }

    if (extensions.includes(path.extname(entry.name))) {
      results.push(fullPath)
    }
  }
}

function countLines(filePath) {
  const contents = fs.readFileSync(filePath, 'utf8')
  if (contents.length === 0) {
    return 0
  }
  return contents.split('\n').length
}

function main() {
  const maxLinesArg = process.argv.find((arg) => arg.startsWith('--max-lines='))
  const maxLines = maxLinesArg ? Number(maxLinesArg.split('=')[1]) : DEFAULT_MAX_LINES

  if (!Number.isFinite(maxLines) || maxLines <= 0) {
    console.error(`check-file-sizes: invalid --max-lines value: ${maxLinesArg}`)
    process.exit(1)
  }

  const files = []
  for (const { dir, extensions } of SCAN_ROOTS) {
    if (fs.existsSync(dir)) {
      walk(dir, extensions, files)
    }
  }

  const offenders = []

  for (const filePath of files) {
    const relativePath = path.relative(projectRoot, filePath).replace(/\\/g, '/')
    if (isExcluded(relativePath)) {
      continue
    }

    const lines = countLines(filePath)
    if (lines > maxLines) {
      offenders.push({ relativePath, lines })
    }
  }

  offenders.sort((a, b) => b.lines - a.lines)

  if (offenders.length > 0) {
    console.error(`check-file-sizes: ${offenders.length} file(s) exceed ${maxLines} lines:`)
    for (const { relativePath, lines } of offenders) {
      console.error(`  ${lines.toString().padStart(6)}  ${relativePath}`)
    }
    console.error('')
    console.error(
      'Split oversized files using the mod-facade pattern (see commands/canvas/ for the exemplar) ' +
        'instead of raising this threshold.',
    )
    process.exit(1)
  }

  console.log(`check-file-sizes: OK — no source file exceeds ${maxLines} lines.`)
}

main()
