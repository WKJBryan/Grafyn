import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const readRepoFile = (path) => readFileSync(resolve(process.cwd(), '..', path), 'utf8')

describe('desktop shell isolation', () => {
  it('passes the signed desktop config relative to the tauri-action projectPath', () => {
    const workflow = readRepoFile('.github/workflows/release-smoke.yml')

    expect(workflow).toContain('projectPath: frontend')
    expect(workflow).toContain('config_path="src-tauri/tauri.desktop.conf.json"')
    expect(workflow).not.toContain('config_path="frontend/src-tauri/tauri.desktop.conf.json"')
  })

  it('compiles and registers MCP invoke commands only for desktop targets', () => {
    const modules = readRepoFile('frontend/src-tauri/src/commands/mod.rs')
    const shell = readRepoFile('frontend/src-tauri/src/lib.rs')

    expect(modules).toMatch(/#\[cfg\(desktop\)\]\s+pub mod mcp;/)
    expect(shell).toMatch(/#\[cfg\(desktop\)\]\s+commands::mcp::get_mcp_status/)
    expect(shell).toMatch(/#\[cfg\(desktop\)\]\s+commands::mcp::get_mcp_config_snippet/)
  })

  it('keeps updater and relaunch permissions in the desktop-only capability', () => {
    const shell = readRepoFile('frontend/src-tauri/src/lib.rs')
    const capability = JSON.parse(readRepoFile('frontend/src-tauri/capabilities/desktop.json'))

    expect(shell).toMatch(/#\[cfg\(all\(desktop, feature = "desktop-updater"\)\)\]\s+let builder = builder\.plugin\(tauri_plugin_updater/)
    expect(shell).toMatch(/#\[cfg\(all\(desktop, feature = "desktop-process"\)\)\]\s+let builder = builder\.plugin\(tauri_plugin_process::init\(\)\)/)
    expect(capability.platforms).toEqual(['linux', 'macOS', 'windows'])
    expect(capability.permissions).toContain('process:default')
    expect(capability.permissions).toContain('updater:default')
  })
})
