# Installed copy

Regex search runs in `jadeslip-worker`, which the app looks for beside its own binary. A plain `target/release/jadeslip` copied alone keeps working, but regex search then says it could not start.

To make a copy that has both binaries together, in a folder you choose:

```sh
scripts/install-local.sh ~/.local/opt/jadeslip
~/.local/opt/jadeslip/bin/jadeslip book.epub
```

The script runs `cargo install` twice with `--root`, so nothing goes outside that folder. It also copies the dictionary's licence notice and the app's `LICENSE` to `share/doc/jadeslip/`. Move the folder as a whole, because the two binaries must stay side by side. The UI, fonts and dictionary are compiled into the app binary, so no other files are needed. The machine still needs `webkit2gtk-4.1`.

To package it yourself, `scripts/package.sh` builds an AppImage and a `.deb`, which needs `cargo tauri`.

## Spelling in other languages

English is built in. For any other language, add a Hunspell dictionary, the `.aff` and `.dic` pair that LibreOffice, Firefox and most Linux distributions use:

```sh
mkdir -p ~/.local/share/jadeslip/dictionaries
cp /usr/share/hunspell/de_DE.aff ~/.local/share/jadeslip/dictionaries/de.aff
cp /usr/share/hunspell/de_DE.dic ~/.local/share/jadeslip/dictionaries/de.dic
```

Both files need the same name, made of letters, digits, `-` or `_`. The `SET` line in the `.aff` file names its character set, and files in UTF-8 or a legacy set such as ISO8859-1 both work. Good sources are your distribution's `hunspell-*` packages, the LibreOffice dictionaries, and `github.com/wooorm/dictionaries`. Check each dictionary's licence before you share it.

A book is checked with the dictionary that matches its language tag, first exactly and then by the part before the `-`, so `de.aff` serves a book tagged `de-AT`. Books with no matching dictionary keep the English behaviour. To pick a dictionary by hand for one book, open Text size, font and spacing (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>A</kbd>), go to its Spelling tab and choose under Spelling language. Spelling marks turn on and off in Preferences (<kbd>Ctrl</kbd>+<kbd>,</kbd>), under Spelling. A dictionary that cannot be read shows a message and is skipped.
