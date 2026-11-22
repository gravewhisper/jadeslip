#!/usr/bin/env bash
set -eu
S=$(cd "$(dirname "$0")" && pwd)
R=$(cd "$S/../.." && pwd)
cd "$R"
CARGO_TARGET_DIR=target/e2e cargo build --release -p spinesprint --features e2e
cargo build --release -p spinesprint -p spinesprint-worker
for c in 48183bb 70bb680; do
    cd "$S/wt-$c"
    CARGO_TARGET_DIR="$S/target-$c" cargo build --release -p spinesprint --features e2e
done
mkdir -p "$S/bin"
cp "$R/target/e2e/release/spinesprint" "$S/bin/spinesprint-head-e2e"
cp "$R/target/release/spinesprint" "$S/bin/spinesprint-head-plain"
for c in 48183bb 70bb680; do cp "$S/target-$c/release/spinesprint" "$S/bin/spinesprint-$c-e2e"; done
sha256sum "$S"/bin/*
