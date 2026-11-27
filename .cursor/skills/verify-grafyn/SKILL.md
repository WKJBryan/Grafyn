---
name: verify-grafyn
description: Verify WKJBryan/Grafyn against a real grafyn-mcp process before opening a pull request. Use when changing notes, search, wikilinks, import, the MCP server, or other user-facing Grafyn flows, and when a cloud agent must start the app, exercise core flows, capture proof, and check regressions.
---

# Verify Grafyn

Read [feature-map.md](feature-map.md) before changing behavior. It is the map of user-facing flows and the invariants below.

## What a cloud agent can run

Grafyn's window is a Tauri 1.8 binary. Compiling it needs `libwebkit2gtk-4.0-dev` (Ubuntu 22.04). Ubuntu 24.04 cloud images do not ship WebKitGTK 4.0, so a default `cargo build` fails before any window opens. CI compiles that binary on `ubuntu-22.04` only.

The process that does run on a cloud VM is `grafyn-mcp`. It is the binary Claude Desktop launches, and it uses the same note store, Tantivy index, graph index, and importers as the desktop app. The live check drives that process over stdio against a throwaway vault.

`npm run dev` in a browser is not that check. Boot calls `get_boot_status` through Tauri `invoke`. A plain browser has no IPC handler, so the splash stays on `data-phase="failed"` and the note flows never run. The Playwright suite in `e2e/` has the same dependency: it needs `npm run tauri:dev` as the IPC backend. See `e2e/README.md`.

## Run the live check

From the repo root. Needs Node 20+, rustup, and a C toolchain that can see OpenSSL (`pkg-config` and `libssl-dev` on Debian/Ubuntu):

```bash
sudo apt-get install -y pkg-config libssl-dev   # once, if pkg-config cannot find openssl
rustup toolchain install 1.93.1                 # once; this is the toolchain CI pins
bash .cursor/skills/verify-grafyn/scripts/verify.sh
```

The wrapper:

1. Refuses to continue when toolchain `1.93.1` is missing, or when `pkg-config` cannot find OpenSSL.
2. Builds only `grafyn-mcp`, with `--locked --no-default-features --features mcp`. It does not build the Tauri app and does not need WebKit.
3. Starts the binary with `--vault` and `--data` under a new directory in the system temp folder. It does not read or write `~/Documents/Grafyn`.
4. Speaks newline-delimited JSON-RPC (`protocolVersion` `2025-03-26`) and runs the flows in the feature map that this binary exposes.
5. Prints a proof block. Exit code 0 means every check passed. The temp directory is removed after the run.

`CARGO_BUILD_JOBS` overrides parallelism. The wrapper defaults to 2 so a memory-ballooned VM is less likely to OOM while Tantivy compiles. `GRAFYN_VERIFY_KEEP=1` leaves the temp vault in place when you need the files from a failed run.

The exercise script is `.cursor/skills/verify-grafyn/scripts/exercise-app.mjs`. Run it directly only when you already have a binary:

```bash
node .cursor/skills/verify-grafyn/scripts/exercise-app.mjs \
  frontend/src-tauri/target/debug/grafyn-mcp
```

## What a passing proof contains

Paste the proof into the PR, from the first `PASS` line through `VERIFICATION PASSED`. A good run names the binary and the temp vault, then passes each of these:

- The process initializes. `serverInfo` is currently the rmcp default (`rmcp 0.15.0`) because `get_info()` does not override it. The instructions string is what identifies the process as Grafyn, and the proof checks for that. The vault tools are advertised.
- Create, read, and update move a note through `draft` → `evidence` → `canonical`, and the markdown file on disk matches.
- `[[Harbor Note]] (supports)` is an outgoing `supports` edge. The backlink on Harbor Note is the reverse relation `related`.
- `[[Phantom Note]]` written only inside a code fence does not become an edge, even though a Phantom Note file exists.
- Search and recall find the note by a unique token. Deleting a note removes the file and the search hit.
- Status `published` is rejected and does not write a file.
- A ChatGPT export imports as an `evidence` note with `source`, `created_via`, and `migration_source` in the frontmatter.
- `search_chunks` reports that the chunk index is absent. That index is built by the desktop app, so this message is the expected result for an MCP-only vault.
- A second `grafyn-mcp` on the same data directory falls back to read-only search and warns that the index may be stale.
- No `*.tmp-*` files remain from atomic writes.

## Regression suites

Run the row that matches the diff. These sit beside the live check. They do not replace it.

| Change | Command | Where it runs |
| --- | --- | --- |
| Rust services, models, or MCP | `cd frontend/src-tauri && cargo +1.93.1 test --locked --no-default-features --features mcp` | Cloud VM, no WebKit. Covers service tests compiled into the MCP binary, including twin store tests. It does not compile `commands/`, which is the Tauri binary. |
| Vue, Pinia, or the API client | `cd frontend && npm ci && npm run test:run` | Cloud VM. |
| Tauri commands, canvas streaming, or the desktop window | CI's `cargo test` on `ubuntu-22.04`, or locally: `cd frontend && npm run prepare:sidecar`, `mkdir -p dist && echo '<html></html>' > dist/index.html`, then `cd src-tauri && cargo test` | Needs WebKitGTK 4.0. |
| Desktop UI with a real window | Terminal 1: `cd frontend && npm run tauri:dev`. Terminal 2: `cd frontend && npm run e2e` | Manual. Needs a display and the IPC backend. Not part of CI. |

Canvas, debate, and twin answers call a model. Do not send those during verification. They spend the user's OpenRouter or Ollama account. Unit tests cover them with a mocked client.

## When the desktop window is available

Use a display (`xvfb-run` when the machine has no screen) and a vault that is not the user's real one. `npm run tauri:dev` is the process that makes `invoke()` work. The e2e project starts Vite on port 5173; interactive specs time out against that page alone.

## Before opening the PR

1. Run `verify.sh` on the branch you are about to push.
2. Run the regression row that matches the diff.
3. Put the `VERIFICATION PASSED` block in the PR body, and name the commit you ran it against.
4. When a check fails, fix the product or fix the check. Do not drop an assertion to force exit 0.

## Failures worth recognizing

- `openssl.pc` / `libssl-dev` missing while compiling `grafyn-mcp`: install `pkg-config` and `libssl-dev`, then re-run. The wrapper exits with that apt command before it launches Cargo.
- `webkit2gtk-4.0` missing while building the default features: you started the Tauri binary. Build `grafyn-mcp` with `--no-default-features --features mcp` instead.
- Splash `data-phase="failed"` in a browser: there is no IPC backend. That page is not the live check.
- `search_chunks` says the chunk index is not available: expected for a vault this script just created.
- Second server logs `Falling back to read-only search`: expected. That is the writer-lock invariant.
- `cargo test` (default features) on Ubuntu 24.04: the Tauri crate needs the 22.04 WebKit package. Use the `--features mcp` command above, or CI.
