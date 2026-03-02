# Phase 6: CI/CD Update

## Goal
Add PyInstaller build step to the release pipeline. Bundle the Python sidecar (`grafyn-server`) in the Tauri app build. Update the 4-platform build matrix.

## Why This Is Tricky
PyInstaller builds are platform-specific (can't cross-compile). Each matrix job needs Python + pip + PyInstaller + all dependencies. The Python sidecar binary also needs platform-specific naming for Tauri's sidecar resolution.

---

## Current Pipeline

```
create-release → build (4-job matrix) → publish-release → upload-to-r2 → build-summary
```

### Current build steps per platform:
1. Setup Node.js 20
2. Setup Rust + cache
3. Install Linux deps (if linux)
4. Install frontend deps (npm ci)
5. Validate/regenerate icons
6. **Build MCP binary** (cargo build --release --bin grafyn-mcp)
7. **Build Tauri app** (tauri-action)
8. Upload artifacts (workflow_dispatch only)

## Updated Pipeline

Same structure, but each build job gains a **Python sidecar build** step between steps 6 and 7:

```
create-release → build (4-job matrix) → publish-release → upload-to-r2 → build-summary
                    ↑ now includes PyInstaller step
```

---

## Task 1: Add Python + PyInstaller to Build Matrix

### New step: Setup Python

```yaml
      # ========================================
      # Setup Python (for sidecar build)
      # ========================================
      - name: Setup Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'
          cache: 'pip'
          cache-dependency-path: backend/requirements.txt

      - name: Install Python dependencies
        working-directory: backend
        run: |
          pip install -r requirements.txt
          pip install pyinstaller
```

### New step: Build Python sidecar

```yaml
      # ========================================
      # Build Python Sidecar (grafyn-server)
      # ========================================
      - name: Build Python sidecar
        working-directory: backend
        shell: bash
        run: |
          pyinstaller grafyn-server.spec --noconfirm

          # Tauri sidecar naming convention: binary-{target}{.exe}
          EXT=""
          if [[ "${{ matrix.os_name }}" == "windows" ]]; then
            EXT=".exe"
          fi

          # Copy built binary to Tauri's binaries/ directory
          mkdir -p ../frontend/src-tauri/binaries

          # PyInstaller --onedir output
          BUILT="dist/grafyn-server/grafyn-server${EXT}"
          TARGET="grafyn-server-${{ matrix.target }}${EXT}"

          if [ -f "$BUILT" ]; then
            cp "$BUILT" "../frontend/src-tauri/binaries/${TARGET}"
            echo "Built sidecar: ${TARGET}"
            ls -lh "../frontend/src-tauri/binaries/${TARGET}"
          else
            echo "ERROR: Built binary not found at $BUILT"
            ls -la dist/grafyn-server/
            exit 1
          fi
```

**Important:** For `--onedir` mode (directory distribution), Tauri v1 sidecar doesn't support directory bundles natively. Options:

### Option A: `--onefile` mode (simplest, slower startup)
Change the PyInstaller spec to produce a single executable:
```python
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,    # Include binaries IN the exe
    a.zipfiles,
    a.datas,
    name='grafyn-server',
    # ...
)
# Remove COLLECT step
```
Pro: Single file, Tauri handles it like grafyn-mcp. Con: Slower cold start (~3-5s unpacking).

### Option B: Directory mode + zip as resource
Bundle the PyInstaller dist directory as a Tauri resource, extract on first run. More complex but faster startup.

**Recommendation:** Start with Option A (`--onefile`). If cold start is too slow, switch to Option B later.

---

## Task 2: Update `tauri.conf.json` (in CI context)

The sidecar binary must exist before `tauri-action` runs. The step order ensures this:

```
1. Build MCP binary     → binaries/grafyn-mcp-{target}{.exe}
2. Build Python sidecar → binaries/grafyn-server-{target}{.exe}
3. Build Tauri app      → bundles both into installer
```

### `tauri.conf.json` change (already done in Phase 2):
```json
"externalBin": [
  "binaries/grafyn-mcp",
  "binaries/grafyn-server"
]
```

Tauri automatically appends the platform target triple and extension when resolving sidecar binaries.

---

## Task 3: Handle Large Binary Size

The PyInstaller binary with `sentence-transformers` + `torch` is ~300-500MB. This affects:

### CI build time:
- PyInstaller needs to download/install torch (~2GB pip install)
- Build step adds ~5-10 minutes per platform
- **Mitigation:** Use pip cache (`cache: 'pip'`) to avoid re-downloading

### Release size:
- Installer goes from ~30MB to ~350-550MB
- Auto-update delta is large for first sidecar update
- **Mitigation:** Consider stripping torch extras:
  ```bash
  pip install torch --index-url https://download.pytorch.org/whl/cpu
  ```
  CPU-only torch is ~150MB instead of ~2GB

### R2 storage:
- 4 platforms × ~400MB = ~1.6GB per release
- **Mitigation:** Acceptable for early releases, optimize later

---

## Task 4: Platform-Specific Considerations

### Windows (x64 + ARM64)
- PyInstaller works natively on x64
- ARM64 Windows: Need ARM64 Python + PyInstaller (available since Python 3.11)
- torch CPU on ARM64 Windows: May need special wheel

### macOS (ARM64)
- PyInstaller works natively on Apple Silicon
- `sentence-transformers` installs via pip without issues
- Universal binary not needed (Rosetta 2 handles x64)
- **Note:** macOS may require code signing for the sidecar binary

### Linux (x64)
- Straightforward PyInstaller build
- May need `--strip` flag to reduce binary size
- AppImage already bundles everything, so sidecar fits naturally

---

## Task 5: Updated Release Workflow

```yaml
name: Build & Release

on:
  push:
    tags: ['v*']
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g., v1.0.0)'
        required: false
        default: 'dev'

jobs:
  create-release:
    # ... unchanged ...

  build:
    needs: create-release
    if: always() && (needs.create-release.result == 'success' || !startsWith(github.ref, 'refs/tags/v'))

    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: windows-latest
            target: x86_64-pc-windows-msvc
            arch: x64
            os_name: windows
          - platform: windows-latest
            target: aarch64-pc-windows-msvc
            arch: arm64
            os_name: windows
          - platform: macos-latest
            target: aarch64-apple-darwin
            arch: arm64
            os_name: macos
          - platform: ubuntu-22.04
            target: x86_64-unknown-linux-gnu
            arch: x64
            os_name: linux

    runs-on: ${{ matrix.platform }}

    steps:
      - uses: actions/checkout@v4

      # ===== Node.js =====
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'
          cache-dependency-path: frontend/package-lock.json

      # ===== Rust =====
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: 'frontend/src-tauri -> target'
          key: ${{ matrix.target }}

      # ===== Python (NEW) =====
      - uses: actions/setup-python@v5
        with:
          python-version: '3.11'
          cache: 'pip'
          cache-dependency-path: backend/requirements.txt

      - name: Install Python dependencies
        working-directory: backend
        shell: bash
        run: |
          # CPU-only torch to keep binary small
          pip install torch --index-url https://download.pytorch.org/whl/cpu
          pip install -r requirements.txt
          pip install pyinstaller

      # ===== Linux Deps =====
      - name: Install Linux dependencies
        if: matrix.os_name == 'linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.0-dev \
            libappindicator3-dev librsvg2-dev patchelf

      # ===== Frontend =====
      - name: Install frontend dependencies
        working-directory: frontend
        run: npm ci

      - name: Validate and regenerate icons
        working-directory: frontend
        shell: bash
        run: |
          if node -e "
            const fs = require('fs');
            const ico = 'src-tauri/icons/icon.ico';
            if (!fs.existsSync(ico)) process.exit(1);
            const magic = fs.readFileSync(ico).slice(0, 4).toString('hex');
            if (magic !== '00000100') process.exit(1);
          "; then
            echo "Icons are valid"
          else
            echo "Regenerating icons..."
            node scripts/generate-icons.cjs
            npx tauri icon src-tauri/icons/app-icon.png
          fi

      # ===== Build MCP Binary =====
      - name: Build MCP binary
        working-directory: frontend/src-tauri
        shell: bash
        run: |
          EXT=""
          if [[ "${{ matrix.os_name }}" == "windows" ]]; then EXT=".exe"; fi
          cargo build --release --bin grafyn-mcp --no-default-features --features mcp --target ${{ matrix.target }}
          mkdir -p binaries
          cp "target/${{ matrix.target }}/release/grafyn-mcp${EXT}" "binaries/grafyn-mcp-${{ matrix.target }}${EXT}"

      # ===== Build Python Sidecar (NEW) =====
      - name: Build Python sidecar
        working-directory: backend
        shell: bash
        run: |
          pyinstaller grafyn-server.spec --noconfirm

          EXT=""
          if [[ "${{ matrix.os_name }}" == "windows" ]]; then EXT=".exe"; fi

          BUILT="dist/grafyn-server/grafyn-server${EXT}"
          TARGET="grafyn-server-${{ matrix.target }}${EXT}"

          mkdir -p ../frontend/src-tauri/binaries
          cp "$BUILT" "../frontend/src-tauri/binaries/${TARGET}"
          echo "Built sidecar: ${TARGET} ($(du -sh ../frontend/src-tauri/binaries/${TARGET} | cut -f1))"

      # ===== Build Tauri App =====
      - name: Build Tauri app
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_PRIVATE_KEY: ${{ secrets.TAURI_PRIVATE_KEY }}
          TAURI_KEY_PASSWORD: ''
          GITHUB_FEEDBACK_REPO: ${{ secrets.FEEDBACK_REPO }}
          GITHUB_FEEDBACK_TOKEN: ${{ secrets.FEEDBACK_TOKEN }}
        with:
          projectPath: frontend
          tauriScript: npx tauri
          args: --target ${{ matrix.target }}
          releaseId: ${{ needs.create-release.outputs.release_id }}
          releaseBody: ${{ needs.create-release.outputs.release_notes }}
          updaterJsonPreferNsis: true

  # ... publish-release, upload-to-r2, build-summary unchanged ...
```

---

## Task 6: Update `version:bump` Script

The version bump script (`npm run version:bump`) should remain unchanged — it updates `package.json`, `tauri.conf.json`, and `Cargo.toml`. No Python version files to update.

---

## Files Modified
| File | Action |
|------|--------|
| `.github/workflows/release.yml` | **Edit** — add Python setup + PyInstaller build step |
| `backend/grafyn-server.spec` | **Created in Phase 2** — may need --onefile adjustment |

## Validation
- `workflow_dispatch` build succeeds on all 4 platforms
- Built installers include both `grafyn-mcp` and `grafyn-server` binaries
- Installed app starts sidecar successfully
- Auto-update works (larger binary, but `latest.json` still valid)
- Build time increase is acceptable (<10min per platform with pip cache)

## Future Optimizations
1. **ONNX Runtime instead of PyTorch**: Replace `sentence-transformers` with `onnxruntime` for embeddings → binary drops from ~400MB to ~80MB
2. **Pre-built wheels**: Cache PyInstaller dist in GitHub Actions artifacts for faster CI
3. **Separate sidecar installer**: Ship sidecar as optional download, auto-fetch on first desktop launch
