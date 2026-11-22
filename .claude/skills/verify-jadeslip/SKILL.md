---
name: verify-jadeslip
description: Launch the real Jadeslip desktop app (Tauri EPUB reader and editor) on the headless Hyprland output, drive it through real GDK clicks and key events over its e2e control socket, and keep screenshots, a journal and data-folder files as proof. Use to prove a user-facing change works in the running app, to reproduce a UI bug, or to explore a feature by hand, beyond what scripts/verify.sh runs.
---

# Verify Jadeslip in the running app

Jadeslip is a Tauri 2 desktop app: a WebKitGTK window over Rust commands. The user touches the window (library shelf, reader, search pane, editor). There is also a small CLI (`jadeslip info|extract <book>`), which is good for checking what a save wrote, but it is not the primary surface.

Drive the app only through the scripts below. They start it through `tests/e2e/headless.sh`, which is the only allowed way to start it (see the `verify` skill's hard rules). The window opens on a headless output called `SS-TEST`, far off the user's screens. Every op goes through the e2e build's Unix control socket (`app/src/e2e.rs`), so clicks and keys are real GDK events delivered to the webview, not DOM `.click()` calls.

All commands run from the repo root. `S=.claude/skills/verify-jadeslip/scripts`.

## Launch

```
$S/launch.sh --book fixtures/real/tech_a.epub
$S/launch.sh                       opens on the library shelf, with no book
$S/launch.sh --no-build --book X   skip the cargo build when the binary is fresh
```

What it does:
1. It refuses to start if a run of this skill is still up, or if another headless run holds `$XDG_RUNTIME_DIR/jadeslip-headless.lock` (an e2e suite or `scripts/verify.sh`). There is one `SS-TEST` output per machine, so two runs never share it. If the lock is held, wait. Do not start beside it.
2. It builds `target/e2e/release/jadeslip --features e2e` and `jadeslip-worker` the same way `tests/e2e/run.sh` does. That takes about 12 s when the binary is fresh and several minutes after core changes. Fixtures are generated only if they are missing.
3. It makes a run folder, `target/verify-app/<YYYYmmdd-HHMMSS>/`, and points `target/verify-app/current` at it. The folder holds `data/` (`JADESLIP_DATA_DIR`, an empty data folder), `xdg/` (`XDG_DATA_HOME`, so trash moves stay inside the run), `tmp/`, `books/`, `evidence/`, `ctl.sock`, `app.log`, `headless.log`, `app.pid`, `headless.pid`, `commit` and `dirty`.
4. It copies `--book` into `books/` and opens the copy. The original is never written. `fixtures/real` must never be edited.
5. It waits until the page is usable. With a book, that is the page's own `ss-first-text` performance mark, set after two painted frames. With no book, it is `#library` shown. Expect under 1 s. A book that fails to open makes launch fail with the app's own message from `#open-failure-text`.

It prints `run:`, `book:` and `evidence:` when it is ready. A non-zero exit means it is not ready: read `app.log` and `headless.log` in the run folder, then run `stop.sh`.

## Doctor

```
$S/doctor.sh
```

Read-only. Run it first whenever anything looks off. Each line is OK, WARN or FAIL, and the exit is 1 on any FAIL. It checks that `headless.sh` and the app pid are alive, that the app pid's exe is `target/e2e/release/jadeslip`, that the window sits on the `SS-TEST` output, that the control socket answers an eval, whether the run was launched at the current HEAD, whether any `app/`, `crates/core` or `crates/worker` file is newer than the binary (if so, relaunch without `--no-build`), and whether the headless lock is held.

If the window is on `SS-TEST` but the page never paints (`ready` times out, or `requestAnimationFrame` never fires), the output is showing a different workspace from the one the window landed on. Hyprland setups with per-monitor workspace blocks put a new output on its own block (`SS-TEST` can get 11). `headless.sh` therefore reads the workspace the output actually shows and points the test window rule there. Check with `hyprctl monitors all -j` and `hyprctl clients -j` (read-only). Never fix this with workspace or focus dispatches.

## Drive

Every command appends a line to `evidence/journal.log`: the action, its target and coordinates, and the value it read back. `ctl.sh help` lists them all.

```
$S/ctl.sh click '#toggle-search'              real click at the element's centre
$S/ctl.sh click 'text=Add to shelf'           by visible text or aria-label
$S/ctl.sh click 'book:text=Duplication'       inside the book (it renders in #book-host's shadow root)
$S/ctl.sh type 'duplication'                  key events into whatever has focus
$S/ctl.sh key Escape | key e | key s ctrl     named keys or one character, with ctrl/shift/alt
$S/ctl.sh wait "document.getElementById('search-action').textContent === 'Rerun'" 30
$S/ctl.sh eval "document.getElementById('where-title').textContent"
$S/ctl.sh shot 02-results                     evidence/02-results.png (1256x750 window)
$S/ctl.sh keep data/searches.json             copy a run file into evidence/
$S/ctl.sh drop /abs/book.epub                 the same path a file drag-and-drop takes
$S/ctl.sh save-answer /abs/out.epub           pre-answers the next native save dialog
$S/ctl.sh state                               app-side url, windows, fullscreen, zoom, phases
```

- `click` refuses an element that is hidden, outside the window, or covered by another element, and names what covers it. Act as a user would to get it into view: scroll, close the dialog, open the pane.
- Prefer stable handles: element ids from `app/ui/index.html` (`#toggle-search`, `#search-input`, `#mode-edit`, `#save`, `#toggle-comfort`, `#go-library`, `#next`), aria-labels through `text=`, and classes the e2e suites rely on (`.result`, `.card`, `.trow`).
- Read state from the DOM with `eval`. Use `wait` instead of `sleep`: the app is async (searches stream in batches, saves land after Escape).
- `drop` and `save-answer` are e2e seams. Native file pickers (`#shelf-open`, `#shelf-add`, Export) cannot be driven headless, so a proof that uses either must say so.
- `JSV_RUN=<run folder>` points any script at a run other than `current`.

## Evidence

Proof goes in `target/verify-app/<run>/evidence/`: `journal.log`, numbered PNGs, and copies made with `keep`. The run folder also keeps `commit`, `dirty`, `app.log` and `headless.log`. All of this survives `stop.sh`. `target/` is gitignored and `cargo clean` deletes it, so move anything you need to keep longer.

Proof standards:
- Use the real user path: clicks and keys on visible controls. A `ctl.sh eval` that calls `ipc.*`, sets `input.value`, or calls `.click()` is setup or inspection only, never the step being proven.
- Capture the action and the state it led to: a shot before and after, and the DOM value read back. A single final screenshot is not enough.
- Check side effects alongside the screen. Saves change the book copy in `books/` (inspect it with `target/release/jadeslip extract <book> <chapter>` or `unzip -p`). Settings land in `data/prefs.json`, searches in `data/searches.json`, and shelf state under `data/books/<key>/`. `keep` them before `stop.sh`, because cleanup deletes `data/` and `books/`.
- Say when you used `drop` or `save-answer`, which skip the native dialog.
- Report the run folder, the commit in `commit`, and whether `dirty` was empty.

## Cleanup

```
$S/stop.sh              the current run
$S/stop.sh <run folder> a specific one
```

It asks the app to quit over the socket, waits for `headless.sh` to exit (its trap closes the window and removes `SS-TEST`), and sends SIGTERM only to the `headless.sh` pid this run recorded. It never kills by process name. It then deletes `data/`, `tmp/`, `xdg/`, `books/` and `ctl.sock` and removes the `current` link. `evidence/` and the logs stay. If anything is still alive it exits 1, leaves the scratch files alone, and names what is left. Run it after every failed attempt too, so nothing holds the headless lock.

After it finishes, `hyprctl monitors all -j` shows no `SS-TEST` and `hyprctl clients -j` shows no `jadeslip-test` window. `scripts/verify.sh leftovers` checks the same things.

## Features

The feature map is in `features/README.md`, with one file per user-facing feature. A proof covers every entry point the map lists for the feature, not just the easiest one. When you find a new entry point or a new gotcha, add it to the map.
