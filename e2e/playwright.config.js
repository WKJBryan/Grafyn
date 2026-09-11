import { randomBytes } from 'node:crypto'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig, devices } from '@playwright/test'

const e2eDir = path.dirname(fileURLToPath(import.meta.url))
const frontendDir = path.resolve(e2eDir, '../frontend')
const tauriDir = path.resolve(frontendDir, 'src-tauri')
const inheritedRuntimeToken = process.env.GRAFYN_E2E_RUNTIME_TOKEN
if (inheritedRuntimeToken !== undefined && !/^[a-f0-9]{64}$/.test(inheritedRuntimeToken)) {
  throw new Error('runtime token must be 64 lowercase hexadecimal characters')
}
const runtimeToken = inheritedRuntimeToken ?? randomBytes(32).toString('hex')
const runtimeUrl = 'http://127.0.0.1:18890'
const frontendUrl = 'http://127.0.0.1:5173'
const openRouterUrl = 'http://127.0.0.1:18891/api/v1'
const runtimeRoot = path.resolve(
  e2eDir,
  'test-results',
  `runtime-${process.pid}-${randomBytes(8).toString('hex')}`,
)

process.env.GRAFYN_E2E_RUNTIME_TOKEN = runtimeToken
process.env.GRAFYN_E2E_RUNTIME_URL = runtimeUrl
process.env.GRAFYN_E2E_OPENROUTER_URL = openRouterUrl

export default defineConfig({
  testDir: './tests',
  outputDir: './test-results/artifacts',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: [
    ['html', { open: 'never' }],
    ['list'],
  ],
  timeout: 120_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: frontendUrl,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'on-first-retry',
  },
  projects: [
    {
      name: 'desktop-chromium',
      testMatch: 'desktop-smoke.spec.js',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1440, height: 900 },
      },
    },
    {
      name: 'companion-pixel',
      testMatch: 'companion-mobile.spec.js',
      use: { ...devices['Pixel 5'] },
    },
  ],
  webServer: [
    {
      command: 'node fixtures/openrouter-stub.js',
      cwd: e2eDir,
      url: 'http://127.0.0.1:18891/health',
      reuseExistingServer: false,
      timeout: 30_000,
      env: {
        ...process.env,
        GRAFYN_E2E_RUNTIME_TOKEN: runtimeToken,
        GRAFYN_E2E_OPENROUTER_PORT: '18891',
        GRAFYN_E2E_OPENROUTER_KEY: 'grafyn-e2e-key',
      },
    },
    {
      command: 'cargo run --locked --features e2e-test-runtime --bin grafyn-test-runtime',
      cwd: tauriDir,
      url: `${runtimeUrl}/health`,
      reuseExistingServer: false,
      timeout: 300_000,
      env: {
        ...process.env,
        GRAFYN_E2E_RUNTIME_ROOT: runtimeRoot,
        GRAFYN_E2E_RUNTIME_PORT: '18890',
        GRAFYN_E2E_RUNTIME_ORIGIN: frontendUrl,
        GRAFYN_E2E_RUNTIME_TOKEN: runtimeToken,
        GRAFYN_E2E_OPENROUTER_URL: openRouterUrl,
      },
    },
    {
      command: 'npm run dev -- --host 127.0.0.1 --port 5173 --strictPort',
      cwd: frontendDir,
      url: frontendUrl,
      reuseExistingServer: false,
      timeout: 120_000,
      env: {
        ...process.env,
        VITE_GRAFYN_E2E_RUNTIME_URL: runtimeUrl,
        VITE_GRAFYN_E2E_RUNTIME_TOKEN: runtimeToken,
      },
    },
  ],
})
