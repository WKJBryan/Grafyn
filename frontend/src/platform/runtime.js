export const RUNTIME_PROFILES = Object.freeze({
  DESKTOP_WIDE: 'desktop-wide',
  ANDROID_COMPACT: 'android-compact',
  PLAIN_WEB: 'plain-web',
})

const MOBILE_PLATFORMS = new Set(['android', 'ios'])

export function createRuntimeProfile({ isTauri = false, platform = null } = {}) {
  if (!isTauri) {
    return Object.freeze({
      name: RUNTIME_PROFILES.PLAIN_WEB,
      isTauri: false,
      platform: null,
    })
  }

  return Object.freeze({
    name: MOBILE_PLATFORMS.has(platform)
      ? RUNTIME_PROFILES.ANDROID_COMPACT
      : RUNTIME_PROFILES.DESKTOP_WIDE,
    isTauri: true,
    platform,
  })
}
