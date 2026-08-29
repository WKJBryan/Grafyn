import { describe, expect, it, vi } from 'vitest'

const plugins = vi.hoisted(() => ({
  open: vi.fn(),
  openUrl: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: plugins.open }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: plugins.openUrl }))

import { tauriTransport } from '@/api/tauriTransport'

describe('Tauri transport external opening', () => {
  it('routes URL and native-file-dialog requests through one operation', async () => {
    const options = { multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] }

    await tauriTransport.openExternal({ type: 'url', url: 'https://grafyn.app/docs' })
    await tauriTransport.openExternal({ type: 'file-dialog', options })

    expect(plugins.openUrl).toHaveBeenCalledWith('https://grafyn.app/docs')
    expect(plugins.open).toHaveBeenCalledWith(options)
  })
})
