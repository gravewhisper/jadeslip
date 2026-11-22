import sys

from mutants import ROOT, chosen, lib_tests, node_tests, not_run, run, test_file

ANCHOR = "crates/core/src/anchor.rs"
LIBRARY = "crates/core/src/library.rs"
LINKS = "crates/core/src/manage/links.rs"
COMMIT = "crates/core/src/commit.rs"
BACKUP = "crates/core/src/backup.rs"
SESSION = "app/src/session.rs"
APP_LIBRARY = "app/src/library.rs"
COMMANDS = "app/src/commands.rs"
MAIN = "app/ui/main.js"
MARKS = "app/ui/marks.js"
SHELF = "app/ui/shelf.js"
SHELF_MODEL = "app/ui/shelf-model.js"
REVERT = "app/ui/revert.js"
SCHEMES = "app/src/schemes.rs"
PLACE = "app/ui/place.js"
CORE = "core"
APP = "library"
NODE = "ui_node"

MUTATIONS = [
    ("resolve ignores context", ANCHOR, "let score = context_score(anchor, text, start, end);", "let score = 0;", CORE, "anchor::tests::the_right_copy"),
    ("a short word is guessed", ANCHOR, "best.filter(|found| lone || found.score >= MIN_CONTEXT_CHARS)", "best", CORE, "anchor::tests::a_short_word"),
    ("a typo inside a mark detaches it", ANCHOR, "best_occurrence(anchor, text).or_else(|| bridged(anchor, text))", "best_occurrence(anchor, text)", CORE, "anchor::tests::a_typo"),
    ("a moved mark claims to be exact", ANCHOR, "let in_place = found.range.start == anchor.vis_hint\n", "let in_place = true || found.range.start == anchor.vis_hint\n", CORE, "anchor::tests::an_edit_before"),
    ("a gone file is never scanned for", ANCHOR, "if !homeless.is_empty() {", "if false {", CORE, "library::tests::a_mark_whose_file"),
    ("a detached mark is dropped from the list", APP_LIBRARY, ".map(|(mark, resolution)| mark_row(&reader, mark, resolution))\n            .rev()", ".filter(|(_, resolution)| !matches!(resolution, AnchorResolution::Detached))\n            .map(|(mark, resolution)| mark_row(&reader, mark, resolution))\n            .rev()", APP, "marks_survive"),
    ("a damaged marks file is overwritten", LIBRARY, "Some(bytes) => parse_marks(&bytes),", "Some(bytes) => Ok(parse_marks(&bytes).unwrap_or_default()),", CORE, "library::tests::a_damaged_marks"),
    ("a mark at the top detaches on a rename", ANCHOR, "if anchor.vis_hint == 0 && anchor.prefix.is_empty() {", "if false {", CORE, "anchor::tests::a_mark_at_the_top"),
    ("a raw file name panics the shelf", LIBRARY, '#[serde(with = "path_text")]\n', "", CORE, "library::tests::a_file_name"),
    ("a bad record is dropped with no copy", LIBRARY, "if lost_some {\n                        let _ = fs::copy(&path, damaged_path(&path));\n                    }\n                    *records = fresh;", "let _ = lost_some;\n                    *records = fresh;", CORE, "library::tests::a_record_that"),
    ("the contents page reverts alone", COMMIT, "if is_package {", "if false {", CORE, "manage::tests::the_contents_page"),
    ("a quote in an id breaks the tag", LINKS, """raw.replace('"', "&quot;")""", "raw.to_owned()", CORE, "manage::tests::delete_unwraps"),
    ("a book switch keeps old questions", MAIN, "  if (chapters.isAsking()) chapters.cancelAsking();\n  if (revert.isAsking()) revert.cancel();\n", "", APP, "a_book_switch"),
    ("the picker lets keys through", MAIN, "if (picking || applier.isBusy()", "if (applier.isBusy()", APP, "a_book_switch"),
    ("a selection follows to another chapter", MAIN, "  marks.forgetSelection();\n", "", APP, "marks_survive"),
    ("the last book opened does not lead", SHELF_MODEL, "opened: { label: 'Last opened', compare: (a, b) => b.touchedAt - a.touchedAt", "opened: { label: 'Last opened', compare: (a, b) => a.touchedAt - b.touchedAt", APP, "a_book_opens"),
    ("a warning is hidden behind the done toast", "app/ui/chapters.js", "            await finish(result);\n            if (result.warnings.length > 0) showToast(result.warnings.join(' '));", "            if (result.warnings.length > 0) showToast(result.warnings.join(' '));\n            await finish(result);", APP, "a_mounted_linking"),
    ("the writing point is ignored on open", SESSION, "entry.write_point.or(entry.read_point)", "entry.read_point", APP, "an_own_book"),
    ("a damaged library is lost, not set aside", LIBRARY, "None => {\n                    set_aside(&path);\n                    Records::new()", "None => {\n                    Records::new()", CORE, "library::tests::a_bad_record"),
    ("the shelf is keyed by the raw fingerprint", LIBRARY, "let key = BookKey::of(&book.identity());\n        let at = now_ms();", "let key = BookKey::of(&summary.fingerprint);\n        let at = now_ms();", CORE, "library::tests::a_package_edit"),
    ("another window's record is lost", LIBRARY, "*records = fresh;", "let _ = fresh;", CORE, "library::tests::a_write_keeps"),
    ("the writing point follows reading", LIBRARY, "if writing {\n                    entry.write_point = Some(point.clone());\n                }", "entry.write_point = Some(point.clone());", CORE, "library::tests::reading_and_writing"),
    ("an export can replace the book", COMMIT, "if fs::canonicalize(dest).is_ok_and(|found| found == self.path) {", "if false {", CORE, "library::tests::an_export"),
    ("delete leaves inbound links", COMMIT, "for chapter in rewritten {\n                        record.linking.push(chapter.window);\n                        link_writes.push((chapter.name, chapter.bytes));\n                    }", "let _ = rewritten;", CORE, "manage::tests::delete_unwraps"),
    ("undo skips the linking chapters", COMMIT, "Some(found) => writes.push(found),", "Some(_) => {}", CORE, "manage::tests::unwrap_then_undo"),
    ("undo overwrites a later edit", LINKS, "if crc32fast::hash(current) != self.after_crc {", "if false {", CORE, "manage::tests::an_edit_after"),
    ("an out of order undo brings a dangling link", COMMIT, "if let Some((name, bytes)) = &mut plan.added\n            && let Ok(Some(unwrapped))", "if let Some((name, bytes)) = &mut plan.added\n            && false\n            && let Ok(Some(unwrapped))", CORE, "manage::tests::undoing_out_of_order"),
    ("a chapter revert brings a dangling link", COMMIT, "if let Some(unwrapped) = unwrap_links(&bytes, &name, &gone, &self.limits)? {", "if let Some(unwrapped) = unwrap_links(&bytes, &name, &BTreeSet::new(), &self.limits)? {", CORE, "manage::tests::reverting_a_linking"),
    ("an id on a link is lost", LINKS, ".find(|attr| attr.name == \"id\")", ".find(|attr| attr.name == \"never\")", CORE, "manage::tests::delete_unwraps"),
    ("a link keeps its closing tag", LINKS, "                if let Some(at) = found\n                    && let Some(text) = held.remove(at).close_as\n", "                if let Some(at) = found\n                    && false\n                    && let Some(text) = held.remove(at).close_as\n", CORE, "manage::tests::delete_unwraps"),
    ("the link fix does not check the text", LINKS, "if after.text.as_str() != model.text.as_str() {", "if false && after.text.as_str() != model.text.as_str() {", CORE, "manage::tests::delete_unwraps"),
    ("gbk is not kept through unwrap", LINKS, "let (bytes, warnings) = finish_bytes(&model, edits);", "let bytes = crate::markup::splice(&model.source, edits).into_bytes();\n    let warnings = Vec::new();", CORE, "manage::tests::a_gbk"),
    ("a mismatched backup is offered", BACKUP, ".filter(|slot| slot.ledger.matches(current))", ".filter(|_| true)", APP, "a_leftover"),
    ("a damaged backup is offered", BACKUP, ".find(|slot| slot.open_verified().is_ok())?;", ".next()?;", APP, "a_leftover"),
    ("launch opens the last book", SESSION, "None => Opening::NoBook,", "None => {\n                    if let Some((_, entry)) = opener.library().ok().and_then(|library| library.shelf().pop()) {\n                        let _ = opener.open_path(&entry.path, false);\n                    }\n                    Opening::NoBook\n                }", APP, "launch_opens_no_epub"),
    ("the shelf reads each book at launch", APP_LIBRARY, "    library\n        .shelf()\n        .into_iter()\n        .map(|(key, entry)| ShelfRow {", "    library\n        .shelf()\n        .into_iter()\n        .inspect(|(_, entry)| {\n            let _ = std::fs::read(&entry.path);\n        })\n        .map(|(key, entry)| ShelfRow {", APP, "launch_opens_no_epub"),
    ("a new UI file is not served", SCHEMES, "    (\n        \"shelf-model.js\",\n        \"text/javascript; charset=utf-8\",\n        include_bytes!(\"../ui/shelf-model.js\"),\n    ),\n", "", APP, "launch_opens_no_epub"),
    ("the Library button skips the unsaved prompt", MAIN, "function goLibrary() {\n  flushDirty();\n  return requestWholeBook(async () => {", "function goLibrary() {\n  flushDirty();\n  return (async (run) => run())(async () => {", APP, "leaving_for_the_shelf"),
    ("the place is not saved on close", MAIN, "    places.flush().finally(() => closeWindow());\n  });\n  window.addEventListener('ss-book-dropped'", "    closeWindow();\n  });\n  window.addEventListener('ss-book-dropped'", APP, "the_place_is_saved"),
    ("the shelf ignores the arrows", SHELF, "      chosen = key;\n      mark(key, true);", "      mark(key, true);", APP, "the_shelf_leads"),
    ("the place is named by position", SHELF_MODEL, "    line: label ? `${label}.` : `${chapters}.`,", "    line: `Chapter ${count(book.read.ordinal)} of ${count(book.chapters)}.`,", NODE, "the_edit_machine"),
    ("a removed book forgets its place", LIBRARY, "                read_capped(&stash)\n                    .ok()\n                    .flatten()\n                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())\n                    .unwrap_or_else(", "                None.unwrap_or_else(", CORE, "library::tests::a_removed_book"),
    ("a failed trash keeps what already moved", LIBRARY, "                        rollback(moved);\n                        return Err(trash_failed(\"Jadeslip's data for this book\", &error));", "                        return Err(trash_failed(\"Jadeslip's data for this book\", &error));", CORE, "library::tests::a_data_folder"),
    ("another book's alias goes to the trash", LIBRARY, ".is_some_and(|root| BookKey::of(&crate::Fingerprint(root.0)) == key)", ".is_some_and(|_| true)", CORE, "library::tests::removing_with"),
    ("words are counted for every book", LIBRARY, "let Some(entry) = self.entry(key).filter(|entry| entry.own) else {\n            return Ok(false);\n        };\n        if entry.words", "let Some(entry) = self.entry(key) else {\n            return Ok(false);\n        };\n        if entry.words", CORE, "library::tests::a_stale_reader"),
    ("the folder comes from the webview", APP_LIBRARY, "        crate::external::reveal(folder);", "        crate::external::reveal(std::path::Path::new(&key));", APP, "the_book_menu"),
    ("the revert list hides deleted chapters", REVERT, "restores.replaceChildren(...deleted.map((row) => {", "restores.replaceChildren(...[].map((row) => {", APP, "revert_restores"),
    ("the book is not reloaded after a revert", REVERT, "await bookReshaped(result);\n        return {", "return {", APP, "revert_restores"),
    ("a mark jump does not highlight", MAIN, "return goTo(row.ix, null, { hit: { range: row.range }, rev: row.rev });", "return goTo(row.ix);", APP, "marks_survive"),
    ("a note goes in the page as markup", MARKS, "text.textContent = row.note ?? (row.quote || 'Bookmark');", "text.innerHTML = row.note ?? (row.quote || 'Bookmark');", APP, "hostile_note"),
    ("a shelf title goes in the page as markup", SHELF, "  if (text !== undefined) node.textContent = text;", "  if (text !== undefined) node.innerHTML = text;", APP, "hostile_note"),
]


def build_app():
    return run("CARGO_TARGET_DIR=target/e2e nice cargo build --release -p jadeslip --features e2e")


def paths(mutation):
    _, file, _, _, suite, test = mutation
    if suite == CORE:
        return [file, *lib_tests("core", test)]
    if suite == NODE:
        return [file, test_file("e2e", NODE), *node_tests()]
    return [file, test_file("e2e", suite)]


def check(suite, test):
    if suite == CORE:
        return run(f"timeout -k 5 600 nice cargo test --release -p jadeslip-core --lib {test} -- --include-ignored")
    if suite == NODE:
        return run(f"timeout -k 5 600 nice cargo test --release -p jadeslip-e2e --test ui_node {test}")
    ok, output = build_app()
    if not ok:
        return None, output
    return run(f"timeout -k 5 300 tests/e2e/headless.sh cargo test --release -p jadeslip-e2e --test {suite} -- --ignored --test-threads=1 {test}")


def main():
    picked = chosen(MUTATIONS, paths)
    survived = []
    tried = 0
    current = {}

    def restore():
        for path, source in current.items():
            path.write_text(source)
        current.clear()

    try:
        for name, file, old, new, suite, test in picked:
            tried += 1
            path = ROOT / file
            source = path.read_text()
            if source.count(old) != 1:
                print(f"STALE {name}: the source text occurs {source.count(old)} times", flush=True)
                survived.append(name)
                continue
            current[path] = source
            path.write_text(source.replace(old, new))
            try:
                passed, output = check(suite, test)
            finally:
                restore()
            reason = "does not build" if passed is None else not_run(output)
            if reason:
                print(f"BROKEN {name}: {reason}\n{output[-600:]}", flush=True)
                survived.append(name)
            elif passed:
                print(f"SURVIVED {name}", flush=True)
                survived.append(name)
            else:
                print(f"caught {name}", flush=True)
    finally:
        restore()
        build_app()
    print(f"{tried - len(survived)} of {tried} caught", flush=True)
    sys.exit(1 if survived else 0)


if __name__ == "__main__":
    main()
