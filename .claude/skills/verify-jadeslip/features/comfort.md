# Text size, font and spacing

The `Aa` panel lets a reader tune the reading view: text size, spacing presets, font, margins, full screen, and the book's spelling language. Theme, page view and spelling marks on or off live in Preferences (`#open-prefs`, Ctrl+,). Choices persist across launches in `prefs.json`.

## Sub-features

- `comfort-open` opens the panel with `#toggle-comfort` or Ctrl+Shift+A and closes it with Escape.
- `comfort-theme` switches between dark and light in Preferences > General (`#prefs-theme-dark`, `#prefs-theme-light`), and with `#shelf-theme` on the shelf.
- `comfort-size` changes text size (`#font-smaller`, `#font-larger`, readout `#font-now`).
- `comfort-layout` switches between scroll and pages in Preferences > General (`#prefs-pages-scroll`, `#prefs-pages-auto`). The column and margins controls stay in the panel.
- `comfort-spelling` picks the book's language in the panel (`#comfort-tab-spelling`, `#spelling-language`). Marks on or off are in Preferences > Spelling (`#prefs-spelling-on`, `#prefs-spelling-off`).

## How to get to it (user POV)

- Click the `Aa` button in the top bar (aria-label `Text size, font and spacing`), or press Ctrl+Shift+A while reading.
- Click the moon or sun on the library shelf for the theme.

## Driving it with ctl.sh

Preconditions:

- Launched with `--book` on a real book, empty data folder (dark theme by default), doctor clean. Not yet proven end to end by this skill. The handles come from `tests/e2e/tests/reader.rs` and `spelling.rs`.

- **Open.** Run `ctl.sh click '#toggle-comfort'`, then wait for `!document.getElementById('comfort').hidden`. Shoot `01-panel`.
- **Light theme.** Run `ctl.sh click '#open-prefs'`, then `ctl.sh click '#prefs-theme-light'`. Read back `getComputedStyle(document.body).backgroundColor` before and after. Shoot `02-light`.
- **Larger text.** Run `ctl.sh click '#font-larger'` twice. `#font-now` goes up by two steps.
- **Persisted.** Run `ctl.sh keep data/prefs.json`. It records the light theme and the new size.

## Gotchas

- The window background colour is chosen at startup from `prefs.json`. Theme changes repaint the page, but only a relaunch on the same data shows the startup colour, and `launch.sh` always starts with empty data.
- The panel has tabs (`#comfort-tab-text`, `-layout`, `-spelling`). Controls on a hidden tab fail `click` with "no visible element", so click the tab first.
- Full screen (`#toggle-fullscreen`, F11) changes the window on the headless output only. Check it with `ctl.sh state`, which reports `fullscreen`.
