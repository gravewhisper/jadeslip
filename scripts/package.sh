#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
stamp=$(mktemp)
trap 'rm -f "$stamp"' EXIT
triple=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release --locked -p jadeslip-worker
install -D -m 755 target/release/jadeslip-worker "app/binaries/jadeslip-worker-$triple"
cd app
cargo tauri build --bundles appimage,deb --config tauri.bundle.json -- --locked
cd ..
rm -rf target/linux
mkdir -p target/linux
find target/release/bundle -maxdepth 2 \( -name '*.AppImage' -o -name '*.deb' \) -newer "$stamp" -exec cp {} target/linux/ \;
(cd target/linux && sha256sum -- *.AppImage *.deb > SHA256SUMS)
ls -l target/linux
