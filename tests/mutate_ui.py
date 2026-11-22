import sys

from mutants import ROOT, chosen, not_run, run, test_file

SESSION = "app/ui/edit-session.js"
MACHINE = "app/ui/edit-machine.js"
MAIN = "app/ui/main.js"
KEYS = "app/ui/keys.js"
SESSION_RS = "app/src/session.rs"
EDIT = "crates/core/src/edit.rs"

MUTATIONS = [
    ("observer ignores typing", SESSION, "characterData: true,", "characterData: false,", "typing_then_undo"),
    ("undo leaves blocks dirty", SESSION, "=== this.baseline.get(number)) this.dirtyBlocks.delete(number);", "=== this.baseline.get(number)) this.dirtyBlocks.size;", "typing_then_undo"),
    ("Ctrl+Z not bound", KEYS, "if (letter === 'z') return key.shift ? 'redo' : 'undo';", "", "typing_then_undo"),
    ("returned rev ignored", MAIN, "chapter = { ...chapter, rev: result.rev };", "", "the_second_save"),
    ("blocks not renumbered", SESSION, "element.setAttribute(MARK, String(at));", "", "enter_in_a_paragraph"),
    ("paste handler removed", SESSION, "paste: (event) => this.onPaste(event),", "", "paste_inserts"),
    ("paste takes the rich flavour", SESSION, "event.clipboardData?.getData('text/plain')", "event.clipboardData?.getData('text/html')", "paste_inserts"),
    ("Enter always splits", SESSION, "if (block && isLineBlock(block, this.root) && !caretInsideNestedShape(point.node, block)) {", "if (false) {", "enter_in_a_div"),
    ("Enter inserts one break in a two break book", SESSION, "const run = dominantBreakRun(block);", "const run = 1;", "enter_on_real"),
    ("close is never guarded", SESSION_RS, "        if !self.editing {", "        if true {", "closing_the_window"),
    ("a failure drops the user to Read", MAIN, "return dispatch({ type: 'failed', failure: asSaveFailure(failure) });", "return dispatch({ type: 'saved' });", "a_read_only"),
    ("split rewrites the whole block", EDIT, ".extend(trim_to_diff(old, &joined, span.start, urls));", ".push(SourceEdit { span, text: to_xhtml(&joined, urls) });", "enter_on_real"),
    ("scroll not kept on the toolbar", MAIN, "scroller.scrollTop += host.getBoundingClientRect().top - before;", "", "e_type_esc"),
    ("reducer starts a save during a save", MACHINE, "esc: (state) => (state.then.to === 'stay' ? retarget(state, THEN.read) : { state, effects: [] }),", "esc: () => startSave(THEN.read),", None),
    ("reducer loses dirty text on Esc", MACHINE, "esc: (state) => (state.dirty ? startSave(THEN.read) : { state: READ, effects: [{ fx: 'endEdit' }] }),", "esc: () => ({ state: READ, effects: [{ fx: 'endEdit' }] }),", None),
    ("reducer discards on failure", MACHINE, "state: editing(true, event.failure),", "state: READ,", None),
    ("arrow keys turn pages while editing", KEYS, "if (key.key === 'Enter' && !key.shift) return 'paragraph';", "if (key.key === 'Enter' && !key.shift) return 'paragraph';\n  if (key.key === 'ArrowRight') return 'next';", None),
]


def build():
    return run("CARGO_TARGET_DIR=target/e2e cargo build --release -p jadeslip --features e2e")


NODE = ["tests/ui/edit-machine.test.mjs", "tests/ui/keys.test.mjs"]


def paths(mutation):
    _, file, _, _, test = mutation
    return [file, *(NODE if test is None else [test_file("e2e", "edit")])]


def check(test):
    if test is None:
        return run(["timeout", "-k", "5", "120", "node", "--test", *NODE])
    ok, output = build()
    if not ok:
        return None, output
    return run(f"timeout -k 5 300 tests/e2e/headless.sh cargo test --release -p jadeslip-e2e --test edit -- --ignored --test-threads=1 {test}")


def main():
    survived = []
    tried = 0
    for name, file, old, new, test in chosen(MUTATIONS, paths):
        tried += 1
        path = ROOT / file
        original = path.read_text(encoding="utf-8")
        if original.count(old) != 1:
            print(f"BAD MUTATION {name}: {original.count(old)} matches", flush=True)
            survived.append(name)
            continue
        try:
            path.write_text(original.replace(old, new), encoding="utf-8")
            passed, output = check(test)
        finally:
            path.write_text(original, encoding="utf-8")
        reason = "does not build" if passed is None else not_run(output)
        if reason:
            print(f"broken           {name}: {reason}", flush=True)
            survived.append(name)
        elif passed:
            print(f"survived         {name}", flush=True)
            survived.append(name)
        else:
            print(f"caught           {name}", flush=True)
    ok, _ = build()
    if not ok:
        print("the restored tree does not build")
        sys.exit(2)
    print(f"mutations {tried} not caught {len(survived)}")
    sys.exit(1 if survived else 0)


if __name__ == "__main__":
    main()
