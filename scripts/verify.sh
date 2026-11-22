#!/usr/bin/env bash
set -u

usage() {
    cat <<'EOF'
usage: scripts/verify.sh [--base <ref>] <mode> [args...]

modes:
  quick                      fixtures, cargo test, clippy (plain and e2e builds),
                             comment check, attribution check
  full                       quick, then every GUI e2e suite one by one, then leftovers
  e2e <suite...> [-- <filter...>]
                             fixtures, the named e2e suites through tests/e2e/run.sh,
                             then leftovers; filters are test name filters
  mutate <name> [<filter>] [--changed] [--list]
                             tests/mutate_<name>.py on the entries whose name holds
                             <filter>, then a check that every source file is
                             restored, then leftovers; --changed keeps only entries
                             whose source or test file differs from the merge base
                             with main, uncommitted and untracked files included;
                             --list prints the chosen entries and runs none
  leftovers                  only the leftover checks

options:
  --base <ref>   base for the attribution check (default: git merge-base HEAD main)
  -h, --help     this text

environment:
  SS_FIXTURES=<abs path>   a fixtures folder holding real/*.epub to copy into a
                           worktree that has none (default: the main checkout's)

full runs every tests/e2e/tests/*.rs suite that has ignored tests, timing included.
Logs go to target/verify/<time>-<mode>/. The summary table prints at the end.
The exit status is 1 when any row is FAIL, 3 when a row is BUSY and none
failed, 2 on a usage error.
EOF
}

BASE=""
MODE=""
ARGS=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h | --help)
            usage
            exit 0
            ;;
        --base)
            [ "$#" -ge 2 ] || { echo "verify: --base needs a ref" >&2; exit 2; }
            BASE=$2
            shift 2
            ;;
        *)
            if [ -z "$MODE" ]; then
                MODE=$1
            else
                ARGS+=("$1")
            fi
            shift
            ;;
    esac
done

case "$MODE" in
    quick | full | leftovers) ;;
    e2e)
        [ "${#ARGS[@]}" -gt 0 ] || { echo "verify: e2e needs at least one suite" >&2; exit 2; }
        ;;
    mutate)
        [ "${#ARGS[@]}" -ge 1 ] && [ "${#ARGS[@]}" -le 4 ] || { echo "verify: mutate takes <name> [<filter>] [--changed] [--list]" >&2; exit 2; }
        ;;
    "")
        usage >&2
        exit 2
        ;;
    *)
        echo "verify: unknown mode $MODE" >&2
        usage >&2
        exit 2
        ;;
esac

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && git rev-parse --show-toplevel) || exit 2
cd "$ROOT" || exit 2
LOGS="$ROOT/target/verify/$(date +%Y%m%d-%H%M%S)-$MODE"
mkdir -p "$LOGS" || exit 2
LOCK="${XDG_RUNTIME_DIR:-/tmp}/jadeslip-headless.lock"
TEST_CLASS='^jadeslip-test$'
STARTED=$SECONDS
STEP_STATUS=""

ROW_NAME=()
ROW_STATUS=()
ROW_SECS=()
ROW_DETAIL=()

record() {
    ROW_NAME+=("$1")
    ROW_STATUS+=("$2")
    ROW_SECS+=("$3")
    ROW_DETAIL+=("$4")
    printf '    %s %s (%ss) %s\n' "$1" "$2" "$3" "$4"
}

logged() {
    local log=$1
    shift
    "$@" >>"$log" 2>&1
}

test_counts() {
    awk '/^test result:/ {
            for (i = 1; i <= NF; i++) {
                if ($(i + 1) ~ /^passed/) p += $i
                if ($(i + 1) ~ /^failed/) f += $i
                if ($(i + 1) ~ /^ignored/) g += $i
            }
            n++
        }
        END { printf "%d %d %d %d", p, f, g, n }' "$1"
}

step() {
    local name=$1
    shift
    local begun=$SECONDS
    STEP_LOG="$LOGS/$name.log"
    STEP_DETAIL=""
    STEP_STATUS=""
    echo "==> $name    log: $STEP_LOG"
    local status=PASS
    "$@" || status=FAIL
    [ -n "$STEP_STATUS" ] && status=$STEP_STATUS
    record "$name" "$status" "$((SECONDS - begun))" "$STEP_DETAIL"
}

main_checkout() {
    local common
    common=$(git rev-parse --path-format=absolute --git-common-dir) || return 1
    dirname "$common"
}

check_fixtures() {
    local real="$ROOT/fixtures/real" source
    if ! compgen -G "$real/*.epub" >/dev/null; then
        source=${SS_FIXTURES:-$(main_checkout)/fixtures}
        if [ "$source" -ef "$ROOT/fixtures" ] || ! compgen -G "$source/real/*.epub" >/dev/null; then
            STEP_DETAIL="no fixtures/real/*.epub here or in $source; the tests find fixtures only at <checkout>/fixtures through CARGO_MANIFEST_DIR, so set SS_FIXTURES to a folder holding real/"
            return 1
        fi
        if ! mkdir -p "$real" || ! logged "$STEP_LOG" cp -a --reflink=auto "$source/real/." "$real"; then
            STEP_DETAIL="copy of $source/real failed"
            return 1
        fi
        STEP_DETAIL="copied real books from $source/real; "
    fi
    if [ ! -f "$ROOT/fixtures/generated/attack.epub" ]; then
        logged "$STEP_LOG" cargo run --release -p jadeslip-fixtures --bin gen-fixtures || {
            STEP_DETAIL+="gen-fixtures failed"
            return 1
        }
        STEP_DETAIL+="generated fixtures/generated; "
    fi
    STEP_DETAIL+="$(find "$real" -maxdepth 1 -name '*.epub' | wc -l) real books"
}

check_cargo_test() {
    logged "$STEP_LOG" cargo test --workspace
    local code=$? passed failed ignored binaries
    read -r passed failed ignored binaries < <(test_counts "$STEP_LOG")
    STEP_DETAIL="$passed passed, $failed failed, $ignored ignored over $binaries binaries"
    [ "$code" -eq 0 ] && [ "$failed" -eq 0 ]
}

check_clippy() {
    logged "$STEP_LOG" cargo clippy --workspace --all-targets -- -D warnings
}

check_clippy_e2e() {
    CARGO_TARGET_DIR=target/e2e logged "$STEP_LOG" cargo clippy --workspace --all-targets --features jadeslip/e2e -- -D warnings
}

check_comments() {
    logged "$STEP_LOG" python3 tests/check_no_comments.py
    local code=$?
    STEP_DETAIL=$(grep -m1 '^comments found:' "$STEP_LOG")
    return "$code"
}

check_attribution() {
    local base=$BASE commit hits=0 count lines
    if [ -z "$base" ]; then
        base=$(git merge-base HEAD main) || {
            STEP_DETAIL="no merge base with main; pass --base"
            return 1
        }
    fi
    git rev-parse --verify --quiet "$base^{commit}" >/dev/null || {
        STEP_DETAIL="unknown base $base"
        return 1
    }
    count=$(git rev-list --count "$base..HEAD")
    : >"$STEP_LOG"
    for commit in $(git rev-list "$base..HEAD"); do
        lines=$(git log -1 --format=%B "$commit" | grep -i -E 'co-authored-by|claude|generated with')
        if [ -n "$lines" ]; then
            hits=$((hits + 1))
            echo "${commit:0:10}: ${lines//$'\n'/ | }" >>"$STEP_LOG"
        fi
    done
    STEP_DETAIL="$count commits since ${base:0:10}, $hits with attribution"
    [ "$hits" -eq 0 ]
}

check_e2e_suite() {
    local suite=$1
    shift
    if [ ! -f "$ROOT/tests/e2e/tests/$suite.rs" ]; then
        STEP_DETAIL="no suite tests/e2e/tests/$suite.rs"
        return 1
    fi
    SS_E2E_TEST=$suite logged "$STEP_LOG" tests/e2e/run.sh "$@"
    local code=$? passed failed ignored binaries
    read -r passed failed ignored binaries < <(test_counts "$STEP_LOG")
    STEP_DETAIL="$passed passed, $failed failed, exit $code"
    if [ "$code" -eq 0 ] && [ "$passed" -eq 0 ]; then
        STEP_DETAIL+=", nothing ran"
        return 1
    fi
    [ "$code" -eq 0 ] && [ "$failed" -eq 0 ]
}

check_mutate() {
    local name=$1 before after
    if [ ! -f "$ROOT/tests/mutate_$name.py" ]; then
        STEP_DETAIL="no runner tests/mutate_$name.py"
        return 1
    fi
    before=$(git status --porcelain=v1 -uno && git diff)
    shift
    logged "$STEP_LOG" python3 "tests/mutate_$name.py" "$@"
    local code=$?
    [[ " $* " == *" --list "* ]] && sed 's/^/    /' "$STEP_LOG"
    after=$(git status --porcelain=v1 -uno && git diff)
    STEP_DETAIL="exit $code; $(grep -v '^\s*$' "$STEP_LOG" | tail -n 1)"
    if [ "$before" != "$after" ]; then
        STEP_DETAIL="source left mutated, restore with git before anything else; $STEP_DETAIL"
        return 1
    fi
    return "$code"
}

lock_is_free() {
    flock -n "$LOCK" true 2>/dev/null
}

check_leftover_folders() {
    local dirs=() dead=() live=() dir pid
    for dir in "${TMPDIR:-/tmp}" /tmp; do
        compgen -G "$dir/jadeslip-test-*" >/dev/null && dirs+=("$dir"/jadeslip-test-*)
    done
    for dir in $(printf '%s\n' "${dirs[@]}" | sort -u); do
        pid=${dir##*jadeslip-test-}
        if [ -d "/proc/$pid" ]; then live+=("$pid"); else dead+=("$dir"); fi
    done
    printf 'dead owner: %s\n' "${dead[@]}" >"$STEP_LOG"
    printf 'live owner pid: %s\n' "${live[@]}" >>"$STEP_LOG"
    STEP_DETAIL="${#dead[@]} left by dead processes, ${#live[@]} held by live processes"
    [ "${#dead[@]}" -eq 0 ]
}

check_leftover_processes() {
    local proc exe ours=() others
    for proc in /proc/[0-9]*; do
        exe=$(readlink "$proc/exe" 2>/dev/null) || continue
        case "$exe" in
            "$ROOT"/target/* | */jadeslip-test-*/*) ours+=("${proc#/proc/} $exe") ;;
        esac
    done
    others=$(pgrep -x 'jadeslip|jadeslip-worker' | wc -l)
    printf 'running: %s\n' "${ours[@]}" >"$STEP_LOG"
    STEP_DETAIL="${#ours[@]} test-built processes still running, $others jadeslip processes in all"
    [ "${#ours[@]}" -eq 0 ]
}

check_leftover_output() {
    if ! command -v hyprctl >/dev/null || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
        STEP_STATUS=SKIP
        STEP_DETAIL="no Hyprland session"
        return 0
    fi
    local monitors clients output windows
    if ! monitors=$(hyprctl monitors all -j 2>&1) || ! clients=$(hyprctl clients -j 2>&1); then
        STEP_DETAIL="hyprctl failed, inconclusive"
        return 1
    fi
    output=$(jq '[.[] | select(.name == "SS-TEST")] | length' <<<"$monitors")
    windows=$(jq --arg class "$TEST_CLASS" '[.[] | select((.class | test($class)) or (.initialClass | test($class)))] | length' <<<"$clients")
    jq -r '.[].name' <<<"$monitors" >"$STEP_LOG"
    STEP_DETAIL="SS-TEST outputs $output, test windows $windows"
    [ "$output" = 0 ] && [ "$windows" = 0 ] && return 0
    if ! lock_is_free; then
        STEP_STATUS=BUSY
        STEP_DETAIL+=", another headless run holds $LOCK; rerun: scripts/verify.sh leftovers"
        return 0
    fi
    return 1
}

default_suites() {
    local file suite
    for file in "$ROOT"/tests/e2e/tests/*.rs; do
        suite=$(basename "$file" .rs)
        grep -q '#\[ignore' "$file" && echo "$suite"
    done
}

run_quick() {
    step fixtures check_fixtures
    step cargo-test check_cargo_test
    step clippy check_clippy
    step clippy-e2e check_clippy_e2e
    step comments check_comments
    step attribution check_attribution
}

run_leftovers() {
    step leftover-folders check_leftover_folders
    step leftover-processes check_leftover_processes
    step leftover-output check_leftover_output
}

run_e2e() {
    local suites=() filters=() seen_dash="" arg suite
    for arg in "$@"; do
        if [ -n "$seen_dash" ]; then
            filters+=("$arg")
        elif [ "$arg" = "--" ]; then
            seen_dash=1
        else
            suites+=("$arg")
        fi
    done
    for suite in "${suites[@]}"; do
        step "e2e-$suite" check_e2e_suite "$suite" "${filters[@]}"
    done
}

case "$MODE" in
    quick)
        run_quick
        ;;
    full)
        run_quick
        mapfile -t SUITES < <(default_suites)
        run_e2e "${SUITES[@]}"
        run_leftovers
        ;;
    e2e)
        step fixtures check_fixtures
        run_e2e "${ARGS[@]}"
        run_leftovers
        ;;
    mutate)
        step fixtures check_fixtures
        step "mutate-${ARGS[0]}" check_mutate "${ARGS[@]}"
        run_leftovers
        ;;
    leftovers)
        run_leftovers
        ;;
esac

FAILED=0
BUSY=0
echo
echo "verify $MODE at $(git rev-parse --short HEAD) in $ROOT, $((SECONDS - STARTED))s"
printf '%-22s %-6s %7s  %s\n' STEP RESULT SECONDS DETAIL
for i in "${!ROW_NAME[@]}"; do
    printf '%-22s %-6s %7s  %s\n' "${ROW_NAME[$i]}" "${ROW_STATUS[$i]}" "${ROW_SECS[$i]}" "${ROW_DETAIL[$i]}"
    [ "${ROW_STATUS[$i]}" = FAIL ] && FAILED=1
    [ "${ROW_STATUS[$i]}" = BUSY ] && BUSY=1
done
echo "logs: $LOGS"
if [ "$FAILED" -eq 1 ]; then
    echo "verify: FAILED"
    exit 1
fi
if [ "$BUSY" -eq 1 ]; then
    echo "verify: INCONCLUSIVE"
    exit 3
fi
echo "verify: PASSED"
