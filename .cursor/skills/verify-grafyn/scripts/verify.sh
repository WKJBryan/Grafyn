#!/usr/bin/env bash
# Build grafyn-mcp (no Tauri / no WebKit) and drive it against a throwaway vault.
# See ../SKILL.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required. Install the 1.93.1 toolchain, then re-run." >&2
  exit 1
fi

if ! rustup toolchain list | grep -q '^1\.93\.1'; then
  echo "Rust 1.93.1 is required (the toolchain CI pins)." >&2
  echo "Install it with: rustup toolchain install 1.93.1" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "Node 20+ is required to drive the MCP session." >&2
  exit 1
fi

if ! command -v pkg-config >/dev/null 2>&1 || ! pkg-config --exists openssl; then
  echo "OpenSSL development files are required to compile grafyn-mcp." >&2
  echo "On Debian/Ubuntu: sudo apt-get install -y pkg-config libssl-dev" >&2
  exit 1
fi

cd "$ROOT/frontend/src-tauri"
cargo +1.93.1 build --locked --bin grafyn-mcp --no-default-features --features mcp

exec node "$ROOT/.cursor/skills/verify-grafyn/scripts/exercise-app.mjs" \
  "$ROOT/frontend/src-tauri/target/debug/grafyn-mcp"
