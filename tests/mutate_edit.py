import sys

from mutants import ROOT, chosen, lib_tests, not_run, run

EDIT = "crates/core/src/edit.rs"
CANON = "crates/core/src/canon.rs"
SANITIZE = "crates/core/src/sanitize.rs"
COMMIT = "crates/core/src/commit.rs"

MUTATIONS = [
    ("verify disabled", EDIT, "if verify(&written, &check) {", "if true || verify(&written, &check) {"),
    ("verify ignores atoms", EDIT, "if !atoms.iter().map(|atom| &atom.atom).eq(check.intended) {", "if false {"),
    ("trim rewrites the whole block", EDIT, "for hunk in diff_atoms(&old_atoms, new) {", "for hunk in [Hunk { old_start: 0, old_end: old_atoms.len(), new_start: 0, new_end: new.len() }] {"),
    ("trim keeps one hunk", EDIT, "let Some(trace) = shortest_edit(old_middle, new_middle) else {", "let Some(trace) = shortest_edit(old_middle, new_middle).filter(|_| false) else {"),
    ("trim suffix off by one", EDIT, "old_end: old.len() - tail,", "old_end: old.len() - tail.saturating_sub(1),"),
    ("removed runs join across gaps", EDIT, "if atom.span.start == run.end {", "if true {"),
    ("NBSP not folded", CANON, "if ch == '\\u{A0}' { ' ' } else { ch }", "ch"),
    ("URLs not inverted when written", CANON, "out.push_str(&urls.to_source_href(&url[..end])?);", "out.push_str(&url[..end]);"),
    ("URLs not inverted when read", SANITIZE, ".then(|| self.map.to_source_href(href))", ".then(|| None::<String>)"),
    ("session not carried", COMMIT, "reader.keep_session(head.session());", ""),
    ("lone break kept", EDIT, "let new = without_lone_break(new);", ""),
    ("duplicate ids kept", EDIT, "dedupe_new_ids(&mut parsed, &segments);", "let _ = dedupe_new_ids;"),
    ("BOM dropped", EDIT, "if *bom {", "if false && *bom {"),
    ("not representable not referenced", EDIT, "let (encoded, _, _) = codec.encode(&spliced);", "let encoded = std::borrow::Cow::Borrowed(spliced.as_bytes());"),
    ("delete keeps its indent", EDIT, "start: indent,\n                    end: span.end,", "start: span.start,\n                    end: span.end,"),
    ("split rewrites the whole block", EDIT, ".extend(trim_to_diff(old, &joined, span.start, urls));", ".push(SourceEdit { span, text: to_xhtml(&joined, urls) });"),
    ("no-op writes", EDIT, "if !parsed.writes() {", "if false {"),
]


def paths(mutation):
    return [mutation[1], *lib_tests("core", "edit::")]


def run_tests():
    return run(["timeout", "-k", "5", "600", "cargo", "test", "-p", "jadeslip-core", "--lib", "edit::"])


def main():
    survived = []
    picked = chosen(MUTATIONS, paths)
    for name, file, old, new in picked:
        path = ROOT / file
        original = path.read_text(encoding="utf-8")
        if original.count(old) != 1:
            print(f"BAD MUTATION {name}: {original.count(old)} matches")
            survived.append(name)
            continue
        try:
            path.write_text(original.replace(old, new), encoding="utf-8")
            passed, output = run_tests()
        finally:
            path.write_text(original, encoding="utf-8")
        ran = not_run(output) is None
        verdict = ("survived" if passed else "caught") if ran else "broken"
        print(f"{verdict:16} {name}")
        if passed or not ran:
            survived.append(name)
    print(f"mutations {len(picked)} not caught {len(survived)}")
    sys.exit(1 if survived else 0)


main()
