import { describe, expect, it } from 'vitest'
import { RUNTIME_PROFILES, createRuntimeProfile } from '@/platform/runtime'
import {
  CapabilityUnavailableError,
  assertCapability,
  hasCapability,
} from '@/platform/capabilities'

describe('runtime capability profiles', () => {
  it('gives a desktop Tauri runtime the wide profile', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'windows' })

    expect(profile.name).toBe(RUNTIME_PROFILES.DESKTOP_WIDE)
    expect(hasCapability(profile, 'mcp')).toBe(true)
    expect(hasCapability(profile, 'spatialCanvas')).toBe(true)
  })

  it('keeps Android Tauri IPC in the compact profile without desktop permissions', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'android' })

    expect(profile.name).toBe(RUNTIME_PROFILES.ANDROID_COMPACT)
    for (const capability of [
      'mcp',
      'ollama',
      'pathImport',
      'migration',
      'optimizer',
      'updater',
      'spatialCanvas',
    ]) {
      expect(hasCapability(profile, capability)).toBe(false)
    }
  })

  it('fails closed with a typed error for unavailable capabilities', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'android' })

    expect(() => assertCapability('updater', profile)).toThrow(CapabilityUnavailableError)
    expect(() => assertCapability('updater', profile)).toThrow('updater is unavailable')
  })

  it('uses the plain-web profile when Tauri is unavailable', () => {
    const profile = createRuntimeProfile({ isTauri: false, platform: 'windows' })

    expect(profile.name).toBe(RUNTIME_PROFILES.PLAIN_WEB)
    expect(hasCapability(profile, 'mcp')).toBe(false)
  })
})
