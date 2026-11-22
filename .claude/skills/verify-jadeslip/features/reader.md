# Read a book

A reader opens an EPUB and reads it. The book renders chapter by chapter in the centre pane with a contents rail on the left, moves between chapters with buttons and keys, and remembers the reading place for next time.

## Sub-features

- `read-open` opens the book given on the command line and shows its first text.
- `read-next-prev` moves chapters with `#next` and `#prev`, and with Right and Left.
- `read-contents` jumps to a chapter from the contents rail (`#toc-list`), filtered by `#filter`.
- `read-jump` jumps with the Ctrl+K palette (`#palette-input`).
- `read-place` saves the reading place into `data/books/<key>/shelf.json` and reopens there.

## How to get to it (user POV)

- Start the app with a book path (`launch.sh --book`).
- Open a book from the library shelf (see library.md).
- Use the Previous and Next buttons in the bottom bar, the arrow keys, the contents rail, or Ctrl+K.

## Driving it with ctl.sh

Preconditions:

- Launched with `--book` on a real book, empty data folder, doctor clean. Not yet proven end to end by this skill. The handles come from `tests/e2e/tests/reader.rs` and `navigation.rs`.

- **Opened.** `launch.sh` waits for the `ss-first-text` mark. `ctl.sh eval "document.getElementById('where-title').textContent"` names the start chapter (`Colophon` for `tech_a`). Shoot `01-opened`.
- **Next chapter by button.** Run `ctl.sh click '#next'`, then wait for `#where-title` to change. Shoot after.
- **Next chapter by key.** Run `ctl.sh key Right`. Same check.
- **Contents rail.** Run `ctl.sh click 'text=Introduction'`, then wait for `#where-title` to read `Introduction`.
- **Place saved.** After a move, run `ctl.sh keep data/books`. The book's `shelf.json` names the chapter you moved to.
- **Place restored on reopen.** Not drivable yet. Every launch starts with an empty data folder, so reopening on the same data needs a `--data` option in `launch.sh` first. Report this step as skipped, with that reason.

## Gotchas

- Book content lives in `#book-host`'s shadow root, so `document.querySelector` cannot see it. Use `book:` selectors.
- The contents rail and the book both contain chapter titles. `text=Introduction` matches the first visible one, which is in the rail.
- The place is written on a timer and on close, not on every move. Read `shelf.json` after `stop.sh` has sent quit (copy it before cleanup by using `keep` after a short `wait`), or after a second move.
