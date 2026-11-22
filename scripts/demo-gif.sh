#!/usr/bin/env bash
set -euo pipefail
export TZ=UTC
cd "$(dirname "$0")/.."
ROOT="$PWD"
S="$ROOT/.claude/skills/verify-jadeslip/scripts"
OUT="$ROOT/docs/screenshots/demo.gif"
FONT="$ROOT/app/ui/fonts/ibm-plex-sans-latin-600-normal.woff2"

cargo run --release -q -p jadeslip-fixtures --bin gen-samples >/dev/null
"$S/launch.sh" >/dev/null
trap '"$S/stop.sh" >/dev/null' EXIT
RUN="$("$S/ctl.sh" run)"
FRAMES="$RUN/evidence/demo"
mkdir -p "$FRAMES"

ctl() { "$S/ctl.sh" "$@" >/dev/null; }
until_true() { ctl wait "$1" "${2:-15}"; }
at() { until_true "document.getElementById('library').hidden && document.getElementById('where-title').textContent === '$1'" 20; }
editing() { until_true "!!document.getElementById('book-host').shadowRoot.querySelector('[contenteditable=\"true\"]') === $1" 10; }

n=0
DELAYS=()
frame() {
    n=$((n + 1))
    local name
    name="$(printf 'demo/%02d' "$n")"
    ctl shot "$name"
    if [[ -n "${3:-}" ]]; then
        magick "$RUN/evidence/$name.png" \( -size 84x58 xc:none -fill '#f4f1ea' -stroke '#59B98A' -strokewidth 3 \
            -draw "roundrectangle 2,2 81,55 12,12" -stroke none -fill '#1b1d1f' -font "$FONT" -pointsize "$2" \
            -gravity center -annotate +0+0 "$3" \) -gravity northwest -geometry +1156+600 -composite "$RUN/evidence/$name.png"
    fi
    DELAYS+=("$1")
}

ctl drop "$ROOT"/fixtures/generated/samples/*.epub
until_true "document.querySelectorAll('#shelf-grid .card').length === 5" 30
key="$("$S/ctl.sh" wait "[...document.querySelectorAll('#shelf-grid .cardgo')].find((b) => b.querySelector('.cardtitle').textContent === 'The Lantern Keeper')?.dataset.key" 10)"
ctl click "#shelf-grid .cardgo[data-key=${key}]"
at 'The Lamp on Harrow Point'
ctl click '#next'
at 'The Unsigned Letter'
until_true "document.getElementById('toast').hidden && document.getElementById('back').hidden" 15
frame 180

ctl key e
editing true
frame 150 30 E

read -r x y < <("$S/ctl.sh" eval "(() => { const w = document.createTreeWalker(document.getElementById('book-host').shadowRoot, NodeFilter.SHOW_TEXT); for (let n; (n = w.nextNode());) { const i = n.data.indexOf('recieved'); if (i >= 0) { const r = document.createRange(); r.setStart(n, i); r.setEnd(n, i + 1); const b = r.getBoundingClientRect(); return Math.round(b.left + 1) + ' ' + Math.round(b.top + b.height / 2); } } return null; })()" | tr -d '"')
ctl click-at "$x" "$y"
ctl key Right ctrl shift
until_true "getSelection().toString() === 'recieved'" 5

word=received
for ((i = 1; i <= ${#word}; i++)); do
    ctl type "${word:i-1:1}"
    frame 9
done
DELAYS[-1]=70

ctl key Escape
editing false
until_true "document.getElementById('book-host').shadowRoot.textContent.includes('had not received a letter') && document.getElementById('toast').textContent.startsWith('Saved.')" 10
frame 280 22 Esc

args=()
for ((i = 1; i <= n; i++)); do
    args+=(-delay "${DELAYS[i-1]}" "$(printf '%s/%02d.png' "$FRAMES" "$i")")
done
magick "${args[@]}" -resize 960x -layers OptimizePlus -loop 0 "$OUT"
echo "wrote $OUT ($(du -h "$OUT" | cut -f1), $n frames; journal: $RUN/evidence/journal.log)"
