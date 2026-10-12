#!/bin/sh
# Builds maze.wasm from maze/ (needs: rustup target add wasm32-unknown-unknown), records the digest of the sources it
# was built from in maze.source-sha256 (tools/tests/test_generated_artifacts.py recomputes it) and prints the module's
# SHA-256 for program/@sha256 in maze.scene.xml.
set -eu
cd "$(dirname "$0")/maze"
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/sr_example_maze.wasm ../maze.wasm
# the digest of the sorted `sha256sum` lines of the sources, paths relative to maze/
find Cargo.toml Cargo.lock src -type f | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d' ' -f1 > ../maze.source-sha256
cd ..
sha256sum maze.wasm
