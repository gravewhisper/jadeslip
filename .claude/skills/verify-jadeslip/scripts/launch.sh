#!/usr/bin/env bash
set -eu
SKILL="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$SKILL/../../.." && pwd)"
cd "$ROOT"
RUNS="$ROOT/target/verify-app"
LOCK="${XDG_RUNTIME_DIR:-/tmp}/jadeslip-headless.lock"
BUILD=1
BOOK=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-build) BUILD=0 ;;
        --book) BOOK="$2"; shift ;;
        *) echo "usage: launch.sh [--book <epub>] [--no-build]" >&2; exit 2 ;;
    esac
    shift
done
for tool in hyprctl jq setsid flock python3 cargo; do
    command -v "$tool" >/dev/null || { echo "launch: $tool is required" >&2; exit 2; }
done
if [ -e "$RUNS/current/headless.pid" ] && kill -0 "$(cat "$RUNS/current/headless.pid")" 2>/dev/null; then
    echo "launch: run $(readlink "$RUNS/current") is still up; drive it, or stop it with stop.sh first" >&2
    exit 3
fi
if ! flock -n "$LOCK" true; then
    echo "launch: another headless run holds $LOCK (a test suite or scripts/verify.sh); wait for it, never start beside it" >&2
    exit 3
fi
if [ -n "$BOOK" ] && [ ! -f "$BOOK" ]; then
    echo "launch: no book at $BOOK" >&2
    exit 2
fi
if [ "$BUILD" = 1 ]; then
    [ -f fixtures/generated/attack.epub ] || cargo run --release -q -p jadeslip-fixtures --bin gen-fixtures
    CARGO_TARGET_DIR=target/e2e cargo build --release -q -p jadeslip --features e2e
    CARGO_TARGET_DIR=target/e2e cargo build --release -q -p jadeslip-worker
fi
BIN="$ROOT/target/e2e/release/jadeslip"
[ -x "$BIN" ] || { echo "launch: $BIN is missing; run without --no-build" >&2; exit 2; }
RUN="$RUNS/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN/data" "$RUN/tmp" "$RUN/xdg" "$RUN/books" "$RUN/evidence"
SOCK="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/jsv-$$-$(basename "$RUN").sock"
ln -sfn "$SOCK" "$RUN/ctl.sock"
ARG=""
if [ -n "$BOOK" ]; then
    cp "$BOOK" "$RUN/books/"
    ARG="$RUN/books/$(basename "$BOOK")"
fi
git rev-parse HEAD > "$RUN/commit"
git status --porcelain > "$RUN/dirty"
ln -sfn "$(basename "$RUN")" "$RUNS/current"
setsid "$ROOT/tests/e2e/headless.sh" bash -c '
    echo "$$" > "$0/app.pid"
    exec env TMPDIR="$0/tmp" JADESLIP_E2E_SOCKET="$3" JADESLIP_DATA_DIR="$0/data" XDG_DATA_HOME="$0/xdg" JADESLIP_E2E_OFF= "$1" ${2:+"$2"} >"$0/app.log" 2>&1
' "$RUN" "$BIN" "$ARG" "$SOCK" >"$RUN/headless.log" 2>&1 </dev/null &
echo "$!" > "$RUN/headless.pid"
if ! python3 "$SKILL/scripts/ctl.py" ready 60; then
    echo "launch: the app did not become usable; see $RUN/app.log and $RUN/headless.log, then run stop.sh" >&2
    exit 1
fi
echo "run: $RUN"
echo "book: ${ARG:-none (library)}"
echo "evidence: $RUN/evidence"
