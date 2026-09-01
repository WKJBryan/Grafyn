import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const readFrontendFile = path => readFileSync(resolve(process.cwd(), path), 'utf8')
const readJson = path => JSON.parse(readFrontendFile(path))

describe('Android shell isolation', () => {
  it('keeps desktop window policy out of the shared config', () => {
    const shared = readJson('src-tauri/tauri.conf.json')
    const desktop = readJson('src-tauri/tauri.desktop.conf.json')
    const android = readJson('src-tauri/tauri.android.conf.json')

    expect(shared.app.windows).toBeUndefined()
    expect(desktop.app.windows).toEqual([
      expect.objectContaining({
        label: 'main',
        minWidth: 800,
        minHeight: 600,
        visible: false,
      }),
    ])
    expect(android.app.windows).toEqual([
      expect.objectContaining({
        label: 'main',
        title: 'Grafyn',
        visible: true,
      }),
    ])
    expect(android.app.windows[0]).not.toHaveProperty('minWidth')
    expect(android.app.windows[0]).not.toHaveProperty('minHeight')
    expect(android.app.security.capabilities).toEqual(['mobile'])
    expect(android.app.security.csp).not.toContain('grafyn-updater')
    expect(android.bundle.android.minSdkVersion).toBe(24)
  })

  it('grants Android only core, platform detection, and URL opening', () => {
    const capability = readJson('src-tauri/capabilities/mobile.json')

    expect(capability.platforms).toEqual(['android'])
    expect(capability.windows).toEqual(['main'])
    expect(capability.permissions).toEqual([
      'core:default',
      'os:allow-platform',
      'opener:allow-open-url',
      'opener:allow-default-urls',
    ])
    expect(capability.permissions.join(' ')).not.toMatch(/dialog|process|updater/)
  })

  it('uses the reviewed package identity and a narrowly scoped FileProvider', () => {
    const gradle = readFrontendFile('src-tauri/gen/android/app/build.gradle.kts')
    const manifest = readFrontendFile('src-tauri/gen/android/app/src/main/AndroidManifest.xml')
    const filePaths = readFrontendFile('src-tauri/gen/android/app/src/main/res/xml/file_paths.xml')

    expect(gradle).toContain('applicationId = "com.grafyn.app"')
    expect(gradle).toContain('minSdk = 24')
    expect(manifest).toContain('android:authorities="${applicationId}.grafyn.share"')
    expect(manifest).toContain('android:allowBackup="false"')
    expect(manifest).toContain('android:exported="false"')
    expect(manifest).toContain('android:grantUriPermissions="true"')
    expect(filePaths).toContain('<cache-path name="grafyn_generated_images" path="Grafyn/grafyn-share-v1/" />')
    expect(filePaths).not.toContain('<external-path')
    expect(filePaths).not.toMatch(/path="\."/)
  })

  it('runs the exact ARM64 Android contract-test variant in CI', () => {
    const workflow = readFileSync(resolve(process.cwd(), '../.github/workflows/test.yml'), 'utf8')

    expect(workflow).toContain('bash ./gradlew --no-daemon testArm64DebugUnitTest')
    expect(workflow).not.toContain('./gradlew --no-daemon testDebugUnitTest')
  })
})
