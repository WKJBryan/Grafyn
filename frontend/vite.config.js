import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'
import { readFileSync } from 'node:fs'

const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf-8'))

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [vue()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
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
    target: process.env.TAURI_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    // Don't minify for debug builds. vite 8 bundles with Rolldown and keeps
    // esbuild out of the default tree, so use the built-in (oxc) minifier
    // rather than 'esbuild' (which would pull a still-flagged esbuild back in).
    minify: !process.env.TAURI_DEBUG,
    // Produce sourcemaps for debug builds
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
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
