#!/usr/bin/env bash
set -eu
cd "$(dirname "$0")/../.."
[ -f fixtures/generated/attack.epub ] || cargo run --release -p jadeslip-fixtures --bin gen-fixtures
CARGO_TARGET_DIR=target/e2e cargo build --release -p jadeslip --features e2e
CARGO_TARGET_DIR=target/e2e cargo build --release -p jadeslip-worker
cargo build --release -p jadeslip -p jadeslip-worker
cargo test --release -p jadeslip-e2e --no-run
exec tests/e2e/headless.sh cargo test --release --no-fail-fast -p jadeslip-e2e ${SS_E2E_TEST:+--test "$SS_E2E_TEST"} -- --ignored --test-threads=1 --nocapture "$@"
