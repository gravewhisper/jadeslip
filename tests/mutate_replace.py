import sys

import mutants
from mutants import ROOT

REPLACE = "crates/core/src/replace.rs"
SEARCH = "crates/core/src/search.rs"
CHAPTER = "crates/core/src/chapter.rs"
COMMIT = "crates/core/src/commit.rs"
WORKER = "crates/core/src/worker.rs"
PANE = "app/ui/search.js"
MODEL = "app/ui/replace-model.js"
MAIN = "app/ui/main.js"
FIX = "app/ui/fix-offer.js"
MACHINE = "app/ui/edit-machine.js"

CORE = ("core", None)
POOL = ("worker", "replace")
E2E = ("e2e", "replace")
ATTACK = ("e2e", "attack")

MUTATIONS = [
    ("exclusions are ignored", REPLACE, "if hit.crosses_block || hit.same || is_out {", "let _ = is_out;\n            if hit.crosses_block || hit.same {", CORE, "replace::tests::exclusions"),
    ("a chapter exclusion is ignored", REPLACE, "touched.changes > 0 && !chapters_out.contains(&touched.chapter)", "touched.changes > 0 && (chapters_out.is_empty() || true)", CORE, "replace::tests::exclusions"),
    ("ordinals skip refused hits", REPLACE, "for (ordinal, hit) in job.hits.iter().enumerate() {", "for (ordinal, hit) in job.hits.iter().filter(|hit| !hit.crosses_block).enumerate() {", POOL, "exclusions_count"),
    ("the replacement is not escaped", CHAPTER, "let escaped = escape_text(replacement);", "let escaped = replacement.to_owned();", CORE, "replace::tests::a_replacement_cannot"),
    ("the tag check is gone", REPLACE, "if tags(&old.source, &old_scan.events).eq(tags(&new.source, &new_scan.events)) {", "if tags(&old.source, &old_scan.events).count() < usize::MAX {", CORE, "replace::tests::a_replacement_cannot",
     (CHAPTER, "let escaped = escape_text(replacement);", "let escaped = replacement.to_owned();")),
    ("a block crossing is not refused", REPLACE, "if hit.crosses_block || hit.same || is_out {", "if hit.same || is_out {", CORE, "replace::tests::a_refused_match"),
    ("a block crossing is not detected", CHAPTER, ".any(|seg| seg.kind == SegKind::BlockEnd)", ".any(|_seg| false)", CORE, "replace::tests::a_match_across_a_block"),
    ("a block crossing is counted as a change", REPLACE, "if hit.crosses_block {\n                self.skipped += 1;\n            } else if hit.same {", "if hit.same {", CORE, "replace::tests::a_match_across_a_block"),
    ("a failed take still commits", REPLACE, "let built = self.built_at(position)?;", "let Ok(built) = self.built_at(position) else {\n            return Ok(EntryFate::RawCopy);\n        };", POOL, "a_chapter_going_past"),
    ("a limited chapter does not abort", REPLACE, "SearchEvent::ChapterLimited { .. } | SearchEvent::CapReached => {\n                    self.fail(too_slow());\n                }", "SearchEvent::ChapterLimited { .. } | SearchEvent::CapReached => {}", POOL, "a_chapter_going_past",
     (REPLACE, "SearchEnd::TooSlow { .. } => self.fail(too_slow()),", "SearchEnd::TooSlow { .. } => self.moved.notify_all(),")),
    ("a limited preview can apply", REPLACE, "if preview.limited > 0 {\n            return Err(too_slow());\n        }", "", POOL, "a_too_slow_pattern"),
    ("the early stale check is gone and the commit check too", REPLACE, "if stamp != preview.snapshot {\n            return Err(Error::PlanStale);\n        }", "let _ = stamp;", CORE, "replace::tests::a_stale_preview",
     (COMMIT, "if state.stamp != snapshot {\n            return Err(Error::PlanStale);\n        }", "let _ = snapshot;")),
    ("cancel is ignored", REPLACE, "if self.cancel.is_cancelled() {\n            self.line.stop();\n            return Err(Error::Cancelled);\n        }\n        let Some(&position)", "let Some(&position)", CORE, "replace::tests::cancel_stops_apply"),
    ("a no-op chapter is rewritten", REPLACE, "if changes == 0 {\n            return Ok(Built::Unchanged);\n        }", "", CORE, "replace::tests::every_hit_excluded"),
    ("nav is replaced", SEARCH, ".filter(|row| !row.is_nav)", ".filter(|_row| true)", CORE, "replace::tests::offers_and_replace_skip"),
    ("a scoped preview widens", REPLACE, "chapters: unique_chapters(&reader, scope),", "chapters: unique_chapters(&reader, SearchScope::Whole),", CORE, "replace::tests::a_scoped"),
    ("shared files are dropped before the scope", REPLACE, "    scope\n        .rows(reader)\n        .filter(|row| row.href.as_ref().is_none_or(|href| seen.insert(href)))", "    reader\n        .rows()\n        .iter()\n        .filter(|row| row.href.as_ref().is_none_or(|href| seen.insert(href)))\n        .filter(|row| scope.rows(reader).any(|kept| kept.ix == row.ix))", CORE, "replace::tests::a_shared_file"),
    ("a fix review keeps the pane's scope", PANE, "start(WHOLE_BOOK);", "start();", E2E, "a_fix_review"),
    ("the offer follows a scope", FIX, "entry.offer.to }, WHOLE_BOOK);", "entry.offer.to }, { kind: 'from', here: 3000 });", E2E, "a_fix_review"),
    ("the refresh after apply moves to the reader's place", PANE, "      applyNote = '';\n      start(results.scope);", "      applyNote = '';\n      start();", E2E, "a_scoped_replace"),
    ("case variants are ignored", REPLACE, "Replacer::Literal { text, variants } if variants.is_empty() => Some(text),", "Replacer::Literal { text, .. } => Some(text),", CORE, "replace::tests::a_saved_word_fix"),
    ("an offer is not whole word", REPLACE, "whole_word: true,", "whole_word: false,", CORE, "replace::tests::a_saved_word_fix"),
    ("two changed words still offer", REPLACE, "let clear = is_one_word(from) && is_one_word(to);", "let clear = !from.is_empty() && !to.is_empty();", CORE, "replace::tests::an_offer_is_one_word"),
    ("a template is not checked", WORKER, "if let Some(Err(message)) = template.map(|template| E::check_template(compiled, template)) {", "if let Some(Err(message)) = template.map(|_| Ok::<(), String>(())) {", POOL, "a_template_that_names"),
    ("a huge expansion is not limited", WORKER, "self.oversized = true;\n            return false;", "self.oversized = false;", POOL, "a_huge_expansion"),
    ("a replacement row parses HTML", PANE, "slot.inserted.textContent = hit.replaced ?? '';", "slot.inserted.innerHTML = hit.replaced ?? '';", ATTACK, "replace_rows"),
    ("the pane ignores row exclusions", PANE, "exclusionsOf(choices), left.changes", "[], left.changes", E2E, "preview_exclude"),
    ("a stale refusal shows nothing", PANE, "if (end.reason === 'stale') start(results.scope);\n      showMessage(end.message);", "if (end.reason === 'stale') start(results.scope);", E2E, "a_stale_preview"),
    ("book-wide replace ignores Match case", PANE, "regex: regexMode, caseSensitive: matchCase(), replacement:", "regex: regexMode, caseSensitive: regexMode, replacement:", E2E, "book_wide_replace"),
    ("apply skips the unsaved prompt", MAIN, "return dispatch({ type: 'apply', run });", "return run();", E2E, "apply_over_unsaved"),
    ("the mounted chapter is not refreshed", MAIN, "await refreshMounted(wasEditing);", "", E2E, "a_mounted_chapter"),
    ("a discard leaves the text on screen", MACHINE, "{ fx: 'restoreChapter' }, ", "", E2E, "apply_over_unsaved"),
    ("Esc does not stop apply", MAIN, "if (applier.isBusy()) applier.requestStop();\n  else ", "", E2E, "esc_during_apply"),
    ("the offer ignores Esc-saved chapters", MAIN, "if (result?.wrote && chapter) fixOffer.offerFrom(result, chapter.ix);", "", E2E, "fix_everywhere"),
    ("the summary hides the undo note", MODEL, "parts.push('Applied as one atomic save.', UNDO_TEXT);", "parts.push('Applied as one atomic save.');", None, ""),
]


NODE = "tests/ui/replace-model.test.mjs"


def paths(mutation):
    _, file, _, _, target, test, *extra = mutation
    if target is None:
        tests = [NODE]
    elif target[1] is None:
        tests = mutants.lib_tests(target[0], test)
    else:
        tests = [mutants.test_file(*target)]
    return [file, *(where for where, _, _ in extra), *tests]


def run(command, env_dir=None):
    prefix = f"CARGO_TARGET_DIR={env_dir} " if env_dir else ""
    return mutants.run(prefix + command)


def check(target, test):
    if target is None:
        return run(f"node --test {NODE}")
    crate, binary = target
    if crate == "e2e":
        ok, output = run("cargo build --release -p jadeslip --features e2e && cargo build --release -p jadeslip-worker", "target/e2e")
        if not ok:
            return None, output
        return run(f"timeout -k 5 280 tests/e2e/headless.sh cargo test --release -p jadeslip-e2e --test {binary} -- --ignored --test-threads=1 {test}")
    where = "--lib" if binary is None else f"--test {binary}"
    return run(f"timeout -k 5 120 cargo test -p jadeslip-{crate} {where} {test}")


def main():
    caught = survived = broken = 0
    for mutation in mutants.chosen(MUTATIONS, paths):
        name, file, old, new, target, test, *extra = mutation
        originals = {}
        failed = False
        for where, before, after in [(file, old, new), *extra]:
            path = ROOT / where
            text = path.read_text()
            originals.setdefault(path, text)
            if before not in text:
                print(f"BROKEN   {name}: the code to change was not found: {before[:40]}", flush=True)
                broken += 1
                failed = True
                break
            path.write_text(text.replace(before, after, 1))
        try:
            if failed:
                continue
            ok, output = check(target, test)
        finally:
            for path, text in originals.items():
                path.write_text(text)
        reason = "does not build" if ok is None else mutants.not_run(output)
        if reason:
            print(f"BROKEN   {name}: {reason}\n{output[-600:]}", flush=True)
            broken += 1
        elif ok:
            print(f"SURVIVED {name}", flush=True)
            survived += 1
        else:
            print(f"CAUGHT   {name}", flush=True)
            caught += 1
    run("cargo build --release -p jadeslip --features e2e && cargo build --release -p jadeslip-worker", "target/e2e")
    print(f"caught {caught} survived {survived} broken {broken}", flush=True)
    sys.exit(0 if survived == 0 and broken == 0 else 1)


if __name__ == "__main__":
    main()
