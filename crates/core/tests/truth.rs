mod common;

use common::{books, fixture, truth};
use serde_json::Value;
use jadeslip_core::{ChapterIx, ChapterRow, DEFAULT_LIMITS, Error, Extracted, Reader};
use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};

fn open(book: &str) -> Result<Reader, Error> {
    Reader::open(File::open(fixture(book)).unwrap(), DEFAULT_LIMITS)
}

type ErrorClass = fn(&Error) -> bool;

enum Expected {
    RefusedAtOpen(ErrorClass),
    Loads,
    ChaptersFail(&'static [(u32, ErrorClass)]),
}

fn is_limit(error: &Error) -> bool {
    matches!(error, Error::Limit(_))
}

fn is_book(error: &Error) -> bool {
    matches!(error, Error::Book(_))
}

fn bomb_outcomes() -> Vec<(&'static str, Expected)> {
    use Expected::*;
    vec![
        ("bomb-book-total", RefusedAtOpen(is_limit)),
        ("bomb-deep-xml-opf", RefusedAtOpen(is_limit)),
        ("bomb-huge-entry-count", RefusedAtOpen(is_limit)),
        ("bomb-lying-extent", RefusedAtOpen(is_book)),
        ("bomb-overlap-nested", RefusedAtOpen(is_book)),
        ("bomb-overlap-shared", RefusedAtOpen(is_book)),
        ("bomb-traversal-absolute", RefusedAtOpen(is_book)),
        ("bomb-traversal-backslash", RefusedAtOpen(is_book)),
        ("bomb-traversal-container", RefusedAtOpen(is_book)),
        ("bomb-traversal-dotdot", RefusedAtOpen(is_book)),
        ("bomb-traversal-nul", RefusedAtOpen(is_book)),
        ("bomb-traversal-opf-href", RefusedAtOpen(is_book)),
        ("bomb-deep-xml-chapter", ChaptersFail(&[(1, is_limit)])),
        ("bomb-oversize-entry", ChaptersFail(&[(0, is_limit)])),
        (
            "bomb-lying-sizes",
            ChaptersFail(&[(1, is_book), (2, is_book), (3, is_book), (4, is_book)]),
        ),
        ("bomb-ratio", Loads),
    ]
}

#[test]
fn every_bomb_is_refused_readably_at_its_stage() {
    let outcomes = bomb_outcomes();
    let all_bombs: Vec<&str> = books()
        .into_iter()
        .filter(|book| book.starts_with("bomb-"))
        .collect();
    assert_eq!(
        outcomes.len(),
        all_bombs.len(),
        "the table covers every bomb"
    );
    for (book, expected) in outcomes {
        let stage = match expected {
            Expected::RefusedAtOpen(_) => "open",
            Expected::Loads => "loads",
            Expected::ChaptersFail(_) => "chapter",
        };
        assert_eq!(
            truth(book)["bomb"]["stage"].as_str(),
            Some(stage),
            "{book}: sidecar stage"
        );
        match (open(book), expected) {
            (Err(error), Expected::RefusedAtOpen(class)) => {
                assert!(class(&error), "{book}: wrong class {error:?}");
                assert!(error.user_message().len() > 10, "{book}: message too thin");
            }
            (Ok(reader), Expected::ChaptersFail(failing)) => {
                for row in reader.rows() {
                    match failing.iter().find(|(ix, _)| *ix == row.ix.0) {
                        Some((_, class)) => {
                            let error = match reader.visible_text(row.ix) {
                                Err(error) => error,
                                Ok(_) => panic!("{book} chapter {}: loaded", row.ix.0),
                            };
                            assert!(class(&error), "{book} chapter {}: {error:?}", row.ix.0);
                            assert!(error.user_message().len() > 10);
                        }
                        None => assert!(
                            reader.visible_text(row.ix).is_ok(),
                            "{book} control chapter {}",
                            row.ix.0
                        ),
                    }
                }
            }
            (Ok(reader), Expected::Loads) => {
                for row in reader.rows() {
                    assert!(reader.visible_text(row.ix).is_ok(), "{book}");
                }
            }
            (Err(error), _) => panic!("{book}: refused at open: {error:?}"),
            (Ok(_), Expected::RefusedAtOpen(_)) => panic!("{book}: opened"),
        }
    }
}

fn suffix_of(row: &ChapterRow) -> Option<&str> {
    row.href.as_ref().map(|name| {
        name.as_str()
            .strip_prefix("OEBPS/")
            .unwrap_or(name.as_str())
    })
}

fn check_rows(book: &str, reader: &Reader, truth: &Value) {
    let spine = truth["spine"].as_array().unwrap();
    assert_eq!(reader.rows().len(), spine.len(), "{book}: row count");
    assert_eq!(
        reader.summary().title,
        truth["title"].as_str().unwrap(),
        "{book}: title"
    );
    assert_eq!(
        u64::from(reader.summary().linear_chapters),
        truth["linear_chapters"].as_u64().unwrap(),
        "{book}: linear count"
    );
    let mut ordinal = 0;
    for (row, want) in reader.rows().iter().zip(spine) {
        assert_eq!(
            u64::from(row.ix.0),
            want["ix"].as_u64().unwrap(),
            "{book}: ix"
        );
        assert_eq!(
            suffix_of(row),
            want["href"].as_str(),
            "{book}: href at {}",
            row.ix.0
        );
        let kind = want["kind"].as_str().unwrap();
        assert_eq!(
            row.is_nav,
            kind == "nav",
            "{book}: nav flag at {}",
            row.ix.0
        );
        assert_eq!(
            row.rev.is_none(),
            matches!(kind, "missing-from-zip" | "unresolved-idref"),
            "{book}: rev at {}",
            row.ix.0
        );
        if want["linear"].as_bool().unwrap() && kind != "nav" {
            ordinal += 1;
            assert_eq!(
                row.linear_ordinal,
                Some(ordinal),
                "{book}: ordinal at {}",
                row.ix.0
            );
        } else {
            assert_eq!(row.linear_ordinal, None, "{book}: ordinal at {}", row.ix.0);
        }
    }
}

fn check_chapter(book: &str, reader: &Reader, truth: &Value, row_truth: &Value) {
    let ix = row_truth["ix"].as_u64().unwrap() as u32;
    let kind = row_truth["kind"].as_str().unwrap();
    let chapter = ChapterIx(ix);
    if matches!(kind, "missing-from-zip" | "unresolved-idref") {
        match reader.visible_text(chapter) {
            Err(Error::Book(message)) => assert!(
                message.contains(&(ix + 1).to_string()),
                "{book} {ix}: {message}"
            ),
            _ => panic!("{book} {ix}: a row with no file must give a readable error"),
        }
        return;
    }
    let text = reader
        .visible_text(chapter)
        .unwrap_or_else(|error| panic!("{book} {ix}: {error:?}"));
    let text = text.as_str();
    if matches!(kind, "image" | "empty") {
        assert!(
            text.trim().is_empty(),
            "{book} {ix}: {kind} chapter has text"
        );
    }
    if kind == "text"
        && let Some(title) = row_truth["title"].as_str()
    {
        assert!(text.contains(title), "{book} {ix}: heading missing");
    }
    if kind == "nav" {
        return;
    }
    let lowered = text.to_lowercase();
    let key = ix.to_string();
    for (term, facts) in truth["terms"].as_object().unwrap() {
        let want = facts["exact"][&key].as_u64().unwrap_or(0);
        assert_eq!(
            text.matches(term.as_str()).count() as u64,
            want,
            "{book} {ix}: exact {term:?}"
        );
        let want = facts["insensitive"][&key].as_u64().unwrap_or(0);
        assert_eq!(
            lowered.matches(&term.to_lowercase()).count() as u64,
            want,
            "{book} {ix}: folded {term:?}"
        );
    }
}

fn check_book(book: &str) -> usize {
    let truth = truth(book);
    let reader = open(book).unwrap_or_else(|error| panic!("{book}: {error:?}"));
    check_rows(book, &reader, &truth);
    let spine = truth["spine"].as_array().unwrap();
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(row) = spine.get(next.fetch_add(1, Ordering::Relaxed)) {
                    check_chapter(book, &reader, &truth, row);
                }
            });
        }
    });
    truth["terms"].as_object().unwrap().len()
}

#[test]
fn planted_words_match_truth_in_small_valid_and_malformed_books() {
    let mut planted = 0;
    for book in books() {
        if book.starts_with("bomb-") || book.starts_with("large-") {
            continue;
        }
        planted += usize::from(check_book(book) > 0);
    }
    assert!(planted >= 10, "only {planted} books planted words");
}

#[test]
fn planted_words_match_truth_in_the_large_h1_p_book() {
    assert!(check_book("large-h1-p") > 0);
}

#[test]
fn planted_words_match_truth_in_the_large_div_br_book() {
    assert!(check_book("large-h1-div-br") > 0);
}

#[test]
fn gbk_books_decode_their_han_headings_and_trail_byte_terms() {
    for book in ["gbk-xml-decl", "gbk-meta-charset"] {
        let reader = open(book).unwrap();
        let chapter = reader
            .rows()
            .iter()
            .find(|row| row.linear_ordinal == Some(2))
            .unwrap();
        let text = reader.visible_text(chapter.ix).unwrap();
        assert!(text.as_str().contains("第2章 山河"), "{book}: heading");
        let Extracted::Document(raw) = reader.extract(chapter.ix).unwrap() else {
            panic!("{book}: expected a document");
        };
        assert!(
            std::str::from_utf8(&raw).is_err(),
            "{book}: raw bytes are not UTF-8"
        );
    }
    let truth = truth("gbk-xml-decl");
    assert!(truth["terms"]["丂亐乥"]["exact_total"].as_u64().unwrap() > 0);
}

#[test]
fn malformed_rows_keep_their_index_and_only_the_broken_row_fails() {
    for (book, broken) in [("malformed-missing-chapter", 2), ("malformed-spine-ref", 2)] {
        let reader = open(book).unwrap();
        let rows = reader.rows();
        assert_eq!(rows[broken].ix, ChapterIx(broken as u32));
        assert!(reader.visible_text(rows[broken].ix).is_err(), "{book}");
        assert!(reader.visible_text(rows[broken + 1].ix).is_ok(), "{book}");
    }
}

#[test]
fn a_book_whose_container_points_nowhere_opens_through_its_only_opf() {
    let reader = open("malformed-container-ref").unwrap();
    assert_eq!(reader.summary().title, "Malformed Container Ref");
}

#[test]
fn a_raw_image_in_the_spine_extracts_as_an_image_and_has_no_text() {
    let reader = open("images-only").unwrap();
    let raw_plate = ChapterIx(4);
    match reader.extract(raw_plate).unwrap() {
        Extracted::Image { media_type, bytes } => {
            assert_eq!(media_type, "image/png");
            assert!(bytes.starts_with(b"\x89PNG"));
        }
        Extracted::Document(_) => panic!("a PNG was treated as a document"),
    }
    assert_eq!(reader.visible_text(raw_plate).unwrap().as_str(), "");
}

#[test]
fn fingerprint_depends_on_the_package_and_not_on_the_file_path() {
    let first = open("toc-none").unwrap();
    let again = open("toc-none").unwrap();
    let other = open("toc-nav-only").unwrap();
    assert_eq!(first.summary().fingerprint, again.summary().fingerprint);
    assert_ne!(first.summary().fingerprint, other.summary().fingerprint);
}

#[test]
fn cover_and_spine_size_come_from_the_directory() {
    let reader = open("large-h1-div-br").unwrap();
    let cover = reader.summary().cover.clone().expect("cover-image item");
    assert_eq!(cover.entry.as_str(), "OEBPS/Images/cover.png");
    assert!(reader.summary().inflated_spine_bytes > 100_000_000);
    assert!(open("toc-none").unwrap().summary().cover.is_none());
}
