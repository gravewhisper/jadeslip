# Search

A reader searches the open book for text or a regex. Results stream into the right-hand pane with a running count, choosing a result scrolls the book to that hit and highlights it, and a search the user actually used is remembered under the search field.

## Sub-features

- `search-open` opens the pane from the toolbar button and from Ctrl+F.
- `search-plain` finds plain text case-insensitively and reports `N matches.` once every chapter is scanned.
- `search-open-result` moves the reading place to the hit and highlights it as the current match.
- `search-recent` records a search that was used in `data/searches.json` under `recent`.
- `search-regex` runs a regex search in the worker process (`#search-regex`).
- `search-saved` saves a search by name (`#search-save`) and lists it under `#search-saved`.

## How to get to it (user POV)

- Click the magnifier button in the top bar (aria-label `Search`, `#toggle-search`).
- Press Ctrl+F while reading.
- Open the `Saved and recent searches` list (`#search-saved`) to rerun an earlier search.

## Driving it with ctl.sh

Preconditions:

- Launched with `--book fixtures/real/tech_a.epub`, empty data folder, doctor clean.

- **Toolbar entry.** Run `ctl.sh click '#toggle-search'`, then `ctl.sh wait "document.activeElement.id === 'search-input'"`. The pane opens with focus in the search field.
- **Keyboard entry.** With the pane closed (`ctl.sh key Escape`), run `ctl.sh key f ctrl`. The same wait passes.
- **Plain query.** Run `ctl.sh type 'duplication'`, then `ctl.sh wait "document.getElementById('search-action').textContent === 'Rerun'" 60`. `#search-count` reads `57 matches.` and `#search-scanned` reads `21 of 21 chapters searched`. Shoot `02-results`.
- **Open a result.** Run `ctl.sh click '#search-results .result:not([hidden])'`, then `ctl.sh wait "CSS.highlights.get('ss-search-current')?.size > 0"`. `[...CSS.highlights.get('ss-search-current').values()].map(r => r.toString())` is `["Duplication"]`, `#where-title` changes from `Colophon` to the chapter holding the first match, and the footer reads `Match 1 of 57.` Shoot `03-result-opened`.
- **Recent search stored.** Run `ctl.sh keep data/searches.json`. It holds `{"recent":[{"matchCase":false,"regex":false,"text":"duplication"}],"saved":[],"version":1}`.
- **Proof.** `evidence/02-results.png`, `evidence/03-result-opened.png`, `evidence/searches.json`, and the journal lines for click, type, wait and eval.

## Gotchas

- `#search-status` stays visible after the scan ends, showing `N of N chapters searched`. The done signal is `#search-action` reading `Rerun`, not the status line being hidden.
- Typing alone does not write `searches.json`. A search is recorded only when it is used: a result is clicked or Enter steps to a hit. Check the file after opening a result.
- Results are virtualised rows. Use `.result:not([hidden])` and only click rows inside the list's visible box. `click` refuses rows that are covered or outside the window.
- Regex search runs in `target/e2e/release/jadeslip-worker`. If it reports that it could not start, the worker was not built (launch without `--no-build`).
