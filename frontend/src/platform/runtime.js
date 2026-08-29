export const RUNTIME_PROFILES = Object.freeze({
  DESKTOP_WIDE: 'desktop-wide',
  ANDROID_COMPACT: 'android-compact',
  PLAIN_WEB: 'plain-web',
})

const DESKTOP_PLATFORMS = new Set(['windows', 'macos', 'linux'])

export function createRuntimeProfile({ isTauri = false, platform = null } = {}) {
  if (!isTauri) {
    return Object.freeze({
      name: RUNTIME_PROFILES.PLAIN_WEB,
      isTauri: false,
      platform: null,
    })
  }

  return Object.freeze({
    name: DESKTOP_PLATFORMS.has(platform)
      ? RUNTIME_PROFILES.DESKTOP_WIDE
      : RUNTIME_PROFILES.ANDROID_COMPACT,
    isTauri: true,
    platform,
  })
}
