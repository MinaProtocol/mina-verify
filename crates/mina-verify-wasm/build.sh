#!/usr/bin/env bash
# Build the mina-verify wasm package + JS bindings.
#
# Threaded wasm (mina-core pulls in wasm_thread; mina-tree uses rayon) => needs a
# NIGHTLY toolchain with rust-src, build-std, and atomics/shared-memory (the flags in
# ./.cargo/config.toml, mirrored from openmina's crates/node/web).
#
# One-time setup:
#   rustup toolchain install nightly
#   rustup component add rust-src --toolchain nightly
#   rustup target add wasm32-unknown-unknown --toolchain nightly
#   cargo install wasm-bindgen-cli --version 0.2.106   # match Cargo.lock's wasm-bindgen
#
# Usage: ./build.sh [nodejs|web|bundler]   (default: nodejs)
set -euo pipefail
cd "$(dirname "$0")"

TARGET_KIND="${1:-nodejs}"
WASM=../../target/wasm32-unknown-unknown/release/mina_verify_wasm.wasm

echo ">> cargo +nightly build --release --target wasm32-unknown-unknown"
cargo +nightly build --release --target wasm32-unknown-unknown

echo ">> wasm-bindgen --target $TARGET_KIND --out-dir pkg"
wasm-bindgen --target "$TARGET_KIND" --out-dir pkg "$WASM"

# wasm-bindgen's nodejs target doesn't emit a package.json; write one so pkg/ is a
# publishable npm package. (The web/bundler targets emit their own.) Keep the version
# in sync with Cargo.toml.
if [ "$TARGET_KIND" = "nodejs" ]; then
  VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
  echo ">> writing pkg/package.json (mina-verify-wasm@$VERSION)"
  cat > pkg/package.json <<JSON
{
  "name": "mina-verify-wasm",
  "version": "$VERSION",
  "description": "WebAssembly bindings for mina-verify: verify a Mina block's proof from JS/TS.",
  "license": "Apache-2.0",
  "type": "commonjs",
  "main": "mina_verify_wasm.js",
  "types": "mina_verify_wasm.d.ts",
  "files": [
    "mina_verify_wasm.js",
    "mina_verify_wasm.d.ts",
    "mina_verify_wasm_bg.wasm",
    "mina_verify_wasm_bg.wasm.d.ts"
  ]
}
JSON
fi

echo ">> done. pkg/:"
ls -la pkg/
