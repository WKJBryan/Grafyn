import { RUNTIME_PROFILES } from './runtime'
import { getRuntimeProfile, getRuntimeStatus } from '@/api/transport'

export const CAPABILITY_NAMES = Object.freeze([
  'notesRead',
  'notesWrite',
  'recall',
  'twinReview',
  'twinChat',
  'linearCanvas',
  'imageGeneration',
  'nativeImageShare',
  'sync',
  'spatialCanvas',
  'nativeVaultPicker',
  'importByPath',
  'localOllama',
  'mcp',
  'vaultMigration',
  'optimizerAdmin',
  'desktopUpdater',
])

const unavailableCapabilities = Object.freeze(
  Object.fromEntries(CAPABILITY_NAMES.map((name) => [name, false])),
)

const androidCandidateCapabilities = Object.freeze({
  ...unavailableCapabilities,
  notesRead: true,
  notesWrite: true,
  recall: true,
  twinReview: true,
  twinChat: true,
  linearCanvas: true,
  imageGeneration: true,
  sync: true,
})

const CAPABILITIES_BY_PROFILE = Object.freeze({
  [RUNTIME_PROFILES.DESKTOP_WIDE]: Object.freeze({
    ...unavailableCapabilities,
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview: true,
    twinChat: true,
    linearCanvas: true,
    imageGeneration: true,
    spatialCanvas: true,
    nativeVaultPicker: true,
    importByPath: true,
    localOllama: true,
    mcp: true,
    vaultMigration: true,
    optimizerAdmin: true,
    desktopUpdater: true,
  }),
  [RUNTIME_PROFILES.ANDROID_COMPACT]: androidCandidateCapabilities,
  [RUNTIME_PROFILES.IOS_COMPACT]: unavailableCapabilities,
  [RUNTIME_PROFILES.PLAIN_WEB]: unavailableCapabilities,
})

const RUNTIME_KINDS = new Set(['desktop', 'android'])
const VAULT_KINDS = new Set(['user_selected', 'app_private'])
const HEALTH_STATES = new Set(['ready', 'unavailable'])

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function hasExactKeys(value, expectedKeys) {
  const keys = Object.keys(value)
  return keys.length === expectedKeys.length
    && keys.every(key => expectedKeys.includes(key))
}

function boundedText(
  value,
  label,
  { nullable = false, max = 240, nonempty = false } = {},
) {
  if (nullable && value === null) return null
  if (typeof value !== 'string'
    || value.length > max
    || (nonempty && !value.trim())
    || [...value].some(character => {
      const codePoint = character.codePointAt(0)
      return codePoint <= 31 || codePoint === 127
    })) {
    throw new Error(`Invalid ${label}`)
  }
  return value
}

function normalizeHealth(value, label) {
  if (!isRecord(value) || !hasExactKeys(value, ['status', 'code', 'message'])) {
    throw new Error('Invalid runtime status shape')
  }
  if (!HEALTH_STATES.has(value.status)) {
    throw new Error(`Invalid ${label}`)
  }
  if (value.status === 'ready') {
    if (value.code !== null || value.message !== null) throw new Error(`Invalid ${label}`)
    return Object.freeze({ status: value.status, code: null, message: null })
  }
  const code = boundedText(value.code, label, { max: 80, nonempty: true })
  const message = boundedText(value.message, label, { nonempty: true })
  return Object.freeze({ status: value.status, code, message })
}

function normalizeCapabilities(value) {
  if (!isRecord(value)) throw new Error('Invalid runtime capabilities')
  const keys = Object.keys(value)
  if (keys.length !== CAPABILITY_NAMES.length
    || !CAPABILITY_NAMES.every(name => typeof value[name] === 'boolean')
    || keys.some(name => !CAPABILITY_NAMES.includes(name))) {
    throw new Error('Invalid runtime capabilities')
  }
  return Object.freeze(Object.fromEntries(
    CAPABILITY_NAMES.map(name => [name, value[name]]),
  ))
}

export function normalizeRuntimeStatus(value) {
  if (!isRecord(value) || value.schemaVersion !== 1 || !RUNTIME_KINDS.has(value.runtime)) {
    throw new Error('Unsupported runtime status')
  }
  if (!hasExactKeys(value, [
    'schemaVersion',
    'runtime',
    'capabilities',
    'vault',
    'secureSecrets',
    'nativeImageShare',
    'diagnostics',
  ])) {
    throw new Error('Invalid runtime status shape')
  }
  if (!isRecord(value.vault)) {
    throw new Error('Invalid runtime vault status')
  }
  if (!hasExactKeys(value.vault, ['kind', 'available'])) {
    throw new Error('Invalid runtime status shape')
  }
  if (!VAULT_KINDS.has(value.vault.kind)
    || typeof value.vault.available !== 'boolean') {
    throw new Error('Invalid runtime vault status')
  }
  if (!Array.isArray(value.diagnostics) || value.diagnostics.length > 32) {
    throw new Error('Invalid runtime diagnostics')
  }
  const diagnostics = value.diagnostics.map((diagnostic) => {
    if (!isRecord(diagnostic) || !hasExactKeys(diagnostic, ['code', 'message'])) {
      throw new Error('Invalid runtime status shape')
    }
    return Object.freeze({
      code: boundedText(diagnostic.code, 'runtime diagnostic code', {
        max: 80,
        nonempty: true,
      }),
      message: boundedText(diagnostic.message, 'runtime diagnostic message', {
        nonempty: true,
      }),
    })
  })

  return Object.freeze({
    schemaVersion: 1,
    runtime: value.runtime,
    capabilities: normalizeCapabilities(value.capabilities),
    vault: Object.freeze({ kind: value.vault.kind, available: value.vault.available }),
    secureSecrets: normalizeHealth(value.secureSecrets, 'secure secret health'),
    nativeImageShare: normalizeHealth(value.nativeImageShare, 'native image share health'),
    diagnostics: Object.freeze(diagnostics),
  })
}

export class CapabilityUnavailableError extends Error {
  constructor(capability, profile) {
    super(`${capability} is unavailable in the ${profile.name} runtime`)
    this.name = 'CapabilityUnavailableError'
    this.code = 'CAPABILITY_UNAVAILABLE'
    this.capability = capability
    this.profile = profile.name
  }
}

export function hasCapability(profile, capability, status = getRuntimeStatus()) {
  const candidate = CAPABILITIES_BY_PROFILE[profile?.name]?.[capability] === true
  if (!candidate) return false

  if (profile.name === RUNTIME_PROFILES.DESKTOP_WIDE) {
    return status?.runtime === 'desktop'
      && status.capabilities?.[capability] === true
  }
  if (profile.name !== RUNTIME_PROFILES.ANDROID_COMPACT || status?.runtime !== 'android') {
    return false
  }
  if (status.capabilities?.[capability] !== true) return false
  if (capability === 'imageGeneration' || capability === 'sync') {
    return status.secureSecrets?.status === 'ready'
  }
  if (capability === 'nativeImageShare') {
    return status.nativeImageShare?.status === 'ready'
  }
  return true
}

export function assertCapability(capability, profile = getRuntimeProfile()) {
  if (!hasCapability(profile, capability)) {
    throw new CapabilityUnavailableError(capability, profile)
  }
}
