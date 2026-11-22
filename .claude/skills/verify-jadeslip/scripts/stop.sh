#!/usr/bin/env bash
set -u
SKILL="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$SKILL/../../.." && pwd)"
RUN="$(readlink -f "${1:-${JSV_RUN:-$ROOT/target/verify-app/current}}" 2>/dev/null)"
if [ -z "$RUN" ] || [ ! -d "$RUN/evidence" ]; then
    echo "stop: no run folder at ${1:-target/verify-app/current}" >&2
    exit 2
fi
pid_of() { cat "$RUN/$1" 2>/dev/null; }
alive() { local p; p=$(pid_of "$1"); [ -n "$p" ] && kill -0 "$p" 2>/dev/null; }
if alive app.pid && [ -S "$RUN/ctl.sock" ]; then
    JSV_RUN="$RUN" timeout 5 python3 "$SKILL/scripts/ctl.py" quit >/dev/null 2>&1
fi
for _ in $(seq 1 100); do alive headless.pid || break; sleep 0.1; done
if alive headless.pid; then
    kill -TERM "$(pid_of headless.pid)"
    for _ in $(seq 1 100); do alive headless.pid || break; sleep 0.1; done
fi
status=0
if alive headless.pid; then echo "stop: headless.sh pid $(pid_of headless.pid) did not exit" >&2; status=1; fi
if alive app.pid; then echo "stop: app pid $(pid_of app.pid) did not exit" >&2; status=1; fi
APP="$(pid_of app.pid)"
if [ -n "$APP" ] && [ "$(hyprctl clients -j | jq --argjson pid "$APP" '[.[] | select(.pid == $pid)] | length')" != 0 ]; then
    echo "stop: the run's window is still open" >&2
    status=1
fi
if [ "$status" = 0 ]; then
    rm -f "$(readlink -f "$RUN/ctl.sock")"
    rm -rf "$RUN/data" "$RUN/tmp" "$RUN/xdg" "$RUN/books" "$RUN/ctl.sock"
    [ "$(readlink -f "$ROOT/target/verify-app/current")" = "$RUN" ] && rm -f "$ROOT/target/verify-app/current"
fi
echo "evidence kept: $RUN/evidence ($(ls "$RUN/evidence" | wc -l) files)"
exit "$status"
