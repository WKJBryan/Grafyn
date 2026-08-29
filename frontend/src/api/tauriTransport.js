import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { platform } from '@tauri-apps/plugin-os'
import { createRuntimeProfile } from '@/platform/runtime'

export function getTauriRuntimeProfile() {
  const tauriRuntime = isTauri()
  return createRuntimeProfile({
    isTauri: tauriRuntime,
    platform: tauriRuntime ? platform() : null,
  })
}

export const tauriTransport = {
  invoke,
  listen,
  async openExternal(request) {
    if (request.type === 'file-dialog') {
      const { open } = await import('@tauri-apps/plugin-dialog')
      return open(request.options)
    }

    if (request.type === 'url') {
      const { openUrl } = await import('@tauri-apps/plugin-opener')
      return openUrl(request.url)
    }

    throw new Error(`Unsupported external open request: ${request.type}`)
  },
  async showMainWindow() {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow')
    return getCurrentWebviewWindow().show()
  },
}
