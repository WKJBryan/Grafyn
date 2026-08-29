import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { platform } from '@tauri-apps/plugin-os'
import { createRuntimeProfile } from '@/platform/runtime'

export const tauriTransport = {
  invoke,
  listen,
  getRuntimeProfile: () => createRuntimeProfile({
    isTauri: isTauri(),
    platform: isTauri() ? platform() : null,
  }),
  async open(options) {
    const { open } = await import('@tauri-apps/plugin-dialog')
    return open(options)
  },
  async openUrl(url) {
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    return openUrl(url)
  },
  async show() {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow')
    return getCurrentWebviewWindow().show()
  },
}
