import { RUNTIME_PROFILES } from './runtime'
import { getRuntimeProfile } from '@/api/transport'

const CAPABILITIES_BY_PROFILE = Object.freeze({
  [RUNTIME_PROFILES.DESKTOP_WIDE]: new Set([
    'mcp',
    'ollama',
    'pathImport',
    'migration',
    'optimizer',
    'updater',
    'spatialCanvas',
    'windowShow',
  ]),
  [RUNTIME_PROFILES.ANDROID_COMPACT]: new Set(),
  [RUNTIME_PROFILES.PLAIN_WEB]: new Set(),
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
  return CAPABILITIES_BY_PROFILE[profile?.name]?.has(capability) ?? false
}

export function assertCapability(capability, profile = getRuntimeProfile()) {
  if (!hasCapability(profile, capability)) {
    throw new CapabilityUnavailableError(capability, profile)
  }
}
