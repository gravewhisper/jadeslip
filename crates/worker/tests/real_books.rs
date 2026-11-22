use serde_json::Value;
use jadeslip_core::{
    Book, CancelToken, Case, DEFAULT_LIMITS, Query, SearchEnd, SearchEvent, SearchOptions,
    SearchScope,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn real_books() -> Vec<PathBuf> {
    let mut books: Vec<PathBuf> = std::fs::read_dir(repo_root().join("fixtures/real"))
        .expect("fixtures/real exists")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "epub"))
        .collect();
    books.sort();
    books
}

fn label(path: &std::path::Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    stem.rsplit('-').next().unwrap_or_default().to_owned()
}

fn counted(book: &Book, query: &Query) -> (BTreeMap<u32, u64>, SearchEnd, u32) {
    let mut counts = BTreeMap::new();
    let mut unreadable = 0;
    let options = SearchOptions {
        max_hits: u32::MAX,
        worker_exe: Some(env!("CARGO_BIN_EXE_jadeslip-worker").into()),
        ..SearchOptions::default()
    };
    let end = book.search(
        query,
        SearchScope::Whole,
        &options,
        &CancelToken::default(),
        &mut |event| match event {
            SearchEvent::Hits { chapter, hits, .. } => {
                counts.insert(chapter.0, hits.len() as u64);
            }
            SearchEvent::ChapterUnreadable { .. } => unreadable += 1,
            _ => {}
        },
    );
    (counts, end, unreadable)
}

const CASES: &[(&str, &str)] = &[
    ("s:the", "literal-sensitive"),
    ("i:the", "literal-folded"),
    ("s:and", "literal-sensitive"),
    ("i:he", "literal-folded"),
    (r"i:(?<![\w\u0300-\u036f])the(?![\w\u0300-\u036f])", "word"),
    ("s:the(?=[a-z])", "regex"),
    ("s:(?<=[a-z])the", "regex"),
];

fn query_of(spec: &str, kind: &str) -> Query {
    let text = spec.split_once(':').unwrap().1.to_owned();
    match kind {
        "literal-sensitive" => Query::Literal {
            text,
            case: Case::Sensitive,
            whole_word: false,
        },
        "literal-folded" => Query::Literal {
            text,
            case: Case::Insensitive,
            whole_word: false,
        },
        "word" => Query::Literal {
            text: "the".to_owned(),
            case: Case::Insensitive,
            whole_word: true,
        },
        _ => Query::Regex {
            pattern: text,
            case: Case::Sensitive,
        },
    }
}

#[test]
#[ignore = "needs fixtures/real and python3, prints counts only"]
fn hit_counts_agree_with_a_separate_python_scan_on_every_real_book() {
    let script = repo_root().join("tests/scan_visible.py");
    let mut disagreements = 0;
    for path in real_books() {
        let specs: Vec<&str> = CASES.iter().map(|(spec, _)| *spec).collect();
        let output = Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(&path)
            .args(&specs)
            .output()
            .expect("python3 runs");
        assert!(
            output.status.success(),
            "the scan failed on {}",
            label(&path)
        );
        let scan: Value = serde_json::from_slice(&output.stdout).unwrap();
        let data = jadeslip_fixtures::scratch::scratch("real");
        let book = Book::open_by_fingerprint(&path, &data, DEFAULT_LIMITS).unwrap();
        for (spec, kind) in CASES {
            let python: BTreeMap<u32, u64> = scan["counts"][spec]
                .as_object()
                .unwrap()
                .iter()
                .map(|(ix, count)| (ix.parse().unwrap(), count.as_u64().unwrap()))
                .collect();
            let (ours, end, unreadable) = counted(&book, &query_of(spec, kind));
            let python_total: u64 = python.values().sum();
            let our_total: u64 = ours.values().sum();
            let mut differing = 0;
            let mut abs_diff = 0u64;
            for ix in python
                .keys()
                .chain(ours.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let (left, right) = (
                    ours.get(ix).copied().unwrap_or(0),
                    python.get(ix).copied().unwrap_or(0),
                );
                if left != right {
                    differing += 1;
                    abs_diff += left.abs_diff(right);
                }
            }
            println!(
                "{} case {} ({kind}): app {our_total} python {python_total} chapters differing {differing} hits differing {abs_diff} unreadable {unreadable} end_ok {}",
                label(&path),
                CASES.iter().position(|(other, _)| other == spec).unwrap(),
                end == SearchEnd::Done,
            );
            if differing > 0 {
                disagreements += 1;
            }
            assert_eq!(end, SearchEnd::Done);
        }
        let _ = std::fs::remove_dir_all(&data);
    }
    assert_eq!(disagreements, 0, "cases that differ from the python scan");
}
