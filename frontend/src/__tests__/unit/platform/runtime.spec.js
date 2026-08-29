import { describe, expect, it } from 'vitest'
import { RUNTIME_PROFILES, createRuntimeProfile } from '@/platform/runtime'
import {
  CapabilityUnavailableError,
  CAPABILITY_NAMES,
  assertCapability,
  hasCapability,
} from '@/platform/capabilities'

describe('runtime capability profiles', () => {
  const expectedDesktopCapabilities = {
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview: true,
    twinChat: true,
    linearCanvas: false,
    imageGeneration: false,
    sync: false,
    spatialCanvas: true,
    nativeVaultPicker: true,
    importByPath: true,
    localOllama: true,
    mcp: true,
    vaultMigration: true,
    optimizerAdmin: true,
    desktopUpdater: true,
  }

  it('uses exactly the canonical capability vocabulary', () => {
    expect(CAPABILITY_NAMES).toEqual(Object.keys(expectedDesktopCapabilities))
  })

  it.each(['windows', 'macos', 'linux', 'freebsd', 'openbsd', 'netbsd', 'solaris'])(
    'gives the %s Tauri runtime the wide desktop profile',
    (platform) => {
      const profile = createRuntimeProfile({ isTauri: true, platform })

      expect(profile.name).toBe(RUNTIME_PROFILES.DESKTOP_WIDE)
      for (const [capability, enabled] of Object.entries(expectedDesktopCapabilities)) {
        expect(hasCapability(profile, capability)).toBe(enabled)
      }
    },
  )

  it.each(['android', 'ios'])('keeps the %s Tauri runtime compact', (platform) => {
    const profile = createRuntimeProfile({ isTauri: true, platform })

    expect(profile.name).toBe(RUNTIME_PROFILES.ANDROID_COMPACT)
    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }
  })

  it('fails closed with a typed error for unavailable and unknown capabilities', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'android' })

    expect(() => assertCapability('desktopUpdater', profile)).toThrow(CapabilityUnavailableError)
    expect(() => assertCapability('desktopUpdater', profile)).toThrow('desktopUpdater is unavailable')
    expect(hasCapability(profile, 'not-a-capability')).toBe(false)
    expect(hasCapability(profile, 'ollama')).toBe(false)
    expect(hasCapability(profile, 'pathImport')).toBe(false)
    expect(() => assertCapability('not-a-capability', profile)).toThrow(CapabilityUnavailableError)
  })

  it('uses the plain-web profile when Tauri is unavailable', () => {
    const profile = createRuntimeProfile({ isTauri: false, platform: 'windows' })

    expect(profile.name).toBe(RUNTIME_PROFILES.PLAIN_WEB)
    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }
  })
})
