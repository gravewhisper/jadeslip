# Library shelf

With no book open, Jadeslip shows the library: a shelf of books the user has added, with covers, recent reading, filtering, sorting, and a detail panel. Adding a book keeps a managed copy in the data folder. Opening a card opens that book in the reader.

## Sub-features

- `shelf-empty` shows the empty state with Add and New actions (`#shelf-empty`).
- `shelf-add` adds a book by drag and drop, by `#shelf-add` (native picker), or with `Add to shelf` (`#add-to-shelf`) from the reader.
- `shelf-open` opens a book from its card (`#shelf-grid .card .cardgo`) or the list row (`#shelf-list .trow`).
- `shelf-filter` filters by title or author (`#shelf-filter`).
- `shelf-new` creates a new blank book (`#shelf-new`, `#newbook-dialog`).
- `shelf-remove` removes a book from the shelf via the book menu (`#detail-more`, `#menu-remove`, `#remove-dialog`).

## How to get to it (user POV)

- Start with no book (`launch.sh` with no `--book`), or click the library icon (`#go-library`, Ctrl+L) in the reader.
- Drag EPUB files onto the window, or use the Add button.
- From an open book, click `Add to shelf`.

## Driving it with ctl.sh

Preconditions:

- Launched with no `--book`, empty data folder, doctor clean. Not yet proven end to end by this skill. The handles come from `tests/e2e/tests/library.rs`.

- **Empty shelf.** `ctl.sh eval "!document.getElementById('shelf-empty').hidden"` is `true`. Shoot `01-empty`.
- **Add by drop.** Copy a book into the run first: `cp fixtures/real/small_a.epub target/verify-app/current/books/`. Then run `ctl.sh drop target/verify-app/current/books/small_a.epub` and wait for `document.querySelectorAll('#shelf-grid .card').length === 1`. Shoot `02-added`. Report that the drop used the e2e seam.
- **Add from the reader.** Launch with `--book`, then run `ctl.sh click '#add-to-shelf'` and `ctl.sh click '#go-library'`. One card is shown.
- **Open from shelf.** Run `ctl.sh click '#shelf-grid .card .cardgo'`, then wait for `#reader` to be shown and the `ss-first-text` mark.
- **Side effect.** Run `ctl.sh keep data/books`. It holds one `<32-hex key>/` folder with `shelf.json`, a cover, and the managed copy of the book.

## Gotchas

- `#shelf-add` and `#shelf-open` open the GTK file picker, which cannot be answered headless. Use `drop`, or the `Add to shelf` path from the reader, and say which.
- Covers are filled in asynchronously (`fill_covers`). Wait for the card rather than its image.
- A book added from `fixtures/real` directly would make the shelf point at a fixture. Always drop a copy from the run's `books/`.
