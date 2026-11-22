#!/usr/bin/env bash
set -euo pipefail
export TZ=UTC
cd "$(dirname "$0")/.."
ROOT="$PWD"
S="$ROOT/.claude/skills/verify-jadeslip/scripts"
SHOTS=(library reader reader-light search edit)

cargo run --release -q -p jadeslip-fixtures --bin gen-samples >/dev/null
"$S/launch.sh" >/dev/null
trap '"$S/stop.sh" >/dev/null' EXIT
RUN="$("$S/ctl.sh" run)"

ctl() { "$S/ctl.sh" "$@" >/dev/null; }
until_true() { ctl wait "$1" "${2:-15}"; }
at() { until_true "document.getElementById('library').hidden && document.getElementById('where-title').textContent === '$1'" 20; }
on_shelf() { until_true "!document.getElementById('library').hidden && document.getElementById('toast').hidden" 15; }
dark() { until_true "document.getElementById('book-host').classList.contains('theme-ink') === $1" 5; }
park_on_shelf() { ctl click-at 1100 600; }
park_in_reader() { ctl click-at 1180 400; }

open_card() {
    local key
    key="$("$S/ctl.sh" wait "[...document.querySelectorAll('#shelf-grid .cardgo')].find((b) => b.querySelector('.cardtitle').textContent === '$1')?.dataset.key" 10)"
    ctl click "#shelf-grid .cardgo[data-key=${key}]"
}

switch_theme() {
    ctl click '#open-palette'
    ctl type 'switch theme'
    until_true "document.querySelector('#palette-rows .palrow.on')?.textContent.includes('Switch theme')" 5
    ctl key Enter
}

ctl drop "$ROOT"/fixtures/generated/samples/*.epub
until_true "document.querySelectorAll('#shelf-grid .card').length === 5" 30

open_card 'The Lantern Keeper'
at 'The Lamp on Harrow Point'
ctl click '#next'
at 'The Unsigned Letter'
ctl click '#go-library'
on_shelf
until_true "document.getElementById('hero-title').textContent === 'The Lantern Keeper' && document.getElementById('hero-go').textContent === 'Resume'"
park_on_shelf
ctl shot library

ctl click '#hero-go'
at 'The Unsigned Letter'
ctl click '#prev'
at 'The Lamp on Harrow Point'
until_true "document.getElementById('back').hidden"
park_in_reader
ctl shot reader

switch_theme
dark false
park_in_reader
ctl shot reader-light
switch_theme
dark true

ctl click '#toggle-search'
ctl click '#search-regex'
until_true "document.getElementById('search-regex').classList.contains('on')" 5
ctl click '#search-input'
ctl type 'lighthouse|logbook'
ctl key Enter
until_true "document.getElementById('search-action').textContent === 'Rerun' && document.querySelectorAll('#search-pane .result').length >= 5" 30
park_in_reader
ctl shot search

ctl click '#tab-contents'
ctl click '#next'
at 'The Unsigned Letter'
until_true "document.getElementById('back').hidden"
ctl click '#mode-edit'
until_true "!!document.getElementById('book-host').shadowRoot.querySelector('[contenteditable=\"true\"]')" 10
words="$("$S/ctl.sh" wait "document.getElementById('word-count').textContent" 10)"
ctl click 'book:p:last-of-type'
ctl key End ctrl
ctl type ' Nell put the key in her pocket and went to find a knife.'
until_true "CSS.highlights.get('ss-spell')?.size > 0 && document.body.innerText.includes('Unsaved changes') && document.getElementById('word-count').textContent !== ${words}" 15
ctl shot edit

for shot in "${SHOTS[@]}"; do
    cp "$RUN/evidence/$shot.png" "docs/screenshots/$shot.png"
done
echo "retook ${SHOTS[*]} into docs/screenshots (journal: $RUN/evidence/journal.log)"
