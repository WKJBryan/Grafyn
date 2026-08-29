import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { createRequire } from 'node:module'
import { fileURLToPath, URL } from 'node:url'
import {
  assertTwinEvalIsolation,
  resolveViteInputs,
} from './scripts/twin-eval-isolation.mjs'

const twinEvalLabEnabled = process.env.GRAFYN_TWIN_EVAL_LAB === '1'
const require = createRequire(import.meta.url)
const pkg = require('./package.json')

function twinEvalIsolationPlugin() {
  return {
    name: 'grafyn-twin-eval-isolation',
    generateBundle(_options, bundle) {
      if (!twinEvalLabEnabled) {
        assertTwinEvalIsolation(bundle)
      }
    },
  }
}

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [vue(), twinEvalIsolationPlugin()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __TWIN_EVAL_LAB__: JSON.stringify(twinEvalLabEnabled),
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  // Prevent vite from obscuring Rust errors
  clearScreen: false,
  server: {
    port: 5173,
    // Tauri expects a fixed port
    strictPort: true,
  },
  // Env variables starting with TAURI_ are exposed to Tauri's API
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    // Tauri uses Chromium on Windows and WebKit on macOS/Linux
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    // Don't minify for debug builds. vite 8 bundles with Rolldown and keeps
    // esbuild out of the default tree, so use the built-in (oxc) minifier
    // rather than 'esbuild' (which would pull a still-flagged esbuild back in).
    minify: !process.env.TAURI_ENV_DEBUG,
    // Produce sourcemaps for debug builds
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: resolveViteInputs(__dirname, twinEvalLabEnabled),
      output: {
        // vite 8 bundles with Rolldown, which only accepts the function form of
        // manualChunks (the object map form is Rollup-only and throws).
        manualChunks(id) {
          if (!id.includes('node_modules')) return
          if (/[\\/]node_modules[\\/](vue|vue-router|pinia)[\\/]/.test(id)) return 'vendor'
          if (/[\\/]node_modules[\\/]marked[\\/]/.test(id)) return 'markdown'
        },
      }
    },
  },
})
