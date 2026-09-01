import { afterEach, describe, expect, it } from 'vitest'
import { RUNTIME_PROFILES, createRuntimeProfile } from '@/platform/runtime'
import {
  CapabilityUnavailableError,
  CAPABILITY_NAMES,
  assertCapability,
  hasCapability,
  normalizeRuntimeStatus,
} from '@/platform/capabilities'
import { resetTransport, setRuntimeStatus } from '@/api/transport'

describe('runtime capability profiles', () => {
  const expectedDesktopCapabilities = {
    notesRead: true,
    notesWrite: true,
    recall: true,
    twinReview: true,
    twinChat: true,
    linearCanvas: true,
    imageGeneration: true,
    nativeImageShare: false,
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

  afterEach(() => {
    resetTransport()
  })

  it('uses exactly the canonical capability vocabulary', () => {
    expect(CAPABILITY_NAMES).toEqual(Object.keys(expectedDesktopCapabilities))
  })

  it.each(['windows', 'macos', 'linux', 'freebsd', 'openbsd', 'netbsd', 'solaris'])(
    'gives the %s Tauri runtime the wide desktop profile',
    (platform) => {
      const profile = createRuntimeProfile({ isTauri: true, platform })
      setRuntimeStatus(normalizeRuntimeStatus(desktopRuntimeStatus()))

      expect(profile.name).toBe(RUNTIME_PROFILES.DESKTOP_WIDE)
      for (const [capability, enabled] of Object.entries(expectedDesktopCapabilities)) {
        expect(hasCapability(profile, capability)).toBe(enabled)
      }
    },
  )

  it('keeps desktop capabilities unavailable until matching backend health is installed', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'windows' })

    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }

    setRuntimeStatus(normalizeRuntimeStatus(runtimeStatus()))
    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }

    setRuntimeStatus(normalizeRuntimeStatus(desktopRuntimeStatus()))
    expect(hasCapability(profile, 'importByPath')).toBe(true)
    expect(hasCapability(profile, 'mcp')).toBe(true)
    expect(hasCapability(profile, 'desktopUpdater')).toBe(true)
  })

  it('keeps iOS compact but unavailable', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'ios' })

    expect(profile.name).toBe(RUNTIME_PROFILES.IOS_COMPACT)
    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }
  })

  it('keeps Android fail closed until typed backend health is installed', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'android' })

    expect(profile.name).toBe(RUNTIME_PROFILES.ANDROID_COMPACT)
    for (const capability of Object.keys(expectedDesktopCapabilities)) {
      expect(hasCapability(profile, capability)).toBe(false)
    }
  })

  it('admits Android local-core capabilities and gates secret/native services by health', () => {
    const profile = createRuntimeProfile({ isTauri: true, platform: 'android' })
    setRuntimeStatus(normalizeRuntimeStatus(runtimeStatus({
      secureSecrets: {
        status: 'unavailable',
        code: 'keystore_unavailable',
        message: 'Android Keystore is unavailable.',
      },
      nativeImageShare: {
        status: 'unavailable',
        code: 'share_unavailable',
        message: 'Native image sharing is unavailable.',
      },
    })))

    for (const capability of [
      'notesRead',
      'notesWrite',
      'recall',
      'twinReview',
      'twinChat',
      'linearCanvas',
    ]) {
      expect(hasCapability(profile, capability)).toBe(true)
    }
    for (const capability of ['imageGeneration', 'nativeImageShare', 'sync']) {
      expect(hasCapability(profile, capability)).toBe(false)
    }

    setRuntimeStatus(normalizeRuntimeStatus(runtimeStatus()))
    expect(hasCapability(profile, 'imageGeneration')).toBe(true)
    expect(hasCapability(profile, 'nativeImageShare')).toBe(false)
    expect(hasCapability(profile, 'sync')).toBe(true)
  })

  it('rejects malformed runtime health instead of partially enabling Android', () => {
    expect(() => normalizeRuntimeStatus({
      ...runtimeStatus(),
      schemaVersion: 2,
    })).toThrow('Unsupported runtime status')
    expect(() => normalizeRuntimeStatus({
      ...runtimeStatus(),
      capabilities: { notesRead: true },
    })).toThrow('Invalid runtime capabilities')
    expect(() => normalizeRuntimeStatus({
      ...runtimeStatus(),
      secureSecrets: { status: 'ready', code: 'should-be-null', message: null },
    })).toThrow('Invalid secure secret health')
  })

  it.each([
    [{ extra: true }, 'top-level'],
    [{ vault: { kind: 'app_private', available: true, path: 'private' } }, 'vault'],
    [{
      secureSecrets: {
        status: 'ready',
        code: null,
        message: null,
        fallback: true,
      },
    }, 'health'],
    [{
      diagnostics: [{ code: 'offline_ready', message: 'Ready.', privatePath: 'private' }],
    }, 'diagnostic'],
  ])('rejects unknown %s RuntimeStatusV1 keys', (override) => {
    expect(() => normalizeRuntimeStatus(runtimeStatus(override)))
      .toThrow('Invalid runtime status shape')
  })

  it.each([
    [{ status: 'unavailable', code: null, message: 'Unavailable.' }],
    [{ status: 'unavailable', code: '', message: 'Unavailable.' }],
    [{ status: 'unavailable', code: 'keystore_unavailable', message: null }],
    [{ status: 'unavailable', code: 'keystore_unavailable', message: '   ' }],
    [{ status: 'unavailable', code: 'x'.repeat(81), message: 'Unavailable.' }],
    [{ status: 'unavailable', code: 'keystore_unavailable', message: 'x'.repeat(241) }],
  ])('requires bounded nonempty unavailable-health code and message', (secureSecrets) => {
    expect(() => normalizeRuntimeStatus(runtimeStatus({ secureSecrets })))
      .toThrow('Invalid secure secret health')
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

function runtimeStatus(overrides = {}) {
  return {
    schemaVersion: 1,
    runtime: 'android',
    capabilities: {
      notesRead: true,
      notesWrite: true,
      recall: true,
      twinReview: true,
      twinChat: true,
      linearCanvas: true,
      imageGeneration: true,
      nativeImageShare: true,
      sync: true,
      spatialCanvas: false,
      nativeVaultPicker: false,
      importByPath: false,
      localOllama: false,
      mcp: false,
      vaultMigration: false,
      optimizerAdmin: false,
      desktopUpdater: false,
    },
    vault: { kind: 'app_private', available: true },
    secureSecrets: { status: 'ready', code: null, message: null },
    nativeImageShare: { status: 'ready', code: null, message: null },
    diagnostics: [],
    ...overrides,
  }
}

function desktopRuntimeStatus(overrides = {}) {
  return runtimeStatus({
    runtime: 'desktop',
    capabilities: Object.fromEntries(CAPABILITY_NAMES.map(name => [name, true])),
    vault: { kind: 'user_selected', available: true },
    nativeImageShare: {
      status: 'unavailable',
      code: 'desktop_save_as',
      message: 'Desktop uses Save As.',
    },
    ...overrides,
  })
}
