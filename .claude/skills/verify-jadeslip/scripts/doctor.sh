#!/usr/bin/env bash
set -u
SKILL="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$SKILL/../../.." && pwd)"
RUN="$(readlink -f "${JSV_RUN:-$ROOT/target/verify-app/current}" 2>/dev/null)"
LOCK="${XDG_RUNTIME_DIR:-/tmp}/jadeslip-headless.lock"
BIN="$ROOT/target/e2e/release/jadeslip"
bad=0
say() { printf '%-5s %s\n' "$1" "$2"; [ "$1" = FAIL ] && bad=1; return 0; }
if [ -z "$RUN" ] || [ ! -d "$RUN" ]; then
    say FAIL "no run folder; start one with launch.sh"
    exit 1
fi
say INFO "run $RUN"
alive() { [ -f "$RUN/$1" ] && kill -0 "$(cat "$RUN/$1")" 2>/dev/null; }
if alive headless.pid; then say OK "headless.sh pid $(cat "$RUN/headless.pid") is up"; else say FAIL "headless.sh is not running (see $RUN/headless.log)"; fi
if alive app.pid; then
    APP="$(cat "$RUN/app.pid")"
    if [ "$(readlink -f "/proc/$APP/exe" 2>/dev/null)" = "$(readlink -f "$BIN")" ]; then
        say OK "app pid $APP runs $BIN"
    else
        say FAIL "app pid $APP is not $BIN"
    fi
else
    say FAIL "the app is not running (see $RUN/app.log)"
    APP=""
fi
if [ -n "$APP" ]; then
    MON=$(hyprctl clients -j | jq -r --argjson pid "$APP" '.[] | select(.pid == $pid) | .monitor')
    NAME=$(hyprctl monitors all -j | jq -r --argjson id "${MON:--1}" '.[] | select(.id == $id) | .name')
    if [ "$NAME" = "SS-TEST" ]; then say OK "window is on the headless output SS-TEST"; else say FAIL "window is on '${NAME:-nowhere}', not SS-TEST"; fi
fi
if REPLY=$(timeout 5 python3 "$SKILL/scripts/ctl.py" eval "location.href" 2>&1); then
    say OK "control socket answers, page $REPLY"
else
    say FAIL "control socket: $REPLY"
fi
COMMIT="$(cat "$RUN/commit" 2>/dev/null)"
if [ "$COMMIT" = "$(git -C "$ROOT" rev-parse HEAD)" ]; then say OK "run was launched at HEAD ${COMMIT:0:7}"; else say WARN "run was launched at ${COMMIT:0:7}, HEAD is now $(git -C "$ROOT" rev-parse --short HEAD)"; fi
NEWER=$(cd "$ROOT" && git ls-files -co --exclude-standard app crates/core crates/worker | while read -r f; do [ -f "$f" ] && [ "$f" -nt "$BIN" ] && echo "$f"; done | head -3)
if [ -z "$NEWER" ]; then say OK "binary is newer than every app/core/worker source"; else say WARN "binary is older than: $(echo $NEWER)"; fi
if flock -n "$LOCK" true; then say WARN "headless lock is free, so no headless.sh owns the output"; else say OK "headless lock is held"; fi
exit "$bad"
