import sys

from mutants import ROOT, chosen, lib_tests, not_run, run, test_file

PLAN = "crates/core/src/manage/plan.rs"
TOC_EDIT = "crates/core/src/manage/toc_edit.rs"
MANAGE = "crates/core/src/manage.rs"
COMMIT = "crates/core/src/commit.rs"
BACKUP = "crates/core/src/backup.rs"
CHECK = "crates/core/src/manage/check.rs"
CHAPTERS = "app/ui/chapters.js"
MAIN = "app/ui/main.js"
TOC = "app/ui/toc.js"
SESSION = "app/src/session.rs"
CORE = "core"
APP = "chapters"
ATTACK = "attack"

MUTATIONS = [
    ("a failing plan is written anyway", COMMIT, "check_plan(&head, &package, &plan, &self.limits)?;", "let _ = check_plan(&head, &package, &plan, &self.limits);", CORE, "manage::tests::a_failing_plan"),
    ("the identity alias is never written", BACKUP, "if new == root || path.exists() {", "if true {", CORE, "manage::tests::a_later_launch"),
    ("commit skips the alias", COMMIT, "        if reader.summary().fingerprint != head.summary().fingerprint {\n            record_alias(", "        if false {\n            record_alias(", CORE, "manage::tests::a_real_kill"),
    ("the place forgets the identity", SESSION, ".save_resume_point(BookKey::of(&book.identity()), writing, resume)", ".save_resume_point(BookKey::of(&reader.summary().fingerprint), writing, resume)", APP, "a_saved_place"),
    ("titles are not escaped", MANAGE, "'<' => out.push_str(\"&lt;\"),", "'<' => out.push('<'),", CORE, "manage::tests::hostile_titles"),
    ("an emptied nested list stays", TOC_EDIT, "edits.cut(with_ws(&toc.source, nested));", "let _ = nested;", CORE, "manage::tests::delete_cuts"),
    ("playOrder is never renumbered", PLAN, "    if play_orders_ok(&after) {\n        return Ok(encode_text(toc.text_form, text));\n    }", "    if true {\n        return Ok(encode_text(toc.text_form, text));\n    }", CORE, "manage::tests::play_order_follows"),
    ("guide references stay after a delete", PLAN, "opf_edits.cut(with_ws(source, guide.span));", "let _ = guide;", CORE, "manage::tests::a_guide_keeps"),
    ("landmarks stay after a delete", PLAN, "self.cut_other_links(doc, name, &mut edits);", "let _ = name;", CORE, "manage::tests::delete_cuts"),
    ("a new file can reuse a deleted name", PLAN, ".chain(&self.reserved_names)", ".chain(&Vec::new())", CORE, "manage::tests::a_new_id"),
    ("undo drops the saved package edits", PLAN, "Some(apply_inverse(text, &saved.edits))", "None", CORE, "manage::tests::delete_then_undo"),
    ("a group's chapters stay behind", PLAN, "let seed = doc.subtree_rows(at);", "let seed: Vec<u32> = doc.rows[at as usize].into_iter().collect();", CORE, "manage::tests::moving_a_group"),
    ("a section row can be moved", PLAN, "                if let Some(row) = doc.rows[at as usize]\n                    && doc.first_entry_of(row) != Some(at)\n                {", "                if let Some(row) = doc.rows[at as usize]\n                    && false && doc.first_entry_of(row) != Some(at)\n                {", CORE, "manage::tests::a_section_row"),
    ("a whole-book revert keeps deleted records", COMMIT, "let _ = slot.clear_deleted();", "", CORE, "manage::tests::a_whole_book_revert_clears"),
    ("the rail skips the unsaved prompt", CHAPTERS, "return requestWhole(async () => {\n      running = true;", "return (async (run) => run())(async () => {\n      running = true;", APP, "a_book_is_made"),
    ("the rail is not reloaded after an op", MAIN, "  search.bookChanged();\n  await toc.load();", "  search.bookChanged();", APP, "a_book_is_made"),
    ("a new chapter opens in Read", MAIN, "  enterPoint = writingPoint(host.shadowRoot);\n  requestEnter();", "", APP, "a_book_is_made"),
    ("a deleted row parses its title", CHAPTERS, "title.textContent = row.title;", "title.innerHTML = row.title;", ATTACK, "hostile_titles"),
    ("a drag commits nothing", TOC, "if (commit && ended?.started && ended.spot) chapters.move(ended.entry, ended.spot);", "", APP, "a_book_is_made"),
    ("the search pane keeps stale results", MAIN, "  search.bookChanged();\n", "", APP, "the_search_pane"),
]


def build_app():
    return run("CARGO_TARGET_DIR=target/e2e nice cargo build --release -p jadeslip --features e2e")


def paths(mutation):
    _, file, _, _, suite, test = mutation
    return [file, *(lib_tests("core", test) if suite == CORE else [test_file("e2e", suite)])]


def check(suite, test):
    if suite == CORE:
        return run(f"timeout -k 5 600 nice cargo test --release -p jadeslip-core --lib {test} -- --include-ignored")
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
        if any(row[4] != CORE for row in MUTATIONS):
            run("nice cargo build --release -p jadeslip -p jadeslip-worker")
    print(f"{tried - len(survived)} of {tried} caught", flush=True)
    sys.exit(1 if survived else 0)


if __name__ == "__main__":
    main()
