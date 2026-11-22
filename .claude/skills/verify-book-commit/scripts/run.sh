#!/usr/bin/env bash
set -u
SKILL="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$SKILL/../../.." && pwd)"
MODEL="$SKILL/model"
JAR_SHA=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
GROUP="${1:-all}"
case "$GROUP" in
    all|checks|findings|mutants|drift) ;;
    *) echo "usage: run.sh [all|checks|findings|mutants|drift]" >&2; exit 2 ;;
esac

find_jar() {
    local candidate
    for candidate in "${TLA2TOOLS:-}" \
        "$(mise where http:tla2tools@1.7.4 2>/dev/null)/tla2tools.jar" \
        "$HOME/.local/share/mise/installs/http-tla2tools/1.7.4/tla2tools.jar" \
        "$HOME/.cache/rcc-tla/1.7.4/tla2tools.jar"; do
        [ -n "$candidate" ] && [ -f "$candidate" ] || continue
        [ "$(sha256sum "$candidate" | cut -d' ' -f1)" = "$JAR_SHA" ] && { echo "$candidate"; return 0; }
    done
    return 1
}

status=0
if [ "$GROUP" = all ] || [ "$GROUP" = drift ]; then
    python3 "$SKILL/scripts/coverage.py" check || status=1
    echo
fi
[ "$GROUP" = drift ] && exit "$status"

command -v java >/dev/null || { echo "run: java 11+ is required" >&2; exit 2; }
JAR="$(find_jar)" || {
    echo "run: TLC 1.7.4 not found. Download https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar and set TLA2TOOLS=<path> (sha256 $JAR_SHA)" >&2
    exit 2
}
OUT="$ROOT/target/verify-book-commit/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

run_one() {
    local cfg="$1" kind="$2" name expected log result verdict started seconds states
    name="$(basename "$(dirname "$cfg")")/$(basename "$cfg" .cfg)"
    log="$OUT/${name//\//-}.log"
    expected="$(awk '/^INVARIANTS/{f=1;next} f&&NF{print $1}' "$cfg" | paste -sd, -)"
    started=$(date +%s)
    (cd "$MODEL" && java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers auto -deadlock \
        -metadir "$OUT/states-${name//\//-}" -config "$cfg" BookCommit.tla) >"$log" 2>&1
    seconds=$(( $(date +%s) - started ))
    rm -rf "$OUT/states-${name//\//-}"
    states="$(grep -oE '[0-9]+ distinct states found' "$log" | tail -1 | cut -d' ' -f1)"
    if grep -q "No error has been found" "$log"; then
        result="no violation"
    elif result="$(grep -oE 'Invariant [A-Za-z]+ is violated' "$log" | head -1 | cut -d' ' -f2)" && [ -n "$result" ]; then
        result="violates $result"
    else
        result="TLC error, see log"
    fi
    case "$kind" in
        checks) [ "$result" = "no violation" ] && verdict=PASS || { verdict=FAIL; status=1; } ;;
        findings) [ "$result" = "violates $expected" ] && verdict=REPRODUCES || { verdict=CHANGED; status=1; } ;;
        mutants) [ "$result" = "violates $expected" ] && verdict=CAUGHT || { verdict=MISSED; status=1; } ;;
    esac
    printf '%-11s %-52s %-36s %9s states %4ss  %s\n' "$verdict" "$name" "$result" "${states:-?}" "$seconds" "$log"
}

for kind in checks findings mutants; do
    [ "$GROUP" = all ] || [ "$GROUP" = "$kind" ] || continue
    for cfg in "$MODEL/$kind"/*.cfg; do
        run_one "$cfg" "$kind"
    done
done
echo
echo "logs: $OUT"
[ "$status" = 0 ] && echo "run: as expected" || echo "run: SURPRISE, read SKILL.md 'Reading a result'"
exit "$status"
