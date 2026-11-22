import sys

from mutants import ROOT, chosen, lib_tests, not_run, run, test_file

MAIN = "app/ui/main.js"
KEYS = "app/ui/keys.js"
TOC = "app/ui/toc.js"
MODEL = "app/ui/toc-model.js"
PALETTE = "app/ui/palette.js"
PALETTE_MODEL = "app/ui/palette-model.js"
PLACE = "app/ui/place.js"
FOCUS = "app/ui/focus.js"
TEXTMAP = "app/ui/textmap.js"
VIEW = "app/ui/chapter-view.js"
PREFS = "crates/core/src/prefs.rs"
SESSION = "app/src/session.rs"
COMMANDS = "app/src/commands.rs"
READER = "reader"
ATTACK = "attack"

MUTATIONS = [
    ("Space never moves to the next chapter", MAIN, "else turnPage('next');\n}\n\nfunction pageUp", "}\n\nfunction pageUp", READER, "space_and_arrows"),
    ("Space types nothing in a field", KEYS, "if (key.inField) return inField(key, look);", "", READER, "space_and_arrows"),
    ("Esc leaves focus before saving the edit", KEYS, "if (look === 'edit') return 'esc';\n  return layers.focus ? 'focusExit' : 'paneEsc';", "if (layers.focus) return 'focusExit';\n  return look === 'edit' ? 'esc' : 'paneEsc';", READER, "in_focus_esc"),
    ("focus hides nothing", "app/ui/app.css", ".app.focus .top,.app.focus .toolbar,.app.focus .rail,.app.focus .footrow,.app.focus .offerbar,.app.focus .pop{display:none}", "", READER, "focus_hides"),
    ("the edge ignores whether the pointer moved there", FOCUS, "if (atEdge && armed) set(false);", "if (atEdge) set(false);", READER, "focus_hides"),
    ("the right edge leaves focus", FOCUS, "x <= EDGE_PX || y <= EDGE_PX", "x <= EDGE_PX || x >= width - 1 - EDGE_PX || y <= EDGE_PX", READER, "focus_hides"),
    ("a font change keeps pixels instead of text", PLACE, "    change();\n    apply(place);\n", "    change();\n", READER, "the_same_text"),
    ("the point at a node boundary reads the block top", TEXTMAP, "if (units < run.before + run.node.data.length) return { run, offset: units - run.before };", "return null;", READER, "the_same_text"),
    ("a launch ignores the saved place", MAIN, "if (opened.place) places.restore(opened.place);", "", READER, "the_place_comes_back"),
    ("a launch parses the contents first", SESSION, "let (start, place) = match saved.start_of(&reader) {", "let _ = reader.toc();\n    let (start, place) = match saved.start_of(&reader) {", READER, "the_place_comes_back"),
    ("corrupt prefs lose the shape guard", PREFS, ".filter(serde_json::Value::is_object)", "", "core", "corrupt_files"),
    ("the rail renders every row", MODEL, "return { first, end: Math.max(first, end) };", "return { first: 0, end: rows };", READER, "the_contents_stay"),
    ("a filter ignores numbers", MODEL, "return entry.folded.includes(query) || (entry.digits !== '' && entry.digits.includes(numberQueryOf(query)));", "return entry.folded.includes(query);", None, None),
    ("folding hides nothing", MODEL, "if (collapsed.has(at)) {", "if (false) {", READER, "nested_nav"),
    ("the current row never comes into view", TOC, "if (next !== list.scrollTop) list.scrollTop = next;", "", READER, "the_current_row"),
    ("a row click skips the unsaved prompt", MAIN, "function openChapter(ix, fragment = null) {\n  flushDirty();\n  return goTo(ix, fragment);", "function openChapter(ix, fragment = null) {\n  return navigate(ix, fragment);", READER, "the_current_row"),
    ("placeholders never fill", TOC, "if (toc.source === 'headings') startHeadings();", "", READER, "no_toc_starts"),
    ("a rail label parses HTML", TOC, "slot.title.textContent = entry.label;", "slot.title.innerHTML = entry.label;", ATTACK, "hostile_contents"),
    ("a palette label parses HTML", PALETTE, "label.textContent = row.label;", "label.innerHTML = row.label;", ATTACK, "hostile_contents"),
    ("a huge label is not cut", COMMANDS, "label: entry.label.chars().take(MAX_LABEL_CHARS).collect(),", "label: entry.label.clone(),", ATTACK, "hostile_contents"),
    ("a palette number uses the spine index", PALETTE_MODEL, "const ix = toc.linear[number - 1];", "const ix = number;", READER, "quick_jump_finds"),
    ("the palette loses the caret", MAIN, "if (machine.mode === 'edit' && session) refocusEditor();\n  else returnTo", "if (false) refocusEditor();\n  else returnTo", READER, "quick_jump_in_edit"),
    ("a book can hide its chapter", VIEW, "display: block !important; visibility: visible !important; opacity: 1 !important;", "", READER, "the_book_cannot"),
    ("the shell default beats book CSS", VIEW, ":where(.ss-book p) { margin: 0 0 1.05em; }", ".ss-book p { margin: 0 0 1.05em; }", READER, "the_book_cannot"),
    ("dark theme keeps book ink", VIEW, "color: var(--ink) !important; background-color: transparent !important;", "", READER, "the_book_cannot"),
]


def build():
    return run("CARGO_TARGET_DIR=target/e2e cargo build --release -p jadeslip --features e2e")


NODE = ["tests/ui/keys.test.mjs", "tests/ui/toc-model.test.mjs"]


def paths(mutation):
    _, file, _, _, suite, test = mutation
    if suite is None:
        return [file, *NODE]
    if suite == "core":
        return [file, *lib_tests("core", test)]
    return [file, test_file("e2e", suite)]


def check(suite, test):
    if suite is None:
        return run(["timeout", "-k", "5", "120", "node", "--test", *NODE])
    if suite == "core":
        return run(f"timeout -k 5 600 cargo test --release -p jadeslip-core --lib {test}")
    ok, output = build()
    if not ok:
        return None, output
    return run(f"timeout -k 5 300 tests/e2e/headless.sh cargo test --release -p jadeslip-e2e --test {suite} -- --ignored --test-threads=1 {test}")


def main():
    picked = chosen(MUTATIONS, paths)
    survived = []
    tried = 0
    try:
        for name, file, old, new, suite, test in picked:
            tried += 1
            path = ROOT / file
            source = path.read_text()
            if source.count(old) != 1:
                print(f"STALE {name}: the source text occurs {source.count(old)} times")
                survived.append(name)
                continue
            path.write_text(source.replace(old, new))
            try:
                passed, output = check(suite, test)
            finally:
                path.write_text(source)
            reason = "does not build" if passed is None else not_run(output)
            if reason:
                print(f"BROKEN {name}: {reason}\n{output[-600:]}")
                survived.append(name)
            elif passed:
                print(f"SURVIVED {name}")
                survived.append(name)
            else:
                print(f"caught {name}", flush=True)
    finally:
        ok, output = build()
        print("restored and rebuilt" if ok else output[-600:])
    print(f"{tried - len(survived)} of {tried} caught")
    return 1 if survived else 0


if __name__ == "__main__":
    sys.exit(main())
