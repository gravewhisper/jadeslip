# Jadeslip verification map

This folder is the maintained source for checking Jadeslip's user-facing behaviour in the running app. Read this index first, then use the matching feature file as the recipe. `S=.claude/skills/verify-jadeslip/scripts`, and every command runs from the repo root.

## Baseline preconditions

- No other headless run holds the lock: `launch.sh` refuses to start while one does.
- Start with `$S/launch.sh --book <epub>`, or with no `--book` for the library shelf. Each run gets an empty data folder, and `--book` opens a copy, never the original.
- `$S/doctor.sh` exits 0 before you drive anything.
- Good default books: `fixtures/real/tech_a.epub` (21 chapters, 57 hits for `duplication`). Generated books live in `fixtures/generated/` (`large-h1-div-br.epub` is the edit suite's book).
- Drive only the run this skill started. Never attach to the user's own Jadeslip window.

## Driving conventions

- Use `ctl.sh click|type|key` for every step being proven. These are real GDK events.
- Use `ctl.sh eval` only to read state or for setup that has no user path, and say which steps were setup.
- Use `ctl.sh wait <js>` for async results. Never sleep instead.
- Selectors: an id from `app/ui/index.html`, `text=<label or aria-label>`, or `book:<selector>` for content inside the book's shadow root.

## Proof and skip reporting

- Shoot before and after each state change (`ctl.sh shot NN-name`), and read the changed value back with `eval`.
- Copy every stored side effect into evidence with `ctl.sh keep` before `stop.sh`.
- Report the run folder, the feature ID, and the entry point used. An entry point that could not be reached is reported as skipped, with the reason. Never count it as covered by a different path.

## Feature entry contract

Each feature file has an H1 title, a one-paragraph description, then four H2s in this order: `Sub-features`, `How to get to it (user POV)`, `Driving it with ctl.sh`, `Gotchas`.

## Features

- [Search](./search.md) covers in-book search from the toolbar and Ctrl+F, opening a result, and recent searches written to `searches.json`. Proven end to end on 2026-11-10.
- [Read a book](./reader.md) covers opening a book, chapter navigation, and the reading place.
- [Edit and save](./edit-save.md) covers entering Edit, typing, and saving into the book file.
- [Library shelf](./library.md) covers the shelf with no book open, adding a book, and opening it from the shelf.
- [Text size, font and spacing](./comfort.md) covers the Aa panel, theme and font size, persisted to `prefs.json`.
