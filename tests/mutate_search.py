import sys

import mutants
from mutants import ROOT

SEARCH = "crates/core/src/search.rs"
WORKER = "crates/core/src/worker.rs"
PANE = "app/ui/search.js"
MODEL = "app/ui/search-model.js"
TEXTMAP = "app/ui/textmap.js"
MAIN = "app/ui/main.js"
KEYS = "app/ui/keys.js"

CORE = ("core", "search")
POOL = ("worker", "pool")
E2E = ("e2e", "search")
ATTACK = ("e2e", "attack")

MUTATIONS = [
    ("results emitted as they finish, not in order", SEARCH,
     "schedule.finished.remove(&head)",
     "{ let _ = head; schedule.finished.pop_first().map(|(_, outcome)| outcome) }",
     CORE, "reading_order"),
    ("nav is searched", SEARCH, ".filter(|row| !row.is_nav)", ".filter(|_row| true)", CORE, "nav_document"),
    ("whole word ignored", SEARCH, "let pattern = if whole_word {", "let pattern = if false {", CORE, "whole_word"),
    ("case option ignored", SEARCH, ".case_insensitive(case == Case::Insensitive)", ".case_insensitive(false)", CORE, "whole_word"),
    ("cap not enforced", SEARCH, "self.capped = true;", "", CORE, "the_cap"),
    ("cancel waits for the threads", SEARCH,
     "if cancel.is_cancelled() {\n                return SearchEnd::Cancelled;\n            }\n            let (ready, early)",
     "if cancel.is_cancelled() {\n                self.run.stop();\n                while self.run.is_stopped() && lock(&self.run.schedule).finished.len() < 3 {\n                    std::thread::sleep(POLL);\n                }\n                return SearchEnd::Cancelled;\n            }\n            let (ready, early)",
     CORE, "cancelling_returns"),
    ("the worker is never killed at the deadline", WORKER,
     "if now >= deadline {\n            return (Outcome::Limited(hits), false);\n        }",
     "if now >= deadline {\n            deadline = now + Duration::from_secs(3600);\n        }",
     POOL, "a_stuck_chapter"),
    ("a dead worker fails the whole search", WORKER,
     "Ok(_) | Err(RecvTimeoutError::Disconnected) => {\n                return (Outcome::Limited(hits), false);\n            }",
     "Ok(_) | Err(RecvTimeoutError::Disconnected) => {\n                run.end_early(SearchEnd::Failed(START_FAILED.to_owned()));\n                return (Outcome::Limited(hits), false);\n            }",
     POOL, "an_outside_kill"),
    ("no memory cap", WORKER, "check(libc::setrlimit(libc::RLIMIT_AS, &memory))?;", "let _ = memory;", POOL, "allocation_cap"),
    ("both parent-death layers removed", WORKER,
     "check(libc::prctl(\n            libc::PR_SET_PDEATHSIG,\n            libc::SIGKILL as libc::c_ulong,\n        ))?;",
     "check(0)?;", POOL, "a_parent_killed", ("unsafe { libc::_exit(0) };", "loop { std::thread::park(); }")),
    ("regex hits lose their order inside the pool", WORKER, "run.put(position, outcome);", "run.put(position.saturating_sub(1), outcome);", POOL, "reading_order"),
    ("the worker keeps the parent's other files", WORKER, "check(libc::dup2(book_fd, BOOK_FD))?;", "check(libc::dup2(book_fd, BOOK_FD))?; check(libc::fcntl(0, libc::F_SETFD, 0))?; libc::open(c\"/dev/null\".as_ptr(), 0);", POOL, "a_worker_has_only"),
    ("a stale hit is not reported", PANE, "if (target && !sameRev(target.rev, chapter.rev)) {", "if (false) {", E2E, "saving_then_clicking"),
    ("the row renderer parses HTML", PANE, "slot.mark.textContent = hit.matched;", "slot.mark.innerHTML = hit.matched;", ATTACK, "search_results_render"),
    ("the label renderer parses HTML", PANE, "slot.where.textContent = whereOf(chapter);", "slot.where.innerHTML = whereOf(chapter);", ATTACK, "search_results_render"),
    ("a hit skips the unsaved prompt", MAIN, "return goTo(ix, null, target);", "return navigate(ix, null, target);", E2E, "unsaved_edits"),
    ("keys in the field act on the book", KEYS, "if (key.inField) return inField(key, look);", "", E2E, "keys_in_the_search_field_never_reach_the_book_while_reading"),
    ("closing the pane leaves the search running", PANE, "const wasRunning = running !== null || pendingStart !== null;\n    cancelRunning();", "const wasRunning = running !== null || pendingStart !== null;", E2E, "closing_the_pane"),
    ("the list renders every row", MODEL, "return { first, end: Math.max(first, end) };", "return { first: 0, end: rows };", E2E, "a_hundred_thousand"),
    ("highlight offsets count wrongly", TEXTMAP, "settle(before + text.length, (units) => ({ node, offset: units - before }));", "settle(before + text.length, (units) => ({ node, offset: Math.max(0, units - before - 1) }));", E2E, "every_planted_term"),
    ("the pane search ignores Match case", PANE, "searchStart(id, { text, regex: regexMode, caseSensitive: matchCase(),", "searchStart(id, { text, regex: regexMode, caseSensitive: regexMode,", E2E, "a_lowercase_regex"),
    ("Match case is not saved", "app/ui/prefs.js", "change({ matchCase: !prefs.matchCase });", "show({ matchCase: !prefs.matchCase });", E2E, "a_lowercase_regex"),
    ("the counter does not say chapters", MODEL, "searched`,", "`,", None, ""),
    ("rows of the old search stay after a new query", PANE, "results = emptyResults(text.length === 0 ? 'idle' : 'running', scope);", "results.phase = text.length === 0 ? 'idle' : 'running';", E2E, "a_new_query"),
    ("from is ignored", SEARCH, "(here.0 as usize).min(len)..len", "0..len", CORE, "from_here"),
    ("up to leaves out the current chapter", SEARCH, "0..(here.0 as usize + 1).min(len)", "0..(here.0 as usize).min(len)", CORE, "up_to_here"),
    ("the pane always sends the whole book", PANE, "wholeWord: false }, wireOf(scope));", "wholeWord: false }, WHOLE_BOOK);", E2E, "the_range_switch"),
    ("the status hides the scope", MODEL, "scanned: `${SCOPE_COPY[results.scope.kind].lead(results.scope.name)}", "scanned: `", None, ""),
    ("reopen keeps scoped rows", PANE, "if (open && results.scope.kind !== 'whole') dropScopedResults();", "", E2E, "reopening"),
    ("jumping to a place leaves the switch labels behind", MAIN, "  search.placeChanged();\n", "", E2E, "the_range_switch"),
    ("clicking the active side keeps the old place", PANE, "if (sameScope(scope, results.scope)) return;", "if (scope.kind === results.scope.kind) return;", E2E, "the_range_switch"),
    ("Rerun moves to the reader's place", PANE, "      start(results.scope);\n      recordOnEnd = true;", "      start();\n      recordOnEnd = true;", E2E, "the_range_switch"),
]


NODE = ["tests/ui/search-model.test.mjs", "tests/ui/keys.test.mjs"]


def paths(mutation):
    target = mutation[4]
    return [mutation[1], *(NODE if target is None else [mutants.test_file(*target)])]


def run(command, env_dir=None):
    prefix = f"CARGO_TARGET_DIR={env_dir} " if env_dir else ""
    return mutants.run(prefix + command)


def check(target, test):
    if target is None:
        return run(" ".join(["node --test", *NODE]))
    crate, binary = target
    if crate == "e2e":
        ok, output = run("cargo build --release -p jadeslip --features e2e && cargo build --release -p jadeslip-worker", "target/e2e")
        if not ok:
            return None, output
        return run(f"timeout -k 5 300 tests/e2e/headless.sh cargo test --release -p jadeslip-e2e --test {binary} -- --ignored --test-threads=1 {test}")
    return run(f"timeout -k 5 300 cargo test -p jadeslip-{crate} --test {binary} {test}")


def main():
    caught = survived = broken = 0
    for mutation in mutants.chosen(MUTATIONS, paths):
        name, file, old, new, target, test, *extra = mutation
        path = ROOT / file
        original = path.read_text()
        changed = original
        for before, after in [(old, new), *extra]:
            if before not in changed:
                print(f"BROKEN   {name}: the code to change was not found: {before[:40]}")
                broken += 1
                changed = None
                break
            changed = changed.replace(before, after, 1)
        if changed is None:
            continue
        path.write_text(changed)
        try:
            ok, output = check(target, test)
        finally:
            path.write_text(original)
        reason = "does not build" if ok is None else mutants.not_run(output)
        if reason:
            print(f"BROKEN   {name}: {reason}\n{output[-600:]}")
            broken += 1
        elif ok:
            print(f"SURVIVED {name}")
            survived += 1
        else:
            print(f"CAUGHT   {name}")
            caught += 1
    run("cargo build --release -p jadeslip --features e2e", "target/e2e")
    print(f"caught {caught} survived {survived} broken {broken}")
    sys.exit(0 if survived == 0 and broken == 0 else 1)


if __name__ == "__main__":
    main()
