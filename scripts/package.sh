#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
triple=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release --locked -p jadeslip-worker
install -D -m 755 target/release/jadeslip-worker "app/binaries/jadeslip-worker-$triple"
cd app
cargo tauri build --bundles appimage,deb --config tauri.bundle.json -- --locked
find ../target/release/bundle -maxdepth 2 \( -name '*.AppImage' -o -name '*.deb' \) -print
