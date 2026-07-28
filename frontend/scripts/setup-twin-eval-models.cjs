const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')
const { pathToFileURL } = require('node:url')

const projectRoot = path.resolve(__dirname, '..')
const modelRoot = path.join(projectRoot, 'twin-eval-models')
const pythonEnvDir = path.join(modelRoot, 'python')
const pythonEnvReadyMarker = path.join(pythonEnvDir, '.twin-eval-python-ready')
const preferredUvPython = process.env.TWIN_EVAL_PYTHON || '3.12'

function parseArgs(argv) {
  const args = new Set(argv)
  const selectedKeys = []
  const modelIndex = argv.indexOf('--model')
  if (modelIndex !== -1 && argv[modelIndex + 1]) {
    selectedKeys.push(argv[modelIndex + 1])
  }

  return {
    execute: args.has('--execute'),
    dryRun: args.has('--dry-run') || !args.has('--execute'),
    selectedKeys,
  }
}

function commandExists(command) {
  const checker = process.platform === 'win32' ? 'where.exe' : 'which'
  const result = spawnSync(checker, [command], { stdio: 'ignore' })
  return result.status === 0
}

function resolvePythonCommand() {
  if (commandExists('uv')) {
    return {
      kind: 'uv',
      command: 'uv',
    }
  }
  if (commandExists('python')) {
    return {
      kind: 'python',
      command: 'python',
    }
  }
  if (commandExists('py')) {
    return {
      kind: 'python',
      command: 'py',
    }
  }
  return null
}

function pythonEnvExecutable() {
  if (process.platform === 'win32') {
    return path.join(pythonEnvDir, 'Scripts', 'python.exe')
  }
  return path.join(pythonEnvDir, 'bin', 'python')
}

function pythonEnvUsesPreferredVersion() {
  const cfgPath = path.join(pythonEnvDir, 'pyvenv.cfg')
  if (!fs.existsSync(cfgPath)) {
    return true
  }

  const cfg = fs.readFileSync(cfgPath, 'utf8')
  return cfg.includes(`version_info = ${preferredUvPython}`) || cfg.includes(`version = ${preferredUvPython}`)
}

function resolveLlamaConverter() {
  if (commandExists('llama-convert-hf-to-gguf')) {
    return {
      kind: 'command',
      command: 'llama-convert-hf-to-gguf',
      argsPrefix: [],
      display: 'llama-convert-hf-to-gguf',
    }
  }

  const envScript = process.env.LLAMA_CPP_CONVERT_HF_TO_GGUF
  const likelyScripts = [
    envScript,
    path.join(process.env.USERPROFILE || '', 'llama.cpp', 'convert_hf_to_gguf.py'),
    path.join(process.env.HOME || '', 'llama.cpp', 'convert_hf_to_gguf.py'),
  ].filter(Boolean)

  const scriptPath = likelyScripts.find(candidate => fs.existsSync(candidate))
  if (!scriptPath) {
    return null
  }

  return {
    kind: 'python-script',
    scriptPath,
    display: scriptPath,
  }
}

function resolveLlamaQuantize() {
  if (commandExists('llama-quantize')) {
    return 'llama-quantize'
  }

  const envBinary = process.env.LLAMA_CPP_QUANTIZE
  const likelyBinaries = [
    envBinary,
    path.join(process.env.USERPROFILE || '', 'llama.cpp', 'build', 'bin', 'Release', 'llama-quantize.exe'),
    path.join(process.env.HOME || '', 'llama.cpp', 'build', 'bin', 'Release', 'llama-quantize'),
  ].filter(Boolean)

  return likelyBinaries.find(candidate => fs.existsSync(candidate)) || null
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    stdio: 'inherit',
    ...options,
  })
  if (result.error) {
    throw result.error
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with ${result.status}`)
  }
}

function commandOutput(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    encoding: 'utf8',
    ...options,
  })
  if (result.error) {
    throw result.error
  }
  return {
    status: result.status,
    stdout: result.stdout || '',
    stderr: result.stderr || '',
  }
}

function ollamaModelExists(tag) {
  const result = commandOutput('ollama', ['list'])
  if (result.status !== 0) {
    return false
  }
  return result.stdout
    .split(/\r?\n/)
    .some(line => line.trim().startsWith(`${tag} `) || line.trim() === tag)
}

function installPythonPackage(envPython, args) {
  run(envPython, ['-m', 'pip', 'install', ...args])
}

function ensurePythonEnvironment(basePython, converter) {
  const envPython = pythonEnvExecutable()
  if (basePython.kind === 'uv' && fs.existsSync(envPython) && !fs.existsSync(pythonEnvReadyMarker) && !pythonEnvUsesPreferredVersion()) {
    fs.rmSync(pythonEnvDir, { recursive: true, force: true })
  }

  if (!fs.existsSync(envPython)) {
    fs.mkdirSync(modelRoot, { recursive: true })
    if (basePython.kind === 'uv') {
      run(basePython.command, ['venv', '--python', preferredUvPython, pythonEnvDir])
    } else {
      run(basePython.command, ['-m', 'venv', pythonEnvDir])
    }
  }

  if (!fs.existsSync(pythonEnvReadyMarker)) {
    if (basePython.kind === 'uv') {
      run(basePython.command, ['pip', 'install', '--python', envPython, 'huggingface_hub'])
    } else {
      installPythonPackage(envPython, ['--upgrade', 'pip'])
      installPythonPackage(envPython, ['huggingface_hub'])
    }

    if (converter.kind === 'python-script') {
      const requirementsPath =
        process.env.LLAMA_CPP_CONVERT_REQUIREMENTS ||
        path.join(path.dirname(converter.scriptPath), 'requirements', 'requirements-convert_hf_to_gguf.txt')

      if (!fs.existsSync(requirementsPath)) {
        throw new Error(`Missing llama.cpp converter requirements file: ${requirementsPath}`)
      }
      if (basePython.kind === 'uv') {
        run(basePython.command, [
          'pip',
          'install',
          '--python',
          envPython,
          '--index-strategy',
          'unsafe-best-match',
          '-r',
          requirementsPath,
        ])
      } else {
        installPythonPackage(envPython, ['-r', requirementsPath])
      }
    }

    fs.writeFileSync(pythonEnvReadyMarker, new Date().toISOString())
  }

  return envPython
}

function ensureTooling() {
  const required = ['ollama']
  const missing = required.filter(tool => !commandExists(tool))
  const basePython = resolvePythonCommand()
  const converter = resolveLlamaConverter()
  const quantize = resolveLlamaQuantize()
  if (!basePython) {
    missing.push('python')
  }
  if (!converter) {
    missing.push('llama-convert-hf-to-gguf or convert_hf_to_gguf.py')
  }
  if (!quantize) {
    missing.push('llama-quantize')
  }

  if (missing.length) {
    throw new Error(
      [
        `Missing required model setup tools: ${missing.join(', ')}`,
        'Install llama.cpp tools and ensure llama-quantize is on PATH or set LLAMA_CPP_QUANTIZE to llama-quantize.',
        'For conversion, either put llama-convert-hf-to-gguf on PATH or set LLAMA_CPP_CONVERT_HF_TO_GGUF to convert_hf_to_gguf.py.',
        'The script stops here to avoid mixing unmatched quantizations.',
      ].join('\n'),
    )
  }

  const python = ensurePythonEnvironment(basePython, converter)
  return { converter, python, quantize }
}

function writeManifest(plan, completedModels) {
  const manifestPath = path.join(projectRoot, plan.modelDir, 'manifest.json')
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true })
  fs.writeFileSync(
    manifestPath,
    JSON.stringify(
      {
        createdAt: new Date().toISOString(),
        quantization: plan.quantization,
        models: completedModels,
      },
      null,
      2,
    ),
  )
}

function materializeModel(model, tooling) {
  const hfDir = path.join(projectRoot, model.hfLocalDir)
  const ggufDir = path.join(projectRoot, 'twin-eval-models', 'gguf')
  const ollamaDir = path.join(projectRoot, 'twin-eval-models', 'ollama')
  const rawGguf = path.join(projectRoot, model.rawGgufPath)
  const quantizedGguf = path.join(projectRoot, model.quantizedGgufPath)
  const modelfilePath = path.join(projectRoot, model.modelfilePath)

  fs.mkdirSync(hfDir, { recursive: true })
  fs.mkdirSync(ggufDir, { recursive: true })
  fs.mkdirSync(ollamaDir, { recursive: true })

  if (!fs.existsSync(rawGguf) && !fs.existsSync(quantizedGguf)) {
    run(tooling.python, [
      '-c',
      [
        'from huggingface_hub import snapshot_download',
        'import sys',
        'snapshot_download(',
        '    repo_id=sys.argv[1],',
        '    local_dir=sys.argv[2],',
        '    allow_patterns=["*.json", "*.jinja", "*.safetensors", "*.txt", "*.md"],',
        ')',
      ].join('\n'),
      model.sourceRepo,
      hfDir,
    ])
    if (tooling.converter.kind === 'python-script') {
      run(tooling.python, [tooling.converter.scriptPath, hfDir, '--outfile', rawGguf, '--outtype', 'bf16'])
    } else {
      run(tooling.converter.command, [hfDir, '--outfile', rawGguf, '--outtype', 'bf16'])
    }
  } else {
    console.log(`Skipping download/convert for ${model.key}; GGUF already exists.`)
  }

  if (!fs.existsSync(quantizedGguf)) {
    run(tooling.quantize, [rawGguf, quantizedGguf, model.quantization])
  } else {
    console.log(`Skipping quantize for ${model.key}; quantized GGUF already exists.`)
  }

  fs.writeFileSync(modelfilePath, `FROM ${quantizedGguf.replace(/\\/g, '/')}\n`)
  if (!ollamaModelExists(model.ollamaTag)) {
    run('ollama', ['create', model.ollamaTag, '-f', modelfilePath])
  } else {
    console.log(`Skipping Ollama create for ${model.key}; ${model.ollamaTag} already exists.`)
  }

  return {
    key: model.key,
    sourceRepo: model.sourceRepo,
    ollamaTag: model.ollamaTag,
    quantization: model.quantization,
    artifactPath: quantizedGguf,
    installed: true,
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const registryUrl = pathToFileURL(path.join(__dirname, 'twin-eval-model-registry.mjs'))
  const { buildSetupPlan } = await import(registryUrl.href)
  const plan = buildSetupPlan({ execute: args.execute, selectedKeys: args.selectedKeys })

  console.log(JSON.stringify(plan, null, 2))
  if (args.dryRun) {
    console.log('\nDry run only. Use npm run twin-eval:models:setup to download, convert, quantize, and create Ollama tags.')
    return
  }

  const tooling = ensureTooling()
  const completed = []
  for (const model of plan.models) {
    completed.push(materializeModel(model, tooling))
  }
  writeManifest(plan, completed)
}

main().catch(error => {
  console.error(error.message)
  process.exit(1)
})
