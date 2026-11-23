#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
image=jadeslip-linuxbuild
cache="${JADESLIP_LINUX_CACHE:-$HOME/.cache/jadeslip-linux}"
mkdir -p "$cache/target" "$cache/registry"

if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker build -t "$image" - <<'EOF'
FROM ubuntu:22.04
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates curl wget file git xdg-utils \
    libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
RUN cargo install --locked tauri-cli@^2 && rm -rf /opt/cargo/registry && chmod -R a+rwX /opt/cargo /opt/rustup
EOF
fi

rm -rf target/linux
mkdir -p target/linux
docker run --rm -m "${JADESLIP_LINUX_MEMORY:-8g}" --cpus 6 -e CARGO_BUILD_JOBS="${JADESLIP_LINUX_JOBS:-6}" --user "$(id -u):$(id -g)" \
    -e HOME=/tmp -e APPIMAGE_EXTRACT_AND_RUN=1 -e NO_STRIP=1 \
    -v "$PWD:/src:ro" -v "$PWD/target/linux:/out" \
    -v "$cache/target:/cache/target" -v "$cache/registry:/opt/cargo/registry" \
    "$image" bash -euc "
        git config --global --add safe.directory '*'
        git clone -q /src /tmp/w
        ln -s /cache/target /tmp/w/target
        cd /tmp/w
        scripts/package.sh
        cp target/linux/* /out/
    "
ls -l target/linux
