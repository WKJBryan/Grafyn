#!/usr/bin/env node
/**
 * Drive a real grafyn-mcp process over stdio and assert vault behavior.
 *
 * Protocol: newline-delimited JSON-RPC (rmcp 0.15), protocolVersion 2025-03-26.
 * Stdout is the protocol. Stderr is logs. The vault and data dir are created
 * under the system temp directory and removed at the end.
 *
 * Usage: node exercise-app.mjs /path/to/grafyn-mcp
 */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import readline from 'node:readline'

const binary = process.argv[2]
if (!binary) {
  console.error('Usage: node exercise-app.mjs /path/to/grafyn-mcp')
  process.exit(1)
}

const REQUIRED_TOOLS = [
  'list_notes',
  'get_note',
  'create_note',
  'update_note',
  'delete_note',
  'search_notes',
  'get_backlinks',
  'get_outgoing',
  'recall_relevant',
  'search_chunks',
  'import_conversation',
]

const TOKEN = 'xylophonequartz'
const IMPORT_MARKER = 'xylophone import marker'
const checks = []

function pass(name, detail) {
  checks.push({ name, ok: true, detail })
  console.log(`PASS  ${name}${detail ? ` — ${detail}` : ''}`)
}

function fail(name, detail) {
  checks.push({ name, ok: false, detail })
  console.error(`FAIL  ${name} — ${detail}`)
}

class Session {
  constructor(child) {
    this.child = child
    this.nextId = 1
    this.pending = new Map()
    this.stderr = ''
    this.closed = false

    child.stderr.on('data', (chunk) => {
      this.stderr += chunk.toString()
      if (this.stderr.length > 80_000) {
        this.stderr = this.stderr.slice(-60_000)
      }
    })

    const rl = readline.createInterface({ input: child.stdout })
    rl.on('line', (line) => this.onLine(line))
    child.on('exit', (code) => {
      this.closed = true
      this.exitCode = code
      for (const [, pending] of this.pending) {
        pending.reject(new Error(`grafyn-mcp exited (${code}) before answering`))
      }
      this.pending.clear()
    })
  }

  onLine(line) {
    const trimmed = line.trim()
    if (!trimmed.startsWith('{')) return
    let msg
    try {
      msg = JSON.parse(trimmed)
    } catch {
      return
    }
    if (msg.method && Object.prototype.hasOwnProperty.call(msg, 'id')) {
      this.send({ jsonrpc: '2.0', id: msg.id, result: {} })
      return
    }
    if (Object.prototype.hasOwnProperty.call(msg, 'id') && this.pending.has(msg.id)) {
      const pending = this.pending.get(msg.id)
      this.pending.delete(msg.id)
      pending.resolve(msg)
    }
  }

  send(obj) {
    this.child.stdin.write(`${JSON.stringify(obj)}\n`)
  }

  request(method, params, timeoutMs = 30_000) {
    const id = this.nextId++
    const payload = { jsonrpc: '2.0', id, method }
    if (params !== undefined) payload.params = params
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`timed out waiting for ${method} (${timeoutMs}ms)`))
      }, timeoutMs)
      this.pending.set(id, {
        resolve: (msg) => {
          clearTimeout(timer)
          resolve(msg)
        },
        reject: (err) => {
          clearTimeout(timer)
          reject(err)
        },
      })
      this.send(payload)
    })
  }

  notify(method) {
    this.send({ jsonrpc: '2.0', method })
  }

  async tool(name, args) {
    const msg = await this.request('tools/call', { name, arguments: args ?? {} })
    if (msg.error) {
      throw new Error(`${name} JSON-RPC error: ${JSON.stringify(msg.error)}`)
    }
    const result = msg.result ?? {}
    const text = (result.content ?? []).find((block) => block.type === 'text')?.text ?? ''
    let data = text
    try {
      data = JSON.parse(text)
    } catch {
      // delete_note and a few errors return plain text
    }
    return { isError: result.isError === true, text, data }
  }

  close() {
    if (this.closed) return
    try {
      this.child.stdin.end()
    } catch {
      // already closed
    }
    this.child.kill('SIGTERM')
  }
}

function start(bin, vault, data) {
  const child = spawn(bin, ['--vault', vault, '--data', data], {
    stdio: ['pipe', 'pipe', 'pipe'],
    // warn is required: the read-only fallback is a log line, and a parent
    // RUST_LOG=error would hide it.
    env: { ...process.env, RUST_LOG: 'warn' },
  })
  return new Session(child)
}

function walk(dir) {
  if (!fs.existsSync(dir)) return []
  const out = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...walk(full))
    else out.push(full)
  }
  return out
}

function findNoteFile(vault, id) {
  return walk(vault).find((file) => {
    if (!file.endsWith('.md')) return false
    const text = fs.readFileSync(file, 'utf8')
    return text.includes(`note_id: ${id}`) || text.includes(`note_id: "${id}"`)
  })
}

function noteText(vault, id) {
  const file = findNoteFile(vault, id)
  if (!file) return null
  return fs.readFileSync(file, 'utf8')
}

async function main() {
  const resolved = path.resolve(binary)
  if (!fs.existsSync(resolved)) {
    console.error(`Binary not found: ${resolved}`)
    process.exit(1)
  }

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grafyn-verify-'))
  const vault = path.join(root, 'vault')
  const data = path.join(root, 'data')
  fs.mkdirSync(vault)
  fs.mkdirSync(data)

  if (vault.includes(`${path.sep}Documents${path.sep}Grafyn`)) {
    console.error(`Refusing to use a real Grafyn directory: ${vault}`)
    process.exit(1)
  }

  const keep = process.env.GRAFYN_VERIFY_KEEP === '1'
  let session
  let second

  try {
    session = start(resolved, vault, data)
    const init = await session.request(
      'initialize',
      {
        protocolVersion: '2025-03-26',
        capabilities: {},
        clientInfo: { name: 'grafyn-verify', version: '1.0.0' },
      },
      60_000,
    )
    if (init.error) throw new Error(`initialize failed: ${JSON.stringify(init.error)}`)
    session.notify('notifications/initialized')

    const info = init.result ?? {}
    const serverName = info.serverInfo?.name ?? 'unknown'
    const serverVersion = info.serverInfo?.version ?? 'unknown'
    const protocol = info.protocolVersion ?? 'missing'
    pass(
      'initialize',
      `serverInfo ${serverName} ${serverVersion}, protocol ${protocol}`,
    )
    // get_info() leaves serverInfo at rmcp's Default, so the name is "rmcp".
    // The instructions string is what identifies this process as Grafyn.
    const instructions = String(info.instructions ?? '')
    if (!instructions.includes('Grafyn')) {
      fail('initialize instructions name Grafyn', JSON.stringify(info.instructions))
    } else {
      pass('initialize instructions name Grafyn')
    }

    const listed = await session.request('tools/list', {})
    const names = (listed.result?.tools ?? []).map((tool) => tool.name)
    const missing = REQUIRED_TOOLS.filter((name) => !names.includes(name))
    if (missing.length) fail('tools advertised', `missing ${missing.join(', ')}`)
    else pass('tools advertised', names.slice().sort().join(', '))

    const empty = await session.tool('list_notes')
    if (empty.isError || !Array.isArray(empty.data) || empty.data.length !== 0) {
      fail('empty vault lists no notes', empty.text)
    } else {
      pass('empty vault lists no notes')
    }

    const harbor = await session.tool('create_note', {
      title: 'Harbor Note',
      content: 'Harbor body for the link target.',
      tags: ['verify'],
      status: 'draft',
    })
    const phantom = await session.tool('create_note', {
      title: 'Phantom Note',
      content: 'This note exists so a leaked code-fence link would resolve.',
      tags: ['verify'],
      status: 'draft',
    })
    const sourceBody = [
      'Harbor link sentence.',
      '',
      '[[Harbor Note]] (supports)',
      '',
      'A fenced sample follows.',
      '',
      '```',
      '[[Phantom Note]] (supports)',
      '```',
      '',
      `Token ${TOKEN} lives in this note.`,
    ].join('\n')
    const source = await session.tool('create_note', {
      title: 'Source Note',
      content: sourceBody,
      tags: ['verify', 'graph'],
      status: 'draft',
    })
    if (harbor.isError || phantom.isError || source.isError) {
      fail(
        'create linked notes',
        [harbor.text, phantom.text, source.text].filter(Boolean).join(' | '),
      )
    } else {
      pass('create linked notes', `source ${source.data.id}`)
    }

    const sourceId = source.data?.id
    const harborId = harbor.data?.id
    const phantomId = phantom.data?.id
    const onDisk = sourceId ? noteText(vault, sourceId) : null
    if (!onDisk || !onDisk.startsWith('---') || !onDisk.includes(`status: draft`) || !onDisk.includes(TOKEN)) {
      fail('note file is markdown with draft frontmatter', onDisk ? onDisk.slice(0, 400) : 'file missing')
    } else {
      pass('note file is markdown with draft frontmatter', findNoteFile(vault, sourceId))
    }

    const fetched = await session.tool('get_note', { id: sourceId })
    if (fetched.isError || fetched.data?.content?.includes(TOKEN) !== true) {
      fail('get_note returns the body', fetched.text)
    } else {
      pass('get_note returns the body')
    }

    const missingNote = await session.tool('get_note', { id: 'does-not-exist' })
    if (!missingNote.isError) fail('missing note is an error', missingNote.text)
    else pass('missing note is an error')

    const outgoing = await session.tool('get_outgoing', { note_id: sourceId })
    const edges = Array.isArray(outgoing.data) ? outgoing.data : []
    const harborEdge = edges.find((edge) => edge.id === harborId)
    const phantomEdge = edges.find((edge) => edge.id === phantomId)
    if (outgoing.isError || !harborEdge || harborEdge.relation !== 'supports' || phantomEdge) {
      fail(
        'typed wikilink and code fence',
        `outgoing=${JSON.stringify(edges)}`,
      )
    } else {
      pass('typed wikilink and code fence', 'supports Harbor Note; Phantom Note is not an edge')
    }

    const backlinks = await session.tool('get_backlinks', { note_id: harborId })
    const back = Array.isArray(backlinks.data)
      ? backlinks.data.find((edge) => edge.id === sourceId)
      : null
    if (backlinks.isError || !back || back.relation !== 'related') {
      fail('backlink uses the reverse relation', JSON.stringify(backlinks.data))
    } else {
      pass('backlink uses the reverse relation', 'supports reverses to related')
    }

    const found = await session.tool('search_notes', { query: TOKEN, limit: 5 })
    const hit = Array.isArray(found.data) && found.data.some((row) => row.id === sourceId)
    if (found.isError || !hit) fail('search finds the note', found.text)
    else pass('search finds the note')

    const recalled = await session.tool('recall_relevant', { query: TOKEN, limit: 5 })
    const recallHit = Array.isArray(recalled.data) && recalled.data.some((row) => row.note_id === sourceId)
    if (recalled.isError || !recallHit) fail('recall finds the note', recalled.text)
    else pass('recall finds the note')

    const evidence = await session.tool('update_note', { id: sourceId, status: 'evidence' })
    const canonical = await session.tool('update_note', { id: sourceId, status: 'canonical' })
    const canonicalDisk = noteText(vault, sourceId) ?? ''
    if (
      evidence.isError ||
      canonical.isError ||
      canonical.data?.status !== 'canonical' ||
      !canonicalDisk.includes('status: canonical')
    ) {
      fail('status moves draft to canonical', `${evidence.text}\n${canonical.text}`)
    } else {
      pass('status moves draft to canonical')
    }

    const before = walk(vault).filter((file) => file.endsWith('.md')).length
    const rejected = await session.tool('update_note', { id: sourceId, status: 'published' })
    const after = walk(vault).filter((file) => file.endsWith('.md')).length
    const stillCanonical = (noteText(vault, sourceId) ?? '').includes('status: canonical')
    if (!rejected.isError || after !== before || !stillCanonical) {
      fail('unknown status is rejected', rejected.text)
    } else {
      pass('unknown status is rejected', 'published wrote nothing')
    }

    const importFile = path.join(root, 'conversations.json')
    fs.writeFileSync(
      importFile,
      JSON.stringify([
        {
          id: 'verify-conv',
          title: 'Verify Import Chat',
          create_time: 1234567890,
          mapping: {
            n1: {
              message: {
                content: { parts: [IMPORT_MARKER] },
                author: { role: 'user' },
                create_time: 1234567890,
              },
            },
          },
        },
      ]),
    )
    const imported = await session.tool('import_conversation', {
      file_path: importFile,
      conversation_ids: [],
    })
    const importedId = imported.data?.note_ids?.[0]
    const importedDisk = importedId ? noteText(vault, importedId) ?? '' : ''
    const importedNote = importedId ? await session.tool('get_note', { id: importedId }) : null
    if (
      imported.isError ||
      imported.data?.platform !== 'chatgpt' ||
      imported.data?.imported !== 1 ||
      !importedDisk.includes('status: evidence') ||
      !importedDisk.includes('source: chatgpt') ||
      !importedDisk.includes('created_via: mcp-import') ||
      !importedDisk.includes('migration_source: mcp-import') ||
      !importedNote?.data?.content?.includes(IMPORT_MARKER)
    ) {
      fail('chatgpt import becomes an evidence note', `${imported.text}\n${importedDisk.slice(0, 500)}`)
    } else {
      pass('chatgpt import becomes an evidence note', importedId)
    }

    const chunks = await session.tool('search_chunks', { query: TOKEN, token_budget: 200 })
    if (!chunks.isError || !/chunk index not available/i.test(chunks.text)) {
      fail('chunk index is absent until the desktop app builds it', chunks.text)
    } else {
      pass('chunk index is absent until the desktop app builds it')
    }

    const scratch = await session.tool('create_note', {
      title: 'Scratch Note',
      content: 'scratchtokenquartz should vanish after delete.',
      tags: [],
      status: 'draft',
    })
    const scratchHit = await session.tool('search_notes', { query: 'scratchtokenquartz', limit: 5 })
    const deleted = await session.tool('delete_note', { id: scratch.data?.id })
    const afterDelete = await session.tool('search_notes', { query: 'scratchtokenquartz', limit: 5 })
    const scratchGone = !findNoteFile(vault, scratch.data?.id)
    const stillListed = (afterDelete.data ?? []).some((row) => row.id === scratch.data?.id)
    if (
      scratch.isError ||
      scratchHit.isError ||
      !scratchHit.data?.some((row) => row.id === scratch.data.id) ||
      deleted.isError ||
      !scratchGone ||
      stillListed
    ) {
      fail(
        'delete removes the file and the search hit',
        `delete=${deleted.text} search=${afterDelete.text} fileGone=${scratchGone}`,
      )
    } else {
      pass('delete removes the file and the search hit')
    }

    second = start(resolved, vault, data)
    const secondInit = await second.request(
      'initialize',
      {
        protocolVersion: '2025-03-26',
        capabilities: {},
        clientInfo: { name: 'grafyn-verify-reader', version: '1.0.0' },
      },
      30_000,
    )
    second.notify('notifications/initialized')
    const probe = await second.tool('create_note', {
      title: 'Lock Probe Note',
      content: 'Written while another grafyn-mcp holds the search writer.',
      tags: [],
      status: 'draft',
    })
    const stderr = second.stderr
    const warned =
      typeof probe.data === 'object' &&
      probe.data &&
      String(probe.data.warning ?? '').includes('stale until reindex')
    if (secondInit.error || probe.isError || !warned || !/read-only search/i.test(stderr)) {
      fail(
        'second server is read-only',
        `init=${JSON.stringify(secondInit.error ?? secondInit.result?.serverInfo)} warning=${probe.data?.warning} stderr=${stderr.slice(-800)}`,
      )
    } else {
      pass('second server is read-only', 'search writer stays with the first process')
    }
    if (probe.data?.id) {
      await second.tool('delete_note', { id: probe.data.id })
    }

    const leftovers = walk(vault).filter((file) => path.basename(file).includes('.tmp-'))
    if (leftovers.length) fail('atomic writes leave no temp files', leftovers.join(', '))
    else pass('atomic writes leave no temp files')

    const finalList = await session.tool('list_notes')
    const ids = new Set((finalList.data ?? []).map((note) => note.id))
    if (!ids.has(sourceId) || !ids.has(harborId) || ids.has(scratch.data?.id)) {
      fail('list_notes matches the files that remain', JSON.stringify([...ids]))
    } else {
      pass('list_notes matches the files that remain', `${ids.size} notes`)
    }
  } catch (error) {
    fail('session', error.stack || error.message)
    if (session?.stderr) {
      console.error('--- server stderr ---')
      console.error(session.stderr.slice(-2000))
    }
  } finally {
    second?.close()
    session?.close()
    if (!keep) {
      fs.rmSync(root, { recursive: true, force: true })
    } else {
      console.log(`Kept temp vault at ${root}`)
    }
  }

  const failed = checks.filter((check) => !check.ok)
  console.log('')
  console.log(`binary: ${path.resolve(binary)}`)
  console.log(`vault:  ${vault}`)
  console.log(`checks: ${checks.length - failed.length} passed, ${failed.length} failed`)
  if (failed.length) {
    console.log('VERIFICATION FAILED')
    process.exit(1)
  }
  console.log('VERIFICATION PASSED')
}

main()
