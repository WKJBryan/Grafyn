export const RUNTIME_PROFILES = Object.freeze({
  DESKTOP_WIDE: 'desktop-wide',
  ANDROID_COMPACT: 'android-compact',
  IOS_COMPACT: 'ios-compact',
  PLAIN_WEB: 'plain-web',
})

export function createRuntimeProfile({
  isTauri = false,
  platform = null,
  nativePlugins = isTauri,
} = {}) {
  if (!isTauri) {
    return Object.freeze({
      name: RUNTIME_PROFILES.PLAIN_WEB,
      isTauri: false,
      platform: null,
      nativePlugins: false,
    })
  }

  let name = RUNTIME_PROFILES.DESKTOP_WIDE
  if (platform === 'android') name = RUNTIME_PROFILES.ANDROID_COMPACT
  if (platform === 'ios') name = RUNTIME_PROFILES.IOS_COMPACT

  return Object.freeze({
    name,
    isTauri: true,
    platform,
    nativePlugins: nativePlugins === true,
  })
}
