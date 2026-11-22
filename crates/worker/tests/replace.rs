#[path = "../../core/tests/common/mod.rs"]
mod common;

use common::{scratch, tiny_book};
use jadeslip_core::{
    AppliedReplace, Book, CancelToken, Case, ChapterIx, DEFAULT_LIMITS, Error, Extracted, Query,
    ReplaceExclusion, ReplacePreview, ReplaceRequest, SearchEvent, SearchOptions, SearchScope,
};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const QUADRATIC: &str = "(?s)(?=.*QQQ)x";

fn options() -> SearchOptions {
    SearchOptions {
        threads: 3,
        worker_exe: Some(PathBuf::from(env!("CARGO_BIN_EXE_jadeslip-worker"))),
        ..SearchOptions::default()
    }
}

fn regex(pattern: &str, replacement: &str) -> ReplaceRequest {
    ReplaceRequest::new(
        Query::Regex {
            pattern: pattern.to_owned(),
            case: Case::Sensitive,
        },
        replacement.to_owned(),
    )
}

struct Open {
    book: Book,
    path: PathBuf,
}

fn open(label: &str, chapters: &[&str]) -> Open {
    let dir = scratch(&format!("replace-{label}"));
    let path = dir.join("book.epub");
    tiny_book(&path, chapters);
    Open {
        book: Book::open(
            &path,
            &dir.join("data"),
            jadeslip_core::BookKey(9),
            DEFAULT_LIMITS,
        )
        .unwrap(),
        path,
    }
}

fn preview(book: &Book, request: &ReplaceRequest) -> Result<ReplacePreview, Error> {
    book.preview_replace(
        request,
        SearchScope::Whole,
        100,
        &options(),
        &CancelToken::default(),
        &mut |_| {},
    )
}

fn apply(
    book: &Book,
    shown: &ReplacePreview,
    exclude: &[ReplaceExclusion],
) -> Result<AppliedReplace, Error> {
    book.apply_replace(
        shown,
        exclude,
        &options(),
        &CancelToken::default(),
        &mut |_, _| {},
    )
}

fn text(book: &Book, ix: u32) -> String {
    match book.read().extract(ChapterIx(ix)).unwrap() {
        Extracted::Document(bytes) => String::from_utf8(bytes).unwrap(),
        Extracted::Image { .. } => panic!("an image"),
    }
}

#[test]
fn numbered_and_named_captures_expand_and_rows_show_the_result() {
    let opened = open(
        "captures",
        &["<p>Smith, Anna and Jones, Bob</p>", "<p>Lee, Cy</p>"],
    );
    let request = regex(r"(?<last>\w+), (?<first>\w+)", "$first ${last}");
    let mut rows = Vec::new();
    let shown = opened
        .book
        .preview_replace(
            &request,
            SearchScope::Whole,
            100,
            &options(),
            &CancelToken::default(),
            &mut |event| {
                if let SearchEvent::Hits { hits, .. } = event {
                    rows.extend(hits);
                }
            },
        )
        .unwrap();
    assert_eq!((shown.changes, shown.chapters), (3, 2));
    assert_eq!(rows[0].replaced.as_deref(), Some("Anna Smith"));
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>Anna Smith and Bob Jones</p>"));
    assert!(text(&opened.book, 1).contains("<p>Cy Lee</p>"));
}

#[test]
fn numbered_captures_expand() {
    let opened = open("numbered", &["<p>Smith, Anna</p>"]);
    let shown = preview(&opened.book, &regex(r"(\w+), (\w+)", "$2 $1 $$")).unwrap();
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>Anna Smith $</p>"));
}

#[test]
fn lookaround_changes_only_the_matched_part() {
    let opened = open("look", &["<p>cat dog catnip</p>"]);
    let shown = preview(&opened.book, &regex(r"(?<=\s)dog(?=\s)|cat(?!nip)", "pet")).unwrap();
    assert_eq!(shown.changes, 2);
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>pet pet catnip</p>"));
}

#[test]
fn a_capture_that_holds_a_decoded_entity_is_escaped_on_the_way_back() {
    let opened = open("entity", &["<p>a &lt;b&gt; &amp; c</p>"]);
    let shown = preview(&opened.book, &regex(r"(<b>) (&)", "$2 $1")).unwrap();
    assert_eq!(shown.changes, 1);
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>a &amp; &lt;b&gt; c</p>"));
}

#[test]
fn a_template_that_names_a_missing_group_is_refused_with_a_message() {
    let opened = open("bad-template", &["<p>aa</p>"]);
    let failure = preview(&opened.book, &regex(r"(a)", "$9")).err().unwrap();
    assert!(failure.user_message().contains("group"), "{failure}");
    let failure = preview(&opened.book, &regex(r"(a)", "${nope}"))
        .err()
        .unwrap();
    assert!(failure.user_message().contains("group"), "{failure}");
}

#[test]
fn exclusions_count_every_match_including_refused_ones() {
    let opened = open("ordinals", &["<p>aa</p><p>aa aa aa</p>"]);
    let shown = preview(&opened.book, &regex(r"aa(\n)?", "zz")).unwrap();
    assert_eq!(shown.matches, 4);
    assert_eq!(shown.skipped, 1);
    let applied = apply(
        &opened.book,
        &shown,
        &[ReplaceExclusion::Hit {
            chapter: ChapterIx(0),
            ordinal: 2,
        }],
    )
    .unwrap();
    assert_eq!(applied.changes, 2);
    assert!(text(&opened.book, 0).contains("<p>aa</p><p>zz aa zz</p>"));
}

#[test]
fn a_too_slow_pattern_in_preview_refuses_apply_with_nothing_written() {
    let body = format!("<p>{}</p>", "x".repeat(400_000));
    let opened = open("slow-preview", &[&body]);
    let before = fs::read(&opened.path).unwrap();
    let shown = preview(&opened.book, &regex(QUADRATIC, "y")).unwrap();
    assert!(shown.limited > 0);
    assert!(matches!(
        apply(&opened.book, &shown, &[]),
        Err(Error::Limit(_))
    ));
    assert_eq!(fs::read(&opened.path).unwrap(), before);
}

#[test]
fn a_chapter_going_past_its_deadline_during_apply_aborts_the_whole_save() {
    let opened = open("slow-apply", &["<p>aa</p>", "<p>aa</p>", "<p>aa</p>"]);
    let before = fs::read(&opened.path).unwrap();
    let shown = preview(&opened.book, &regex("aa", "zz")).unwrap();
    let slow = SearchOptions {
        chapter_deadline: Duration::from_millis(10),
        delay: Some(std::sync::Arc::new(|chapter: ChapterIx| {
            if chapter.0 == 2 {
                Duration::from_millis(400)
            } else {
                Duration::ZERO
            }
        })),
        ..options()
    };
    let attempt =
        opened
            .book
            .apply_replace(&shown, &[], &slow, &CancelToken::default(), &mut |_, _| {});
    assert!(
        matches!(attempt, Err(Error::Limit(_))),
        "{:?}",
        attempt.err()
    );
    assert_eq!(fs::read(&opened.path).unwrap(), before);
    assert_eq!(apply(&opened.book, &shown, &[]).unwrap().changes, 3);
}

#[test]
fn no_worker_means_a_readable_refusal_and_nothing_written() {
    let opened = open("no-worker", &["<p>aa</p>"]);
    let shown = preview(&opened.book, &regex("aa", "zz")).unwrap();
    let before = fs::read(&opened.path).unwrap();
    let none = SearchOptions {
        worker_exe: None,
        ..options()
    };
    let attempt =
        opened
            .book
            .apply_replace(&shown, &[], &none, &CancelToken::default(), &mut |_, _| {});
    assert!(matches!(attempt, Err(Error::Book(_))));
    assert_eq!(fs::read(&opened.path).unwrap(), before);
}

#[test]
fn a_huge_expansion_is_limited_not_a_dead_worker_or_a_full_memory() {
    let body = format!("<p>{}</p>", "x".repeat(600_000));
    let opened = open("huge", &[&body]);
    let shown = preview(&opened.book, &regex("(x+)", "$1$1$1")).unwrap();
    assert_eq!(shown.limited, 1);
}

#[test]
fn many_large_expansions_keep_their_frames_under_the_limit() {
    let word = "y".repeat(40_000);
    let body = format!("<p>{}</p>", vec![word.as_str(); 300].join(" "));
    let opened = open("frames", &[&body]);
    let patient = SearchOptions {
        chapter_deadline: Duration::from_secs(60),
        book_deadline: Duration::from_secs(60),
        ..options()
    };
    let cancel = CancelToken::default();
    let shown = opened
        .book
        .preview_replace(
            &regex("(y+)", "${1}z"),
            SearchScope::Whole,
            1,
            &patient,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!((shown.limited, shown.changes), (0, 300));
    opened
        .book
        .apply_replace(&shown, &[], &patient, &cancel, &mut |_, _| {})
        .unwrap();
    assert_eq!(text(&opened.book, 0).matches('z').count(), 300);
}

#[test]
fn a_hostile_template_cannot_write_markup_or_lose_text() {
    let opened = open("hostile", &["<p>aa</p>"]);
    let shown = preview(&opened.book, &regex("(aa)", "<script>$1</script>&")).unwrap();
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>&lt;script&gt;aa&lt;/script&gt;&amp;</p>"));
}

#[test]
fn a_zero_width_match_is_found_and_replace_inserts_text_there() {
    let opened = open(
        "zero-width",
        &["<p>one</p><p>two</p>", "<p>cat dog</p>", "<p></p>"],
    );
    let starts = Query::Regex {
        pattern: "(?m)^".to_owned(),
        case: Case::Sensitive,
    };
    let mut found = Vec::new();
    opened.book.search(
        &starts,
        SearchScope::Whole,
        &options(),
        &CancelToken::default(),
        &mut |event| {
            if let SearchEvent::Hits { chapter, hits, .. } = event {
                found.extend(
                    hits.iter()
                        .map(|hit| (chapter.0, hit.range.start, hit.range.end)),
                );
            }
        },
    );
    assert_eq!(found, [(0, 0, 0), (0, 4, 4), (1, 0, 0)]);
    let shown = preview(&opened.book, &regex("(?m)^", "* ")).unwrap();
    assert_eq!((shown.matches, shown.changes, shown.chapters), (3, 3, 2));
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 0).contains("<p>* one</p><p>* two</p>"));
    assert!(text(&opened.book, 1).contains("<p>* cat dog</p>"));
    let shown = preview(&opened.book, &regex("(?=dog)", "big ")).unwrap();
    assert_eq!(shown.changes, 1);
    apply(&opened.book, &shown, &[]).unwrap();
    assert!(text(&opened.book, 1).contains("<p>* cat big dog</p>"));
}
