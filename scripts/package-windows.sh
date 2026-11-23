#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
image=jadeslip-wincross
triple=x86_64-pc-windows-msvc
cache="${JADESLIP_WIN_CACHE:-$HOME/.cache/jadeslip-windows}"
mkdir -p "$cache/target" "$cache/registry" "$cache/xwin"

if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker build -t "$image" - <<'EOF'
FROM rust:latest
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends mingw-w64 nsis clang lld llvm && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu x86_64-pc-windows-msvc && rustup component add clippy
RUN cargo install --locked cargo-xwin tauri-cli@^2 && rm -rf /usr/local/cargo/registry
EOF
fi

docker run --rm -m "${JADESLIP_WIN_MEMORY:-8g}" --cpus 6 -e CARGO_BUILD_JOBS="${JADESLIP_WIN_JOBS:-6}" --user "$(id -u):$(id -g)" \
    -e HOME=/tmp -e CARGO_HOME=/usr/local/cargo -e CARGO_TARGET_DIR=/target \
    -e XWIN_CACHE_DIR=/xwin \
    -v "$PWD:/src" -v "$cache/target:/target" \
    -v "$cache/registry:/usr/local/cargo/registry" -v "$cache/xwin:/xwin" \
    -w /src "$image" bash -euc "
        CARGO_TARGET_DIR=/target/worker RUSTFLAGS='-C target-feature=+crt-static' \\
            cargo xwin build --release --locked -p jadeslip-worker --target $triple
        install -D -m 755 /target/worker/$triple/release/jadeslip-worker.exe app/binaries/jadeslip-worker-$triple.exe
        cd app
        cargo tauri build --runner cargo-xwin --target $triple --bundles nsis --config tauri.bundle.json -- --locked
        for exe in /target/$triple/release/jadeslip.exe /target/worker/$triple/release/jadeslip-worker.exe; do
            if llvm-objdump -p \$exe | grep -iE 'DLL Name: (vcruntime|msvcp)'; then
                echo \"\$exe needs the Visual C++ runtime, which a fresh Windows does not have\" >&2
                exit 1
            fi
        done
    "
mkdir -p target/windows
cp "$cache/target/$triple/release/bundle/nsis/"*.exe target/windows/
ls -l target/windows
