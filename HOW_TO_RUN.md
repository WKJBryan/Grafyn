# Grafyn — Run & Test (Tauri 2 Desktop + Android)

Grafyn is a companion-first Tauri 2 application: Vue 3 + Vite runs in a WebView over a shared Rust core, with separate desktop-wide and Android-compact command registrations. There is no web/Python backend and no hosted relay, account, billing, subscription, pairing, or recovery service.

## Common Prerequisites

- Node.js `^20.19.0` or `>=22.12.0`, and npm
- Rust installed through `rustup`
- The [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for the host OS

Run npm commands from `frontend/` unless a section says otherwise.

## Desktop Development

```bash
cd frontend
npm install
npm run tauri:dev
```

The desktop runtime keeps the wide shell and existing desktop capabilities, including the user-selected vault, spatial Canvas, native file dialogs/import, Ollama, MCP, migration/optimizer administration, updater integration, and one-shot image generation.

`OPENROUTER_API_KEY` is an optional desktop-only environment fallback. Android deliberately ignores it.

## Desktop Build

```bash
cd frontend
npm run tauri:build
```

This prepares the bundled MCP sidecar and writes desktop packages below `frontend/src-tauri/target/release/bundle/`. If `TAURI_PRIVATE_KEY` is unset, a local build skips the signed updater bundle while still producing normal installer artifacts.

## Android Prerequisites

Install these exact toolchain components:

- JDK 17
- Android SDK platform `android-36`
- Android Build Tools `35.0.0`
- Android NDK `28.2.13676358`
- Rust target `aarch64-linux-android`

Set `JAVA_HOME`, `ANDROID_HOME` (or `ANDROID_SDK_ROOT`), and `NDK_HOME` to the installed JDK, SDK, and exact NDK. Then install or confirm the required components:

```bash
sdkmanager "platforms;android-36" "build-tools;35.0.0" "ndk;28.2.13676358"
rustup target add aarch64-linux-android
```

## Android Debug APK

The canonical arm64 debug APK command is:

```bash
cd frontend
npm install
npx tauri android build --debug --target aarch64 --apk --ci
```

On Windows, the Tauri wrapper's final native-library handoff uses a symbolic link. If Developer Mode or `SeCreateSymbolicLinkPrivilege` is unavailable, the wrapper can finish the Rust/NDK build and then stop with `Creation symbolic link is not allowed for this system.` That is a host-policy boundary. The equivalent reviewed packaging fallback is to copy the exact built `target/aarch64-linux-android/debug/libgrafyn_lib.so` into `src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a/`, then run a clean `:app:assembleArm64Debug` while excluding `:app:rustBuildArm64Debug`. Hash the packaged `lib/arm64-v8a/libgrafyn_lib.so` and require it to match the source build; verify the resulting APK with SDK Build Tools `apksigner` before accepting it.

Do not treat the APK as verified merely because host compilation or unit tests pass. A current Android verification also requires installing the generated APK on an arm64 emulator or physical device, launching it, and exercising the native boundaries listed below.

## Android Runtime Contract

Android uses the compact companion shell and a bounded native command surface for local Capture, note CRUD, Recall, governed Twin review, secret-backed Twin chat, chronological one-model Canvas, and runtime/settings/sync status.

The typed backend `RuntimeStatusV1` is loaded before the router mounts and is authoritative. The frontend profile may remove capabilities but cannot add them. On Android:

- Canonical vault health gates notes read/write, Recall, Twin review, and linear Canvas.
- Twin chat also requires the native secure-secret adapter to be ready.
- Image generation and native image sharing are unavailable. The validated FileProvider/native share command remains dormant until Android has a production receipt producer or import path.
- Sync is reported only when canonical storage and secure secrets are healthy and a real `SyncEngine` was constructed.
- Spatial Canvas, native vault picking, arbitrary-path import, Ollama, MCP, vault migration, optimizer administration, and the desktop updater are unavailable.

Canonical event/root corruption or startup failure returns a recoverable failed boot state, disables the affected capabilities, and leaves the process alive for recovery UI. It must not serve a disposable/default Twin.

### Secrets and Degraded Offline Operation

Android uses one native Keystore-backed secret-store adapter for both OpenRouter/settings transitions and sync identity material. Every durable record contains a non-secret canonical account mapping plus nonce, ciphertext, and authentication tag; its filename prefix is bound to the account hash, and native health AEAD-authenticates every record with the exact account as associated data before reporting ready. Same-length ciphertext or tag tampering therefore fails closed instead of passing a shape-only health check. Grafyn never reads `OPENROUTER_API_KEY` on Android, never falls back to memory or plaintext secrets, and rejects a plaintext OpenRouter key in Android settings before accessing the native secret store.

If Keystore is unavailable, Grafyn keeps the app-private canonical vault and local mutation coordinator active. Notes, Capture, Recall, and governed Twin review remain local-only; secret-backed Twin chat and sync fail closed, and no `SyncEngine` is constructed. Native image share is gated independently by share-bridge health; a missing bridge or invalid share root disables sharing without disabling the local vault. No hosted fallback is implied.

## Storage Boundaries

Paths are intentionally described conceptually because Android app-private paths are not public API values:

- Desktop: a user-selected vault plus the OS-specific application config, data, and cache directories.
- Android app data: `Grafyn/config`, `Grafyn/data`, and `Grafyn/vault` below the application's private data root.
- Android app cache: `Grafyn/`, with FileProvider staging restricted to `Grafyn/grafyn-share-v1`.

Android does not use the public Documents directory or expose an app-private vault path through settings/runtime status. Share files are transient cache material, not canonical vault data.

## Frontend-Only Tasks

```bash
cd frontend
npm run lint
npm run test:run
npm run build
```

These commands verify Vue/Vite behavior only. They do not exercise Rust IPC, native file access, Android plugins, or installed-app lifecycle behavior.

## Rust Host Tests

```bash
cd frontend
npm run prepare:sidecar
cd src-tauri
cargo test
```

Focused tests may be run with `cargo test test_name`. On Windows, `cargo test -- --test-threads=4` reduces transient scanner/ACL pressure from filesystem-heavy tests.

Host Rust tests can verify target-independent contracts and cfg-selected compilation, but they do not prove Android JNI command registration, Keystore behavior, FileProvider URI grants, chooser behavior, WebView rendering, runtime permissions, process death, or lifecycle recovery.

## Android Native Verification Boundary

After building, install and launch the APK on an arm64 emulator or device. The native smoke boundary includes:

1. Cold start and recovery UI behavior with healthy and unavailable native bridge health.
2. App-private note capture, Recall, Twin review, and linear Canvas without storage permissions or a Documents path.
3. Keystore-backed OpenRouter/settings and sync-secret access, plus local-only behavior when Keystore is unavailable.
4. FileProvider staging from only the dedicated cache subtree and read-only URI permission behavior. A future invocation may report only that the share sheet opened; recipient delivery and chooser cancellation are not observable or claimed.
5. WebView safe areas, back navigation, rotation/recreation, background/foreground, and process-death recovery.

Host tests and a successful APK build are prerequisites for this boundary, not substitutes for it. This guide does not claim that the current APK or full suite has passed those checks.

## Windows Common Controls Safeguard

`frontend/src-tauri/build.rs` supplies the Microsoft Common Controls v6 manifest dependency to MSVC test harnesses so Windows can resolve `TaskDialogIndirect`. Tauri's generated resource already owns the desktop application manifest, so the `grafyn` binary is linked with `/MANIFEST:NO` to prevent a duplicate while test executables retain the activation dependency.

If Windows shows an “Entry Point Not Found” dialog for a stale `target/debug/deps/grafyn_lib-*.exe`, rebuild and run the harness through Cargo; do not launch the stale executable directly.

## Playwright UI Tests

```bash
cd e2e
npm install
npm run install-browsers
npm test
```

These tests cover the Vite UI surface. They do not provide Android native-plugin or installed-APK evidence.

## Quick Start

1. Install the common prerequisites and run `npm install` in `frontend/`.
2. For desktop, run `npm run tauri:dev`.
3. For Android, install the exact mobile toolchain and use the canonical debug APK command above.
4. Before describing a target as verified, run the relevant host suites and complete that target's native smoke boundary.
