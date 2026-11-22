# Edit and save

A writer switches the open book into Edit mode, changes the text in place, and leaves Edit. Leaving Edit, or pressing Ctrl+S, writes the change back into the EPUB file on disk. The book's original bytes are kept as a backup in the data folder, so Revert can restore them.

## Sub-features

- `edit-enter` enters Edit with `#mode-edit` or the `e` key, and leaves it with `#mode-read` or Escape.
- `edit-type` inserts typed text at the caret and marks the chapter dirty (`#save-label`, `#unsaved`).
- `edit-save` saves with `#save`, Ctrl+S, or by leaving Edit, and the EPUB file changes on disk.
- `edit-noop` enters and leaves Edit without typing and the file stays byte-identical.
- `edit-undo` undoes and redoes with `#tool-undo` and `#tool-redo`.
- `edit-revert` restores the backup with `#tool-revert`.

## How to get to it (user POV)

- Click `Edit` in the top-right Mode switch (`#mode-edit`), or press `e` while reading.
- Save with the `Save` button (`#save`), Ctrl+S, or by pressing Escape to go back to Read.

## Driving it with ctl.sh

Preconditions:

- Launched with `--book fixtures/generated/large-h1-div-br.epub` (or a real book), so the copy in `books/` is the file being written. Not yet proven end to end by this skill. The handles come from `tests/e2e/tests/edit.rs`.

- **Before bytes.** Run `sha256sum target/verify-app/current/books/*.epub > target/verify-app/current/evidence/before.sha256`.
- **Enter Edit.** Run `ctl.sh key e`, then `ctl.sh wait "document.getElementById('mode-edit').classList.contains('on')"`. Shoot `01-edit`.
- **Place the caret.** Run `ctl.sh click 'book:p'`. This is a real click into the first visible paragraph.
- **Type.** Run `ctl.sh type 'Zq'`, then wait for `#unsaved` to be shown. Shoot `02-dirty`.
- **Save by leaving.** Run `ctl.sh key Escape`, then wait for `mode-read` to be `.on` and `#unsaved` to be hidden.
- **Side effect.** The sha256 of the book copy differs from `before.sha256`. Run `target/release/jadeslip info <copy>` to find the chapter index, then `target/release/jadeslip extract <copy> <ix>` contains `Zq`. Save that output into evidence, and `keep data/books` for the backup.

## Gotchas

- Edit mode is a contenteditable in the shadow root. Keys go to whatever has focus, so click into the book text first or the typed characters drive reader shortcuts.
- Escape in Edit saves. The save itself runs asynchronously, so wait for `#unsaved` to hide before you read the file.
- `fixtures/real` must never be the file being written. `launch.sh` always opens a copy, so pass the original path and let it copy.
- `#fix-offer` or `#draft-offer` can appear for odd books or after a crash and cover the page. `click` names what covers the target.
