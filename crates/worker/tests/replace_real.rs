#[path = "../../core/tests/common/mod.rs"]
mod common;

use common::{fixture, scratch, tiny_book, truth};
use jadeslip_core::{
    Book, BookKey, CancelToken, Case, DEFAULT_LIMITS, Extracted, Query, Reader, ReplaceRequest,
    RevertTarget, SearchOptions, SearchScope,
};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

const TOKEN: &str = "QXZQXZ";
const TERM: &str = "the";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn real_books() -> Vec<PathBuf> {
    let mut books: Vec<PathBuf> = fs::read_dir(root().join("fixtures/real"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "epub"))
        .collect();
    books.sort();
    books
}

fn options() -> SearchOptions {
    SearchOptions {
        worker_exe: Some(PathBuf::from(env!("CARGO_BIN_EXE_jadeslip-worker"))),
        ..SearchOptions::default()
    }
}

fn literal(term: &str, replacement: &str) -> ReplaceRequest {
    ReplaceRequest::new(
        Query::Literal {
            text: term.to_owned(),
            case: Case::Sensitive,
            whole_word: false,
        },
        replacement.to_owned(),
    )
}

fn documents(path: &Path) -> Vec<(String, Vec<u8>)> {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .filter_map(|row| match reader.extract(row.ix) {
            Ok(Extracted::Document(bytes)) => Some((row.href.as_ref()?.as_str().to_owned(), bytes)),
            _ => None,
        })
        .collect()
}

fn run_tool(script: &str, args: &[&Path]) -> String {
    let done = Command::new("python3")
        .arg("-I")
        .arg(root().join(script))
        .args(args)
        .output()
        .unwrap();
    assert!(done.status.success(), "{script} failed");
    String::from_utf8(done.stdout).unwrap()
}

fn prove_book(source: &Path, label: &str) {
    let dir = scratch(&format!("real-{label}"));
    let copy = dir.join("book.epub");
    let original = fs::read(source).unwrap();
    fs::write(&copy, &original).unwrap();
    let before = documents(&copy);
    let book = Book::open(&copy, &dir.join("data"), BookKey(7), DEFAULT_LIMITS).unwrap();
    let cancel = CancelToken::default();
    let shown = book
        .preview_replace(
            &literal(TERM, TOKEN),
            SearchScope::Whole,
            10,
            &options(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(shown.limited, 0, "{label}");
    let touched: BTreeSet<u32> = shown
        .touched
        .iter()
        .filter(|row| row.changes > 0)
        .map(|row| row.chapter.0)
        .collect();
    let applied = book
        .apply_replace(&shown, &[], &options(), &cancel, &mut |_, _| {})
        .unwrap();
    assert_eq!(applied.changes, shown.changes, "{label}");
    let reader = book.read();
    let names: BTreeSet<String> = touched
        .iter()
        .filter_map(|ix| reader.rows()[*ix as usize].href.as_ref())
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(names.len() as u32, applied.chapters, "{label}");
    let normalised = applied.report.mimetype_normalised;
    assert_eq!(
        applied.report.entries_rewritten,
        applied.chapters + u32::from(normalised),
        "{label}"
    );

    let after = documents(&copy);
    assert_eq!(before.len(), after.len(), "{label}");
    let (mut exact, mut across_tags, mut unchanged, mut tokens) = (0u64, 0u64, 0u64, 0u64);
    for ((name, old), (same_name, new)) in before.iter().zip(&after) {
        assert_eq!(name, same_name, "{label}");
        if !names.contains(name) {
            assert_eq!(old, new, "{label}: an unmatched chapter changed");
            unchanged += 1;
            continue;
        }
        let old_text = String::from_utf8_lossy(old);
        let new_text = String::from_utf8_lossy(new);
        assert!(!old_text.contains(TOKEN), "{label}");
        tokens += new_text.matches(TOKEN).count() as u64;
        if new_text.replace(TOKEN, TERM) == old_text {
            exact += 1;
        } else {
            across_tags += 1;
        }
    }
    assert_eq!(
        tokens, applied.changes,
        "{label}: every change is in the source"
    );
    assert_eq!(
        across_tags > 0,
        shown.spanning > 0,
        "{label}: only a tag span may differ"
    );
    assert!(!book.read().rows().is_empty(), "{label}");

    let changed_list = dir.join("changed.txt");
    let mut listed: Vec<String> = names.iter().cloned().collect();
    if normalised {
        listed.push("mimetype".to_owned());
    }
    fs::write(&changed_list, listed.join("\n")).unwrap();
    let old_copy = dir.join("old.epub");
    fs::write(&old_copy, &original).unwrap();
    let records = run_tool(
        "crates/core/tests/zip_records.py",
        &[&old_copy, &copy, &changed_list],
    );
    assert!(
        records.contains("differ_unexpected=0"),
        "{label}: {records}"
    );
    assert!(records.contains("removed=0"), "{label}: {records}");
    assert!(records.contains("crc_failed=0"), "{label}: {records}");
    assert!(
        records.contains("mimetype_first_stored=True"),
        "{label}: {records}"
    );
    let parsed = run_tool("tests/check_replaced.py", &[&old_copy, &copy]);
    assert!(parsed.contains("broke=0"), "{label}: {parsed}");

    let meta = Command::new("ebook-meta").arg(&copy).output().unwrap();
    assert!(
        meta.status.success(),
        "{label}: ebook-meta refused the saved book"
    );

    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(
        fs::read(&copy).unwrap(),
        original,
        "{label}: revert is not exact"
    );
    println!(
        "{label}: changes={} chapters={} skipped={} spanning={} exact_chapters={exact} span_chapters={across_tags} unchanged_chapters={unchanged} {} {}",
        applied.changes,
        applied.chapters,
        shown.skipped,
        shown.spanning,
        records.trim(),
        parsed.trim()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real books, run by hand"]
fn only_matched_bytes_change_on_every_real_book_copy_and_revert_is_exact() {
    for (ix, source) in real_books().iter().enumerate() {
        prove_book(source, &format!("real{ix}"));
    }
}

#[test]
#[ignore = "large fixtures, run by hand"]
fn only_matched_bytes_change_on_the_large_books() {
    prove_book(&fixture("large-h1-p"), "large-h1-p");
    prove_book(&fixture("large-h1-div-br"), "large-h1-div-br");
}

#[test]
#[ignore = "large fixtures, run by hand"]
fn every_planted_term_falls_to_zero_and_the_new_one_rises_by_the_same_on_the_large_book() {
    let name = "large-h1-p";
    let facts = truth(name);
    let terms = facts["terms"].as_object().unwrap();
    let dir = scratch("truth-large");
    let copy = dir.join("book.epub");
    fs::copy(fixture(name), &copy).unwrap();
    let book = Book::open(&copy, &dir.join("data"), BookKey(8), DEFAULT_LIMITS).unwrap();
    let cancel = CancelToken::default();
    let mut proven = 0;
    for (ordinal, (term, fact)) in terms.iter().enumerate() {
        let planted: u64 = fact["exact"]
            .as_object()
            .unwrap()
            .values()
            .map(|count| count.as_u64().unwrap())
            .sum();
        if planted == 0 {
            continue;
        }
        let token = format!("QXZ{ordinal}QXZ");
        let shown = book
            .preview_replace(
                &literal(term, &token),
                SearchScope::Whole,
                1,
                &options(),
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(shown.matches, planted, "term {ordinal}");
        let applied = book
            .apply_replace(&shown, &[], &options(), &cancel, &mut |_, _| {})
            .unwrap();
        assert_eq!(applied.changes + shown.skipped, planted, "term {ordinal}");
        let after = book
            .preview_replace(
                &literal(term, &token),
                SearchScope::Whole,
                1,
                &options(),
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(after.matches, shown.skipped, "term {ordinal} did not fall");
        let risen = book
            .preview_replace(
                &literal(&token, "x"),
                SearchScope::Whole,
                1,
                &options(),
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            risen.matches, applied.changes,
            "term {ordinal} did not rise"
        );
        proven += 1;
    }
    assert!(proven > 10, "only {proven} terms proved");
    let _ = fs::remove_dir_all(&dir);
}

fn peak_rss_mib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let kib: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        .unwrap();
    kib / 1024
}

#[test]
#[ignore = "slow: run with --release and alone, builds a book of huge all-entity chapters"]
fn huge_all_entity_chapters_stay_inside_the_byte_budget_through_apply() {
    let dir = scratch("entities-replace");
    let path = dir.join("book.epub");
    let chapter = format!("<p>{}needle</p>", "&amp;".repeat(12 << 20));
    let chapters: Vec<&str> = std::iter::repeat_n(chapter.as_str(), 8).collect();
    tiny_book(&path, &chapters);
    drop(chapter);
    let book = Book::open(&path, &dir.join("data"), BookKey(9), DEFAULT_LIMITS).unwrap();
    let cancel = CancelToken::default();
    let shown = book
        .preview_replace(
            &literal("needle", "pin"),
            SearchScope::Whole,
            10,
            &options(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(shown.changes, 8);
    let applied = book
        .apply_replace(&shown, &[], &options(), &cancel, &mut |_, _| {})
        .unwrap();
    assert_eq!(applied.changes, 8);
    let high_water = book.search_high_water();
    println!(
        "budget high water {} MiB, parent peak {} MiB",
        high_water >> 20,
        peak_rss_mib()
    );
    assert!(high_water <= jadeslip_core::CHAPTER_BYTE_BUDGET);
    assert!(peak_rss_mib() < 2048, "{} MiB", peak_rss_mib());
    let too_many = book
        .preview_replace(
            &literal("&", "x"),
            SearchScope::Whole,
            10,
            &options(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        too_many.limited, 8,
        "a chapter past the match cap is limited"
    );
    let _ = fs::remove_dir_all(&dir);
}
