import { RUNTIME_PROFILES } from './runtime'
import { getRuntimeProfile } from '@/api/transport'

export const CAPABILITY_NAMES = Object.freeze([
  'notesRead',
  'notesWrite',
  'recall',
  'twinReview',
  'twinChat',
  'linearCanvas',
  'imageGeneration',
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

const CAPABILITIES_BY_PROFILE = Object.freeze({
  [RUNTIME_PROFILES.DESKTOP_WIDE]: Object.freeze({
    ...unavailableCapabilities,
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview: true,
    twinChat: true,
    linearCanvas: true,
    spatialCanvas: true,
    nativeVaultPicker: true,
    importByPath: true,
    localOllama: true,
    mcp: true,
    vaultMigration: true,
    optimizerAdmin: true,
    desktopUpdater: true,
  }),
  [RUNTIME_PROFILES.ANDROID_COMPACT]: unavailableCapabilities,
  [RUNTIME_PROFILES.PLAIN_WEB]: unavailableCapabilities,
})

export class CapabilityUnavailableError extends Error {
  constructor(capability, profile) {
    super(`${capability} is unavailable in the ${profile.name} runtime`)
    this.name = 'CapabilityUnavailableError'
    this.code = 'CAPABILITY_UNAVAILABLE'
    this.capability = capability
    this.profile = profile.name
  }
}

export function hasCapability(profile, capability) {
  return CAPABILITIES_BY_PROFILE[profile?.name]?.[capability] === true
}

export function assertCapability(capability, profile = getRuntimeProfile()) {
  if (!hasCapability(profile, capability)) {
    throw new CapabilityUnavailableError(capability, profile)
  }
}
