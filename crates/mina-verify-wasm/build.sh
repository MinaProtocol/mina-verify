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

echo ">> done. pkg/:"
ls -la pkg/
