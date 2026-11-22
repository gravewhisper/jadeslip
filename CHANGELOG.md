# Changelog

Jadeslip uses [semantic versioning](https://semver.org). Before 1.0, a minor release (0.2, 0.3) can change behaviour or the data folder layout; the release notes say so when it does.

## 0.1.0 (2026-11-22)

The first public release. Linux only, as an AppImage, a `.deb`, or built from source.

### What's in it

- A library shelf with covers, progress, search and sorting.
- A reader with a contents sidebar, light and dark themes, adjustable type and layout, scroll or page view, and focus mode.
- Editing in place: press <kbd>E</kbd> to edit the chapter you are reading, <kbd>Esc</kbd> to save.
- Whole-book search, plain or regex, and book-wide replace with a preview.
- English spell check, with room for any Hunspell dictionary.
- Clean up book: one save removes web-novel watermarks, translator notes and repeated chapter titles.
- Adding, renaming, reordering and deleting chapters.
- Backups, history on every save, named snapshots, compare, and crash-recovery drafts.
- Bookmarks and notes, kept in the data folder rather than inside the book.
- Quick jump (<kbd>Ctrl</kbd>+<kbd>K</kbd>) and writing a new book from scratch.

### Safety work before release

A review of the archive reader, the book sanitiser, file handling, the app's internal commands and the save path found 13 problems, listed here in 12 lines. All are fixed, each with a test that failed before the fix:

- Restoring a history point after a cleanup in another window could bring back a different point.
- Two windows on one book could overwrite or delete each other's crash-recovery drafts.
- Deleting backups in one window made later saves in another window fail.
- An outside edit that kept the same size and timestamp could be overwritten without a warning.
- Two writes to the same data file could tear it, and a planted link could redirect a write outside the data folder.
- Two books with long, similar names could delete each other's save in progress.
- A restore from the trash could overwrite a newer file.
- `mailto:` links in a book could pass attachments and other fields to the mail program.
- Book styles could hide a chapter or override the dark theme.
- A hidden second end-of-archive record was missed in archives without a comment.
- Deeply nested contents made opening and saving slow.
- Storage use read 0 when the data folder was a symbolic link.

### Known issues

- A colour set with `!important` on a single element inside a book can still beat the dark theme.
- Regex search needs the `jadeslip-worker` program next to `jadeslip`. The AppImage and `.deb` include it.
