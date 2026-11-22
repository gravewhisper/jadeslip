import sys

from mutants import ROOT, chosen, lib_tests, not_run, run, test_file

SPELL = "crates/core/src/spell.rs"
WRITE = "crates/core/src/zip/write.rs"
APP_SPELL = "app/src/spell.rs"
APP_MAIN = "app/src/main.rs"
UI = "app/ui/spelling.js"
MAIN = "app/ui/main.js"
VIEW = "app/ui/chapter-view.js"
CORE = "core"
APP = "spelling"
WIRING = "wiring"

MUTATIONS = [
    ("a word keeps its apostrophes", SPELL, "let edge = |ch: char| is_apostrophe(ch) || is_joiner(ch);", "let edge = |ch: char| is_joiner(ch);", CORE, "spell::tests::the_mark_covers"),
    ("curly apostrophes are not folded", SPELL, ".map(|ch| if is_apostrophe(ch) { '\\'' } else { ch })", ".map(|ch| ch)", CORE, "spell::tests::a_curly_added"),
    ("non-Latin words are checked", SPELL, "if !is_latin(ch) {", "if false {", CORE, "spell::tests::only_latin"),
    ("accents are not folded", SPELL, "|| accent_free(&plain).is_some_and(|folded| dictionary.check(&folded))", "", CORE, "spell::tests::accents_and"),
    ("joiners split a word", SPELL, "ch.is_alphanumeric() || is_apostrophe(ch) || is_joiner(ch) || is_combining(ch)", "ch.is_alphanumeric() || is_apostrophe(ch) || is_combining(ch)", CORE, "spell::tests::accents_and"),
    ("names are marked", SPELL, "spans.retain(|span| !names.contains(name_key(&text[span.clone()])));", "", CORE, "spell::tests::a_capital_word"),
    ("a capital at a sentence start counts as a name", SPELL, "&& !starts_sentence(text, span.start)", "", CORE, "spell::tests::a_capital_word"),
    ("a possessive name is a different word", SPELL, "word.split(is_apostrophe).next().unwrap_or(word)", "word", CORE, "spell::tests::a_capital_word"),
    ("capitals are checked", SPELL, "letters >= 2 && lower", "letters >= 2", CORE, "spell::tests::a_capital_run"),
    ("addresses are checked", SPELL, "if !looks_like_address(chunk) {", "if true {", CORE, "spell::tests::numbers_capitals"),
    ("a hyphen joins words", SPELL, "ch.is_alphanumeric() || is_apostrophe(ch)", "ch == '-' || ch.is_alphanumeric() || is_apostrophe(ch)", CORE, "spell::tests::a_hyphen_splits"),
    ("a long run is checked", SPELL, "if word.len() > MAX_WORD_BYTES {", "if false {", CORE, "spell::tests::a_long_run"),
    ("raw offsets count bytes", SPELL, "units += text[seen..at].encode_utf16().count() as u32;", "units += (at - seen) as u32;", CORE, "spell::tests::raw_block_offsets"),
    ("an added word is not written", SPELL, 'writeln!(file, "{plain}")?;', "let _ = &mut file;", CORE, "spell::tests::an_added_word"),
    ("the word list is ignored at start", SPELL, "let _ = dictionary.add(&plain_form(line));", "let _ = line;", CORE, "spell::tests::an_added_word"),
    ("an added word is not usable until restart", SPELL, "dictionary\n            .add(&plain)\n            .map_err", "Ok::<(), std::convert::Infallible>(())\n            .map_err", CORE, "spell::tests::an_added_word"),
    ("non-words reach the list", SPELL, "&& text.chars().all(in_word)", "", CORE, "spell::tests::text_that_is_not"),
    ("a huge word list is read", SPELL, "&& fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_USER_LIST_BYTES)", "", CORE, "spell::tests::a_damaged"),
    ("a French book is checked", SPELL, "if !self.covers(reader) {", "if false {", CORE, "spell::tests::a_book_that_says"),
    ("the language gate ignores the tag", SPELL, "primary.is_empty()\n        || [", "true || primary.is_empty()\n        || [", CORE, "spell::tests::a_book_that_says"),
    ("marks have no cap", SPELL, ".into_iter()\n            .take(MAX_MARKS)\n            .map(|span| Misspelling {\n                range: visible", ".into_iter()\n            .map(|span| Misspelling {\n                range: visible", CORE, "spell::tests::marks_stop"),
    ("a capital word gets no lowercase suggestions", SPELL, "if lower != plain {", "if false {", CORE, "spell::tests::every_planted"),
    ("entries are written with a zlib wrapper", WRITE, "window_bits: -15,", "window_bits: 15,", CORE, "deflate_tests"),
    ("a compressed entry keeps its padding", WRITE, "body.truncate(written);", "", CORE, "deflate_tests"),
    ("the plain build lacks a command", APP_MAIN, '        spell::spell_add,\n    ]);\n    #[cfg(feature = "e2e")]', '    ]);\n    #[cfg(feature = "e2e")]', WIRING, "both_handler_lists"),
    ("first text waits for the check", MAIN, "      showOpened(opened);\n    }\n  } catch", "      showOpened(opened);\n      const ipc = await import('./ipc.js');\n      await ipc.spellChapter(opened.chapter.Ok.ix, opened.chapter.Ok.rev);\n    }\n  } catch", APP, "a_slow_check"),
    ("a turned-away chapter still paints", UI, "if (started !== turn || reply.state !== 'ranges') return;", "if (reply.state !== 'ranges') return;", APP, "a_turned_away"),
    ("an old revision still gets marks", APP_SPELL, "!= Some(rev)", "== None", APP, "an_old_revision"),
    ("marking writes into the DOM", UI, "    CSS.highlights.set(NAME, new Highlight(...visible));", "    for (const block of marks.keys()) block.setAttribute('data-ss-spell', '1');\n    CSS.highlights.set(NAME, new Highlight(...visible));", APP, "planted_typos"),
    ("the mark has no style", VIEW, "::highlight(ss-spell) { text-decoration: underline wavy #D9483B; }\n", "", APP, "the_marks_are_painted"),
    ("entering Edit drops the marks", MAIN, "  search.clearHighlights();\n  void setCloseGuard(true);", "  search.clearHighlights();\n  CSS.highlights.delete('ss-spell');\n  void setCloseGuard(true);", APP, "marks_stay"),
    ("Read shows the menu", UI, "    if (!isEditing()) return;\n", "", APP, "marks_stay"),
    ("Escape with the menu open leaves Edit", UI, "      event.stopPropagation();\n", "", APP, "escape_closes"),
    ("a chosen suggestion changes nothing", UI, "    document.execCommand('insertText', false, word);\n", "", APP, "a_right_click_fix"),
    ("add to dictionary does not clear the marks", UI, "      await recheck([...marks.keys()]);\n", "", APP, "add_to_dictionary"),
    ("the word list goes elsewhere", APP_SPELL, '"spelling-words.txt"', '"words.txt"', APP, "add_to_dictionary"),
    ("an edit is not rechecked", UI, "      recheck(blocks).catch(() => {});\n", "", APP, "a_new_typo"),
    ("a late chapter reply overwrites an edited block", UI, "if (!block || touched.has(block) || range.end.block", "if (!block || range.end.block", APP, "a_late_reply"),
    ("code is checked", UI, "    if (NOT_PROSE.has(at.localName)) return true;\n", "", APP, "planted_typos"),
    ("a tagged foreign passage is checked", UI, "  return language !== null && !ENGLISH.test(language);", "  return false;", APP, "planted_typos"),
    ("typing in a French book is checked", APP_SPELL, "if !speller.covers(&reader) {", "if false {", APP, "a_book_that_says"),
    ("going to the shelf keeps the closed book's marks", MAIN, "  spelling.cleared();\n  hideFailureBar();", "  hideFailureBar();", APP, "marks_come_on_mount"),
    ("a bare body is not marked", UI, "    if (root.hasAttribute(MARK)) blocks.set(Number(root.getAttribute(MARK)), root);\n", "", APP, "planted_typos"),
]


def build_app():
    return run("CARGO_TARGET_DIR=target/e2e nice cargo build --release -p jadeslip --features e2e")


def paths(mutation):
    _, file, _, _, suite, test = mutation
    if suite == CORE:
        return [file, *lib_tests("core", test)]
    return [file, test_file("e2e", APP if suite == WIRING else suite)]


def check(suite, test):
    if suite == CORE:
        return run(f"timeout -k 5 600 nice cargo test --release -p jadeslip-core --lib {test}")
    if suite == WIRING:
        return run(f"timeout -k 5 600 nice cargo test --release -p jadeslip-e2e --test {APP} {test}")
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
