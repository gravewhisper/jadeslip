import sys

from mutants import ROOT, chosen, lib_tests, not_run, run

DRAFT = "crates/core/src/commit/draft.rs"
COMMIT = "crates/core/src/commit.rs"

MUTATIONS = [
    ("stale base accepted", DRAFT, "if self.read().rev_of(ix)? != edits.base {", "if false {"),
    ("book hash ignored", DRAFT, ".is_ok_and(|hash| parse_hex(&stored.book) == Some(hash));", ".is_ok_and(|_| true);"),
    ("chapter rev ignored", DRAFT, "same_file && self.read().rev_of(stored.chapter).ok() == Some(stored.edits.base)", "same_file"),
    ("unrecovered draft overwritten", DRAFT, "if !self.draft_claimed.load(Ordering::SeqCst) && path.exists() {", "if false {"),
    ("empty edits written", DRAFT, "if edits.changes.is_empty() {\n            remove(&path)?;", "if false {\n            remove(&path)?;"),
    ("written in place", DRAFT, "write_file(&path, &bytes)?;", "fs::write(&path, &bytes)?;"),
    ("cut-off temps kept", DRAFT, "if file.starts_with(&prefix) && file.ends_with(\".tmp\") {", "if false {"),
    ("recover does not claim", DRAFT, "self.draft_claimed.store(true, Ordering::SeqCst);\n        Ok((stored.chapter", "Ok((stored.chapter"),
    ("discard keeps the file", DRAFT, "remove(&self.draft_path())?;\n        self.draft_claimed.store(true, Ordering::SeqCst);\n        Ok(())", "self.draft_claimed.store(true, Ordering::SeqCst);\n        Ok(())"),
    ("save drops an unrecovered draft", DRAFT, "if self.draft_claimed.load(Ordering::SeqCst) {\n            let _ = remove", "if true {\n            let _ = remove"),
    ("save keeps the draft", COMMIT, "        self.drop_draft_locked();\n", ""),
    ("hash not refreshed by a save", COMMIT, "        state.hash = Some(hash);\n", ""),
    ("damaged draft read as absent", DRAFT, "Err(_) => return Found::Unreadable,\n    };\n    match serde_json::from_slice::<Stored<BlockEdits>>(&bytes) {\n        Ok(stored) if stored.version == DRAFT_VERSION => Found::Stored(stored),\n        _ => Found::Unreadable,", "Err(_) => return Found::Unreadable,\n    };\n    match serde_json::from_slice::<Stored<BlockEdits>>(&bytes) {\n        Ok(stored) if stored.version == DRAFT_VERSION => Found::Stored(stored),\n        _ => Found::Absent,"),
]


def paths(mutation):
    return [mutation[1], *lib_tests("core", "draft::")]


def run_tests():
    return run(["timeout", "-k", "5", "600", "cargo", "test", "-p", "jadeslip-core", "--lib", "draft::"])


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
