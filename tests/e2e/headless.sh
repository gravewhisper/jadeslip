#!/usr/bin/env bash
set -u

if [ "$#" -eq 0 ]; then
    echo "usage: tests/e2e/headless.sh <command> [args...]" >&2
    exit 2
fi

if [ -n "${SS_HEADLESS_OUTPUT:-}" ]; then
    exec "$@"
fi

OUTPUT="SS-TEST"
CLASS="^jadeslip-test$"
RUNTIME="${XDG_RUNTIME_DIR:-/tmp}"
LOCK="$RUNTIME/jadeslip-headless.lock"
COMMAND_PID=""
WATCHDOG_PID=""
FOCUS_GUARD_PID=""
CLEANED=""

remaining_windows() {
    hyprctl clients -j | jq --arg class "$CLASS" '[.[] | select((.class | test($class)) or (.initialClass | test($class)))] | length'
}

has_output() {
    hyprctl monitors all -j | jq -e --arg name "$OUTPUT" 'any(.[]; .name == $name)' >/dev/null
}

close_remaining_windows() {
    hyprctl clients -j | jq -r --arg class "$CLASS" '.[] | select((.class | test($class)) or (.initialClass | test($class))) | .pid' |
        while read -r pid; do kill -KILL "$pid" 2>/dev/null; done
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        [ "$(remaining_windows)" = "0" ] && break
        sleep 0.1
    done
}

hypr_eval() {
    local reply
    reply=$(hyprctl eval "$1" 2>&1)
    if [ "$reply" != "ok" ]; then
        echo "headless: hyprctl eval failed: $reply" >&2
        return 1
    fi
}

output_workspace() {
    hyprctl monitors all -j | jq -r --arg name "$OUTPUT" '.[] | select(.name == $name) | .activeWorkspace.id | select(. > 0)'
}

focused_monitor() {
    hyprctl monitors -j | jq -r '.[] | select(.focused) | .name'
}

keep_owner_focus() {
    local owner
    owner=$(hyprctl monitors -j | jq -r --arg name "$OUTPUT" '[.[] | select(.name != $name)] | (map(select(.focused)) + .)[0].name')
    while kill -0 "$1" 2>/dev/null; do
        if [ "$(focused_monitor)" = "$OUTPUT" ]; then
            hyprctl eval "local cursor = hl.get_cursor_pos() hl.dispatch(hl.dsp.focus({ monitor = '$owner' })) if cursor.x < 20000 then hl.dispatch(hl.dsp.cursor.move({ x = cursor.x, y = cursor.y })) end" >/dev/null 2>&1
        fi
        sleep 0.1
    done
}

remove_output() {
    hyprctl --batch "eval SS_TEST_CURSOR = hl.get_cursor_pos() ; output remove $OUTPUT ; eval hl.dispatch(hl.dsp.cursor.move({ x = SS_TEST_CURSOR.x, y = SS_TEST_CURSOR.y }))" >/dev/null
}

cleanup() {
    [ -n "$CLEANED" ] && return
    CLEANED=1
    trap - EXIT INT TERM HUP
    [ -n "$WATCHDOG_PID" ] && kill -KILL "$WATCHDOG_PID" 2>/dev/null
    if [ -n "$COMMAND_PID" ]; then
        kill -TERM -- "-$COMMAND_PID" 2>/dev/null
        for _ in 1 2 3 4 5 6 7 8 9 10; do
            kill -0 -- "-$COMMAND_PID" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL -- "-$COMMAND_PID" 2>/dev/null
    fi
    close_remaining_windows
    hypr_eval 'if JADESLIP_ONLY_TEST_RULE then JADESLIP_ONLY_TEST_RULE:set_enabled(false) end' || true
    has_output && remove_output
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        has_output || break
        sleep 0.1
    done
    [ -n "$FOCUS_GUARD_PID" ] && kill -KILL "$FOCUS_GUARD_PID" 2>/dev/null
    local status=0
    if [ "$(remaining_windows)" != "0" ]; then
        echo "headless: a test window is still open" >&2
        status=1
    fi
    if has_output; then
        echo "headless: the output $OUTPUT is still present" >&2
        status=1
    fi
    if [ -n "$COMMAND_PID" ] && pgrep -s "$COMMAND_PID" >/dev/null; then
        echo "headless: a test process is still running" >&2
        status=1
    fi
    return $status
}

for tool in hyprctl jq setsid flock pgrep; do
    if ! command -v "$tool" >/dev/null; then
        echo "headless: $tool is required" >&2
        exit 2
    fi
done

exec 9>"$LOCK"
if ! flock 9; then
    echo "headless: could not take $LOCK" >&2
    exit 2
fi

on_signal() {
    cleanup
    exit 130
}
trap 'cleanup' EXIT
trap 'on_signal' INT TERM HUP

has_output && remove_output
hypr_eval "SS_TEST_WORKSPACE = SS_TEST_WORKSPACE or hl.workspace_rule({ workspace = '99', monitor = '$OUTPUT', default = true })" || exit 1
keep_owner_focus "$$" 9>&- >/dev/null 2>&1 &
FOCUS_GUARD_PID=$!
if [ "$(hyprctl output create headless "$OUTPUT")" != "ok" ]; then
    echo "headless: the output could not be created" >&2
    exit 1
fi
for _ in $(seq 1 50); do
    has_output && break
    sleep 0.1
done
has_output || { echo "headless: the output did not appear" >&2; exit 1; }

hypr_eval "hl.monitor({ output = '$OUTPUT', mode = '1280x800@60', position = '20000x20000', scale = 1 })" || exit 1
SHOWN=""
SEEN=""
for _ in $(seq 1 30); do
    NOW=$(output_workspace)
    if [ -n "$NOW" ] && [ "$NOW" = "$SEEN" ]; then
        SHOWN=$NOW
        break
    fi
    SEEN=$NOW
    sleep 0.1
done
[ -n "$SHOWN" ] || { echo "headless: the output shows no workspace" >&2; exit 1; }
hypr_eval "if SS_TEST_RULE then SS_TEST_RULE:set_enabled(false) end if SS_TEST_SILENT_RULE then SS_TEST_SILENT_RULE:set_enabled(false) end if JADESLIP_TEST_RULE then JADESLIP_TEST_RULE:set_enabled(false) end if JADESLIP_ONLY_TEST_RULE then JADESLIP_ONLY_TEST_RULE:set_enabled(false) end JADESLIP_TEST_RULES = JADESLIP_TEST_RULES or {} JADESLIP_TEST_RULES['$SHOWN'] = JADESLIP_TEST_RULES['$SHOWN'] or hl.window_rule({ name = 'jadeslip-only-test-silent-$SHOWN', match = { class = '^jadeslip-test\$' }, workspace = '$SHOWN silent', no_initial_focus = true, suppress_event = 'fullscreen maximize activate activatefocus', no_anim = true }) JADESLIP_ONLY_TEST_RULE = JADESLIP_TEST_RULES['$SHOWN'] JADESLIP_ONLY_TEST_RULE:set_enabled(true)" || exit 1

SS_HEADLESS_OUTPUT="$OUTPUT" WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}" setsid "$@" 9>&- &
COMMAND_PID=$!
SCRIPT_PID=$$
SESSION=$COMMAND_PID
setsid bash -c '
    while kill -0 "$1" 2>/dev/null; do sleep 1; done
    pkill -KILL -s "$2"
    hyprctl clients -j | jq -r ".[] | select(.class | test(\"^jadeslip-test\$\")) | .pid" | xargs -r kill -KILL
    sleep 0.5
    hyprctl eval "if JADESLIP_ONLY_TEST_RULE then JADESLIP_ONLY_TEST_RULE:set_enabled(false) end" >/dev/null
    hyprctl monitors all -j | jq -e --arg name "$3" "any(.[]; .name == \$name)" >/dev/null && hyprctl --batch "eval SS_TEST_CURSOR = hl.get_cursor_pos() ; output remove $3 ; eval hl.dispatch(hl.dsp.cursor.move({ x = SS_TEST_CURSOR.x, y = SS_TEST_CURSOR.y }))" >/dev/null
' jadeslip-headless-watchdog "$SCRIPT_PID" "$SESSION" "$OUTPUT" 9>&- >/dev/null 2>&1 &
WATCHDOG_PID=$!
disown "$WATCHDOG_PID"
wait "$COMMAND_PID"
STATUS=$?
cleanup
CLEAN_STATUS=$?
if [ "$STATUS" -eq 0 ]; then
    STATUS=$CLEAN_STATUS
fi
exit "$STATUS"
