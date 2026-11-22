use super::*;
use crate::CleanRule;
use crate::commit::seam::{Step, install};
use crate::common::{fixture, tiny_book, truth};
use crate::{BlockChange, BlockEdits, BlockIx, BookKey, DEFAULT_LIMITS, Extracted, RevertTarget};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

const KEY: BookKey = BookKey(0x9_1);
static NEXT: AtomicU32 = AtomicU32::new(0);

struct Place(PathBuf);

impl Place {
    fn new(label: &str) -> Place {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/replace-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("data")).unwrap();
        Place(dir)
    }

    fn book_path(&self) -> PathBuf {
        self.0.join("book.epub")
    }

    fn open(&self) -> Book {
        Book::open(&self.book_path(), &self.0.join("data"), KEY, DEFAULT_LIMITS).unwrap()
    }

    fn tiny(label: &str, chapters: &[&str]) -> (Place, Book) {
        let place = Place::new(label);
        tiny_book(&place.book_path(), chapters);
        let book = place.open();
        (place, book)
    }

    fn copy_of(label: &str, name: &str) -> (Place, Book) {
        let place = Place::new(label);
        fs::copy(fixture(name), place.book_path()).unwrap();
        let book = place.open();
        (place, book)
    }

    fn hash(&self) -> [u8; 32] {
        Sha256::digest(fs::read(self.book_path()).unwrap()).into()
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn options() -> SearchOptions {
    SearchOptions {
        threads: 3,
        ..SearchOptions::default()
    }
}

fn plain(text: &str, replacement: &str) -> ReplaceRequest {
    ReplaceRequest::new(
        Query::Literal {
            text: text.to_owned(),
            case: Case::Sensitive,
            whole_word: false,
        },
        replacement.to_owned(),
    )
}

fn preview(book: &Book, request: &ReplaceRequest) -> ReplacePreview {
    preview_in(book, request, SearchScope::Whole)
}

fn preview_in(book: &Book, request: &ReplaceRequest, scope: SearchScope) -> ReplacePreview {
    book.preview_replace(
        request,
        scope,
        5_000,
        &options(),
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap()
}

fn rows_of(book: &Book, request: &ReplaceRequest, max_rows: u32) -> Vec<(ChapterIx, Hit)> {
    let mut rows = Vec::new();
    book.preview_replace(
        request,
        SearchScope::Whole,
        max_rows,
        &options(),
        &CancelToken::default(),
        &mut |event| {
            if let SearchEvent::Hits { chapter, hits, .. } = event {
                rows.extend(hits.into_iter().map(|hit| (chapter, hit)));
            }
        },
    )
    .unwrap();
    rows
}

fn apply(book: &Book, preview: &ReplacePreview, exclude: &[ReplaceExclusion]) -> AppliedReplace {
    book.apply_replace(
        preview,
        exclude,
        &options(),
        &CancelToken::default(),
        &mut |_, _| {},
    )
    .unwrap()
}

fn chapter_bytes(book: &Book, ix: u32) -> Vec<u8> {
    match book.read().extract(ChapterIx(ix)).unwrap() {
        Extracted::Document(bytes) => bytes,
        Extracted::Image { .. } => panic!("an image chapter"),
    }
}

fn chapter_text(book: &Book, ix: u32) -> String {
    String::from_utf8(chapter_bytes(book, ix)).unwrap()
}

fn count_of(book: &Book, text: &str) -> usize {
    let mut found = 0;
    book.search(
        &Query::Literal {
            text: text.to_owned(),
            case: Case::Sensitive,
            whole_word: false,
        },
        SearchScope::Whole,
        &SearchOptions {
            max_hits: u32::MAX,
            ..options()
        },
        &CancelToken::default(),
        &mut |event| {
            if let SearchEvent::Hits { hits, .. } = event {
                found += hits.len();
            }
        },
    );
    found
}

#[test]
fn a_literal_replace_counts_and_changes_only_the_matched_bytes() {
    let (_place, book) = Place::tiny(
        "literal",
        &[
            "<p>one word and a word.</p><p>nothing here</p>",
            "<p>no match at all</p>",
            "<p class=\"word\">word</p>",
        ],
    );
    let before: Vec<String> = (0..3).map(|ix| chapter_text(&book, ix)).collect();
    let request = plain("word", "term");
    let shown = preview(&book, &request);
    assert_eq!(
        (shown.matches, shown.changes, shown.chapters, shown.skipped),
        (3, 3, 2, 0)
    );
    let applied = apply(&book, &shown, &[]);
    assert_eq!((applied.changes, applied.chapters), (3, 2));
    assert_eq!(applied.report.entries_rewritten, 2);
    assert_eq!(chapter_text(&book, 0), before[0].replace("word", "term"));
    assert_eq!(chapter_text(&book, 1), before[1]);
    assert_eq!(
        chapter_text(&book, 2),
        before[2].replace(">word<", ">term<")
    );
    assert_eq!(count_of(&book, "word"), 0);
    assert_eq!(count_of(&book, "term"), 3);
}

#[test]
fn rows_carry_the_replacement_and_their_ordinal_is_the_chapter_position() {
    let (_place, book) = Place::tiny("rows", &["<p>aa bb aa</p>", "<p>aa</p>"]);
    let rows = rows_of(&book, &plain("aa", "<cc>"), 2);
    assert_eq!(rows.len(), 2, "rows stop at max_rows");
    assert_eq!(rows[0].1.matched, "aa");
    assert_eq!(rows[0].1.replaced.as_deref(), Some("<cc>"));
    assert_eq!(rows[1].1.range.start, 6);
    assert!(rows.iter().all(|(chapter, _)| *chapter == ChapterIx(0)));
}

fn touched_chapters(preview: &ReplacePreview) -> Vec<u32> {
    preview
        .touched
        .iter()
        .map(|touched| touched.chapter.0)
        .collect()
}

#[test]
fn a_scoped_preview_touches_and_applies_only_its_side() {
    let chapters = ["<p>aa</p>", "<p>aa aa</p>", "<p>aa</p>", "<p>aa aa aa</p>"];
    let (_place, book) = Place::tiny("scoped", &chapters);
    let before = [chapter_bytes(&book, 0), chapter_bytes(&book, 1)];
    let shown = preview_in(
        &book,
        &plain("aa", "zz"),
        SearchScope::From { here: ChapterIx(2) },
    );
    assert_eq!(touched_chapters(&shown), [2, 3]);
    assert_eq!((shown.changes, shown.chapters), (4, 2));
    let applied = apply(&book, &shown, &[]);
    assert_eq!((applied.changes, applied.chapters), (4, 2));
    assert_eq!([chapter_bytes(&book, 0), chapter_bytes(&book, 1)], before);
    assert!(chapter_text(&book, 2).contains("<p>zz</p>"));
    assert!(chapter_text(&book, 3).contains("<p>zz zz zz</p>"));
}

#[test]
fn a_scoped_fix_request_stays_inside_its_scope() {
    let chapters = ["<p>recieved</p>", "<p>Recieved</p>", "<p>recieved</p>"];
    let (_place, book) = Place::tiny("scoped-fix", &chapters);
    let request = FixOffer {
        from: "recieved".to_owned(),
        to: "received".to_owned(),
    }
    .request();
    let after = chapter_bytes(&book, 2);
    let shown = preview_in(&book, &request, SearchScope::UpTo { here: ChapterIx(1) });
    assert_eq!(touched_chapters(&shown), [0, 1]);
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 0).contains("<p>received</p>"));
    assert!(chapter_text(&book, 1).contains("<p>Received</p>"));
    assert_eq!(chapter_bytes(&book, 2), after);
}

#[test]
fn a_replace_scoped_to_one_chapter_saves_only_that_chapter() {
    let (_place, book) = Place::tiny("only-here", &["<p>aa</p>", "<p>aa</p>", "<p>aa</p>"]);
    let request = plain("aa", "zz");
    let first = chapter_bytes(&book, 0);
    let last = chapter_bytes(&book, 2);
    let shown = preview_in(&book, &request, SearchScope::Only { here: ChapterIx(1) });
    assert_eq!(touched_chapters(&shown), [1]);
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 1).contains("<p>zz</p>"));
    assert_eq!(chapter_bytes(&book, 0), first);
    assert_eq!(chapter_bytes(&book, 2), last);
}

#[test]
fn a_shared_file_counts_when_its_later_row_is_in_scope() {
    use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
    let place = Place::new("shared-href");
    let item = |id: &str| {
        format!("<item id=\"{id}\" href=\"{id}.xhtml\" media-type=\"application/xhtml+xml\"/>")
    };
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">T</dc:title></metadata><manifest>{}{}</manifest><spine><itemref idref=\"a\"/><itemref idref=\"b\"/><itemref idref=\"a\"/></spine></package>",
        item("a"),
        item("b")
    );
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let page = |body: &str| {
        format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{body}</body></html>"
        )
    };
    let mut zip = ZipWriter::new(fs::File::create(place.book_path()).unwrap());
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"content.opf", opf.as_bytes(), DEFLATE).unwrap();
    zip.add(b"a.xhtml", page("<p>aa</p>").as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"b.xhtml", page("<p>aa</p>").as_bytes(), DEFLATE)
        .unwrap();
    zip.finish().unwrap();
    let book = place.open();
    let request = plain("aa", "zz");
    assert_eq!(touched_chapters(&preview(&book, &request)), [0, 1]);
    let shown = preview_in(&book, &request, SearchScope::From { here: ChapterIx(2) });
    assert_eq!(touched_chapters(&shown), [2]);
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 0).contains("<p>zz</p>"));
    assert!(chapter_text(&book, 1).contains("<p>aa</p>"));
}

#[test]
fn exclusions_by_hit_and_by_chapter_are_honoured_exactly() {
    let (_place, book) = Place::tiny("exclude", &["<p>aa aa aa</p>", "<p>aa</p>", "<p>aa</p>"]);
    let request = plain("aa", "zz");
    let shown = preview(&book, &request);
    let exclude = [
        ReplaceExclusion::Hit {
            chapter: ChapterIx(0),
            ordinal: 1,
        },
        ReplaceExclusion::Chapter {
            chapter: ChapterIx(2),
        },
    ];
    let untouched = chapter_bytes(&book, 2);
    let applied = apply(&book, &shown, &exclude);
    assert_eq!((applied.changes, applied.chapters), (3, 2));
    assert!(chapter_text(&book, 0).contains("<p>zz aa zz</p>"));
    assert!(chapter_text(&book, 1).contains("<p>zz</p>"));
    assert_eq!(chapter_bytes(&book, 2), untouched);
    assert_eq!(applied.report.entries_rewritten, 2);
}

#[test]
fn every_hit_excluded_in_every_chapter_writes_nothing() {
    let (place, book) = Place::tiny("all-out", &["<p>aa</p>"]);
    let shown = preview(&book, &plain("aa", "zz"));
    let hash = place.hash();
    let applied = apply(
        &book,
        &shown,
        &[ReplaceExclusion::Hit {
            chapter: ChapterIx(0),
            ordinal: 0,
        }],
    );
    assert_eq!(applied.changes, 0);
    assert_eq!(applied.report.entries_rewritten, 0);
    assert_eq!(place.hash(), hash);
}

#[test]
fn a_match_across_a_block_end_is_refused_and_counted_but_a_br_is_a_noted_change() {
    let (_place, book) = Place::tiny(
        "boundary",
        &[
            "<p>tail one</p><p>two head</p>",
            "<div>tail one<br/>two head</div>",
        ],
    );
    let query = Query::Literal {
        text: "one\ntwo".to_owned(),
        case: Case::Sensitive,
        whole_word: false,
    };
    let request = ReplaceRequest::new(query, "X".to_owned());
    let rows = rows_of(&book, &request, 10);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.crosses_block && rows[0].1.spans_tags);
    assert!(!rows[1].1.crosses_block && rows[1].1.spans_tags);
    let shown = preview(&book, &request);
    assert_eq!(
        (shown.matches, shown.changes, shown.skipped, shown.spanning),
        (2, 1, 1, 1)
    );
    let before = chapter_bytes(&book, 0);
    let applied = apply(&book, &shown, &[]);
    assert_eq!(applied.changes, 1);
    assert_eq!(chapter_bytes(&book, 0), before);
    assert!(chapter_text(&book, 1).contains("<div>tail X<br/> head</div>"));
}

#[test]
fn a_refused_match_stays_when_its_chapter_has_a_real_change() {
    let (_place, book) = Place::tiny("mixed", &["<p>one</p><p>two<br/>one<br/>two</p>"]);
    let query = Query::Literal {
        text: "one\ntwo".to_owned(),
        case: Case::Sensitive,
        whole_word: false,
    };
    let shown = preview(&book, &ReplaceRequest::new(query, "X".to_owned()));
    assert_eq!((shown.matches, shown.skipped, shown.changes), (2, 1, 1));
    assert_eq!(apply(&book, &shown, &[]).changes, 1);
    assert!(chapter_text(&book, 0).contains("<p>one</p><p>two<br/>X<br/></p>"));
}

#[test]
fn a_chapter_whose_only_matches_are_refused_is_not_rewritten() {
    let (place, book) = Place::tiny("only-refused", &["<p>aa</p><p>bb</p>"]);
    let query = Query::Literal {
        text: "aa\nbb".to_owned(),
        case: Case::Sensitive,
        whole_word: false,
    };
    let shown = preview(&book, &ReplaceRequest::new(query, "X".to_owned()));
    assert_eq!((shown.skipped, shown.changes, shown.chapters), (1, 0, 0));
    let hash = place.hash();
    let applied = apply(&book, &shown, &[]);
    assert_eq!(applied.report.entries_rewritten, 0);
    assert_eq!(place.hash(), hash);
    assert!(!matches!(
        book.backup_status().unwrap(),
        crate::BackupStatus::Valid { .. }
    ));
}

#[test]
fn a_match_across_inline_tags_follows_the_sketch_rule() {
    let (_place, book) = Place::tiny("inline", &["<p>foo <b>bar</b> baz</p>"]);
    let request = plain("o ba", "X");
    let rows = rows_of(&book, &request, 10);
    assert!(rows[0].1.spans_tags && !rows[0].1.crosses_block);
    let shown = preview(&book, &request);
    assert_eq!(shown.spanning, 1);
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 0).contains("<p>foX<b>r</b> baz</p>"));
}

#[test]
fn a_replacement_cannot_inject_markup() {
    for (replacement, stored) in [
        ("<b onclick=\"x()\">&", "&lt;b onclick=\"x()\"&gt;&amp;"),
        (
            "</p><script>x</script>",
            "&lt;/p&gt;&lt;script&gt;x&lt;/script&gt;",
        ),
        ("&amp;", "&amp;amp;"),
        ("]]>", "]]&gt;"),
        ("<!--", "&lt;!--"),
    ] {
        let (_place, book) = Place::tiny("escape", &["<p>aa zz</p>"]);
        let shown = preview(&book, &plain("aa", replacement));
        apply(&book, &shown, &[]);
        assert!(
            chapter_text(&book, 0).contains(&format!("<p>{stored} zz</p>")),
            "{replacement}"
        );
        assert_eq!(count_of(&book, replacement), 1, "{replacement}");
    }
}

#[test]
fn a_replacement_a_book_cannot_store_is_refused_before_any_work() {
    let (_place, book) = Place::tiny("illegal", &["<p>aa</p>"]);
    let attempt = book.preview_replace(
        &plain("aa", "\u{1}"),
        SearchScope::Whole,
        10,
        &options(),
        &CancelToken::default(),
        &mut |_| {},
    );
    assert!(matches!(attempt, Err(Error::Book(_))));
}

#[test]
fn a_replacement_equal_to_the_match_changes_nothing() {
    let (place, book) = Place::tiny("same", &["<p>aa  bb</p>"]);
    let shown = preview(&book, &plain("aa bb", "aa bb"));
    assert_eq!((shown.matches, shown.same, shown.changes), (1, 1, 0));
    let hash = place.hash();
    apply(&book, &shown, &[]);
    assert_eq!(place.hash(), hash);
}

#[test]
fn a_stale_preview_is_refused_after_any_save_and_nothing_is_written() {
    let (place, book) = Place::tiny("stale", &["<p>aa</p>", "<p>aa</p>"]);
    let shown = preview(&book, &plain("aa", "zz"));
    let chapter = book.read().rev_of(ChapterIx(1)).unwrap();
    let edits = BlockEdits {
        base: chapter,
        changes: vec![BlockChange::Replace {
            of: BlockIx(0),
            xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">aa changed</p>".to_owned(),
        }],
    };
    book.save_chapter(ChapterIx(1), &edits).unwrap();
    let hash = place.hash();
    let attempt = book.apply_replace(
        &shown,
        &[],
        &options(),
        &CancelToken::default(),
        &mut |_, _| {},
    );
    assert!(matches!(attempt, Err(Error::PlanStale)));
    assert_eq!(place.hash(), hash);
    let again = preview(&book, &plain("aa", "zz"));
    assert_eq!(apply(&book, &again, &[]).changes, 2);
}

#[test]
fn an_injected_failure_at_every_commit_step_leaves_the_file_unchanged() {
    use crate::commit::seam::LedgerWrite::{Created, Pending};
    let ledger = |stage| {
        [
            Step::LedgerTempWritten(stage),
            Step::LedgerSynced(stage),
            Step::LedgerRenamed(stage),
        ]
    };
    let mut steps = vec![
        Step::BackupCopied,
        Step::BackupSynced,
        Step::BackupRenamed,
        Step::TempCreated,
        Step::TempWriting,
        Step::TempWritten,
        Step::TempSynced,
        Step::TempVerified,
        Step::StampRechecked,
    ];
    steps.extend(ledger(Created));
    steps.extend(ledger(Pending));
    for failing in steps {
        let (place, book) = Place::tiny("inject", &["<p>aa</p>", "<p>bb aa</p>"]);
        let hash = place.hash();
        let shown = preview(&book, &plain("aa", "zz"));
        install(move |step| {
            if step == failing {
                Err(std::io::Error::other("injected"))
            } else {
                Ok(())
            }
        });
        let attempt = book.apply_replace(
            &shown,
            &[],
            &options(),
            &CancelToken::default(),
            &mut |_, _| {},
        );
        install(|_| Ok(()));
        assert!(attempt.is_err(), "{failing:?} did not fail the save");
        assert_eq!(place.hash(), hash, "{failing:?} changed the file");
        assert_eq!(count_of(&book, "aa"), 2, "{failing:?} moved the head");
        drop(book);
        let reopened = place.open();
        assert_eq!(place.hash(), hash, "{failing:?} recovery changed the file");
        assert_eq!(
            apply(&reopened, &preview(&reopened, &plain("aa", "zz")), &[]).changes,
            2
        );
    }
}

#[test]
fn cancel_stops_apply_with_nothing_written_and_no_temp_left() {
    let (place, book) = Place::copy_of("cancel", "images-mixed");
    let hash = place.hash();
    let request = plain("the", "THE");
    let shown = preview(&book, &request);
    assert!(shown.changes > 0);
    let cancel = CancelToken::default();
    let stopping = cancel.clone();
    let attempt = book.apply_replace(&shown, &[], &options(), &cancel, &mut |done, _| {
        if done == 0 {
            stopping.cancel();
        }
    });
    assert!(matches!(attempt, Err(Error::Cancelled)));
    assert_eq!(place.hash(), hash);
    let temps = fs::read_dir(&place.0)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(temps, 0);
}

#[test]
fn cancel_stops_a_preview() {
    let (_place, book) = Place::tiny("cancel-preview", &["<p>aa</p>"]);
    let cancel = CancelToken::default();
    cancel.cancel();
    let attempt = book.preview_replace(
        &plain("aa", "zz"),
        SearchScope::Whole,
        10,
        &options(),
        &cancel,
        &mut |_| {},
    );
    assert!(matches!(attempt, Err(Error::Cancelled)));
}

#[test]
fn whole_book_revert_returns_the_exact_original_after_apply() {
    let (place, book) = Place::copy_of("revert", "images-mixed");
    let hash = place.hash();
    let shown = preview(&book, &plain("the", "THE"));
    assert!(apply(&book, &shown, &[]).changes > 0);
    assert_ne!(place.hash(), hash);
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(place.hash(), hash);
}

#[test]
fn gbk_chapters_keep_their_encoding_and_an_unmappable_replacement_stays_storable() {
    for name in ["gbk-xml-decl", "gbk-meta-charset"] {
        let (_place, book) = Place::copy_of("gbk", name);
        let reader = book.read();
        let (ix, word, raw) = reader
            .rows()
            .iter()
            .filter(|row| !row.is_nav)
            .find_map(|row| {
                let word = reader
                    .visible_text(row.ix)
                    .ok()?
                    .as_str()
                    .chars()
                    .find(|ch| *ch > '\u{4E00}')?;
                match reader.extract(row.ix).ok()? {
                    Extracted::Document(raw) => Some((row.ix, word.to_string(), raw)),
                    Extracted::Image { .. } => None,
                }
            })
            .expect("a chapter with Chinese text");
        drop(reader);
        let shown = preview(&book, &plain(&word, "\u{5C71}\u{1F600}"));
        assert!(shown.changes > 0);
        let applied = apply(&book, &shown, &[]);
        assert!(applied.report.warnings.is_empty(), "{name}");
        let after = chapter_bytes(&book, ix.0);
        assert!(
            std::str::from_utf8(&after).is_err(),
            "{name}: not GBK any more"
        );
        let (decoded, _, had_errors) = encoding_rs::GBK.decode(&after);
        assert!(!had_errors, "{name}");
        assert!(decoded.contains("\u{5C71}&#128512;"), "{name}");
        assert!(after.len() > raw.len());
        assert_eq!(
            count_of(&book, "\u{5C71}\u{1F600}") as u64,
            shown.changes,
            "{name}"
        );
        assert_eq!(count_of(&book, &word), 0, "{name}");
    }
}

#[test]
fn a_hit_past_the_row_limit_still_counts_and_applies() {
    let (_place, book) = Place::tiny("rows-limit", &["<p>aa aa aa aa</p>"]);
    let request = plain("aa", "zz");
    let rows = rows_of(&book, &request, 1);
    assert_eq!(rows.len(), 1);
    let shown = book
        .preview_replace(
            &request,
            SearchScope::Whole,
            1,
            &options(),
            &CancelToken::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(shown.changes, 4);
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 0).contains("<p>zz zz zz zz</p>"));
}

fn offer(old: &str, new: &str) -> Option<(String, String)> {
    fix_offer(old, new).map(|offer| (offer.from, offer.to))
}

fn pair(from: &str, to: &str) -> Option<(String, String)> {
    Some((from.to_owned(), to.to_owned()))
}

#[test]
fn an_offer_is_one_word_changed_and_nothing_else() {
    assert_eq!(
        offer("he recieved it", "he received it"),
        pair("recieved", "received")
    );
    assert_eq!(offer("Jhon went", "John went"), pair("Jhon", "John"));
    assert_eq!(offer("at end teh", "at end the"), pair("teh", "the"));
    assert_eq!(offer("dont go", "don't go"), pair("dont", "don't"));
    assert_eq!(offer("colour", "color"), pair("colour", "color"));
    assert_eq!(
        offer("caf\u{e9}s here", "cafes here"),
        pair("caf\u{e9}s", "cafes")
    );
    assert_eq!(offer("'helo' he", "'hello' he"), pair("helo", "hello"));
    assert_eq!(offer("same", "same"), None);
    assert_eq!(offer("a new word", "a word"), None, "a deleted word");
    assert_eq!(offer("a word", "a new word"), None, "an added word");
    assert_eq!(
        offer("teh cat adn dog", "the cat and dog"),
        None,
        "two fixes"
    );
    assert_eq!(offer("alot here", "a lot here"), None, "one word to two");
    assert_eq!(offer("the end", "The end"), None, "only the case changed");
    assert_eq!(offer("a b", "x b"), None, "one letter");
    assert_eq!(offer("in 12 days", "in 13 days"), None, "a number");
    assert_eq!(offer("ab\ncd", "abcd"), None, "a join");
    assert_eq!(offer("aaaa", "aaaaa"), pair("aaaa", "aaaaa"));
    assert_eq!(offer("aa aa", "aa"), None, "overlap of prefix and suffix");
}

#[test]
fn case_rules_follow_the_shape_of_the_word() {
    let of = |from: &str, to: &str| -> Vec<(String, String)> {
        FixOffer {
            from: from.to_owned(),
            to: to.to_owned(),
        }
        .request()
        .variants
        .into_iter()
        .map(|variant| (variant.matched, variant.replacement))
        .collect()
    };
    let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    };
    assert_eq!(
        of("recieved", "received"),
        pairs(&[
            ("recieved", "received"),
            ("Recieved", "Received"),
            ("RECIEVED", "RECEIVED")
        ])
    );
    assert_eq!(
        of("Jhon", "John"),
        pairs(&[("Jhon", "John"), ("JHON", "JOHN")])
    );
    assert_eq!(of("JHON", "JOHN"), pairs(&[("JHON", "JOHN")]));
    assert_eq!(
        of("McDonlad", "McDonald"),
        pairs(&[("McDonlad", "McDonald")])
    );
}

#[test]
fn a_saved_word_fix_offers_the_others_whole_word_and_by_case_and_applies() {
    let (_place, book) = Place::tiny(
        "offer",
        &[
            "<p>He recieved it.</p><p>She recieved more. Unrecieved stays.</p>",
            "<p>Recieved. RECIEVED! ReCiEvEd recieved.</p>",
        ],
    );
    let rev = book.read().rev_of(ChapterIx(0)).unwrap();
    let saved = book
        .save_chapter(
            ChapterIx(0),
            &BlockEdits {
                base: rev,
                changes: vec![BlockChange::Replace {
                    of: BlockIx(0),
                    xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">He received it.</p>".to_owned(),
                }],
            },
        )
        .unwrap();
    assert_eq!(
        saved.fix_offers,
        vec![FixOffer {
            from: "recieved".to_owned(),
            to: "received".to_owned()
        }]
    );
    let request = saved.fix_offers[0].request();
    let shown = preview(&book, &request);
    assert_eq!((shown.changes, shown.chapters), (4, 2));
    apply(&book, &shown, &[]);
    assert!(chapter_text(&book, 0).contains("<p>She received more. Unrecieved stays.</p>"));
    assert!(chapter_text(&book, 1).contains("<p>Received. RECEIVED! ReCiEvEd received.</p>"));
}

#[test]
fn a_save_that_changes_two_words_makes_no_offer() {
    let (_place, book) = Place::tiny("no-offer", &["<p>teh cat adn dog</p>"]);
    let rev = book.read().rev_of(ChapterIx(0)).unwrap();
    let saved = book
        .save_chapter(
            ChapterIx(0),
            &BlockEdits {
                base: rev,
                changes: vec![BlockChange::Replace {
                    of: BlockIx(0),
                    xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">the cat and dog</p>".to_owned(),
                }],
            },
        )
        .unwrap();
    assert!(saved.fix_offers.is_empty());
}

#[test]
fn offers_and_replace_skip_the_nav_document() {
    let (_place, book) = Place::copy_of("nav", "images-mixed");
    let reader = book.read();
    let nav = reader
        .rows()
        .iter()
        .find(|row| row.is_nav)
        .expect("a nav")
        .ix;
    let nav_bytes = chapter_bytes(&book, nav.0);
    let word = reader
        .visible_text(nav)
        .unwrap()
        .as_str()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    drop(reader);
    let shown = preview(&book, &plain(&word, "ZZQQ"));
    apply(&book, &shown, &[]);
    assert!(shown.touched.iter().all(|touched| touched.chapter != nav));
    assert_eq!(chapter_bytes(&book, nav.0), nav_bytes);
}

#[test]
fn planted_truth_terms_fall_to_zero_and_the_new_term_rises_by_the_same() {
    for name in ["images-mixed", "gbk-xml-decl"] {
        let truth = crate::common::truth(name);
        let terms = truth["terms"].as_object().unwrap();
        let mut proven = 0;
        for (term, facts) in terms {
            let planted: u64 = facts["exact"].as_object().map_or(0, |chapters| {
                chapters.values().map(|n| n.as_u64().unwrap()).sum()
            });
            if planted == 0 {
                continue;
            }
            let (_place, book) = Place::copy_of("truth", name);
            let shown = preview(&book, &plain(term, "QXZQXZ"));
            assert_eq!(shown.matches, planted, "{name}");
            assert_eq!(shown.skipped, 0, "{name}");
            apply(&book, &shown, &[]);
            assert_eq!(count_of(&book, term), 0, "{name}");
            assert_eq!(count_of(&book, "QXZQXZ") as u64, planted, "{name}");
            proven += 1;
        }
        assert!(proven > 0, "{name}");
    }
}

const KILL_TERM: &str = "the";
const KILL_NEW: &str = "QXZKILL";

#[test]
#[ignore = "kill child, run by the kill test"]
fn kill_child() {
    let root = PathBuf::from(std::env::var_os("SS_KILL_ROOT").unwrap());
    let wanted = std::env::var("SS_KILL_STEP").unwrap();
    let place = Place(root.clone());
    let book = place.open();
    let shown = preview(&book, &plain(KILL_TERM, KILL_NEW));
    install(move |step| {
        if format!("{step:?}") == wanted {
            fs::write(root.join("sentinel"), b"at").unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        Ok(())
    });
    apply(&book, &shown, &[]);
    std::mem::forget(place);
}

#[test]
fn a_real_kill_at_every_step_of_apply_leaves_old_or_new_and_recovery_converges() {
    use crate::commit::seam::LedgerWrite::{Created, Head, Pending};
    let ledger = |stage| {
        [
            Step::LedgerTempWritten(stage),
            Step::LedgerSynced(stage),
            Step::LedgerRenamed(stage),
            Step::LedgerDirSynced(stage),
        ]
    };
    let mut steps = vec![
        Step::BackupCopied,
        Step::BackupSynced,
        Step::BackupRenamed,
        Step::BackupDirSynced,
    ];
    steps.extend(ledger(Created));
    steps.extend([
        Step::TempCreated,
        Step::TempWriting,
        Step::TempWritten,
        Step::TempSynced,
        Step::TempVerified,
    ]);
    steps.extend(ledger(Pending));
    steps.extend([Step::StampRechecked, Step::BookRenamed, Step::BookDirSynced]);
    steps.extend(ledger(Head));
    let rename_at = steps
        .iter()
        .position(|step| *step == Step::BookRenamed)
        .unwrap();
    let (reference, book) = Place::copy_of("kill-reference", "images-mixed");
    let old = fs::read(reference.book_path()).unwrap();
    apply(&book, &preview(&book, &plain(KILL_TERM, KILL_NEW)), &[]);
    let new = fs::read(reference.book_path()).unwrap();
    assert_ne!(old, new);
    for (ix, step) in steps.iter().enumerate() {
        let name = format!("{step:?}");
        let (place, opened) = Place::copy_of("kill", "images-mixed");
        drop(opened);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "replace::tests::kill_child",
                "--ignored",
                "--nocapture",
            ])
            .env("SS_KILL_ROOT", &place.0)
            .env("SS_KILL_STEP", &name)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !place.0.join("sentinel").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "{name}: child ended early"
            );
            assert!(
                Instant::now() < deadline,
                "{name}: child never reached the step"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let on_disk = fs::read(place.book_path()).unwrap();
        assert!(
            on_disk == old || on_disk == new,
            "{name}: neither old nor new"
        );
        assert_eq!(
            on_disk == new,
            ix >= rename_at,
            "{name}: wrong side of the rename"
        );
        let book = place.open();
        assert_eq!(
            book.recover().unwrap(),
            0,
            "{name}: recovery did not converge"
        );
        assert_eq!(
            fs::read(place.book_path()).unwrap(),
            on_disk,
            "{name}: recovery changed it"
        );
        let temps = fs::read_dir(&place.0)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(temps, 0, "{name}: a temp survives");
        let rerun = preview(&book, &plain(KILL_TERM, KILL_NEW));
        apply(&book, &rerun, &[]);
        assert_eq!(
            fs::read(place.book_path()).unwrap(),
            new,
            "{name}: re-run differs"
        );
        book.revert(RevertTarget::WholeBook).unwrap();
        assert_eq!(
            fs::read(place.book_path()).unwrap(),
            old,
            "{name}: revert differs"
        );
    }
}

fn cpu_ms() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let part = |time: libc::timeval| time.tv_sec as f64 * 1e3 + time.tv_usec as f64 / 1e3;
    part(usage.ru_utime) + part(usage.ru_stime)
}

#[test]
#[ignore = "timing report: all chapters of the large generated book"]
fn report_all_chapters_replace() {
    let (place, book) = Place::copy_of("all-chapters", "large-h1-p");
    let size_before = fs::metadata(place.book_path()).unwrap().len();
    let timed = SearchOptions::default();
    let request = plain("the", "thee");
    let found = book
        .preview_replace(
            &request,
            SearchScope::Whole,
            0,
            &timed,
            &CancelToken::default(),
            &mut |_| {},
        )
        .unwrap();
    let cpu_before = cpu_ms();
    let started = Instant::now();
    let applied = book
        .apply_replace(&found, &[], &timed, &CancelToken::default(), &mut |_, _| {})
        .unwrap();
    let total = started.elapsed();
    let cpu = cpu_ms() - cpu_before;
    println!(
        "all-chapters replace: {} chapters, {} changes, {} threads, apply {:.0} ms, cpu {cpu:.0} ms, sync {:.0} ms, hash {:.0} ms, commit write {:.0} ms, read {:.0} ms, verify {:.0} ms, file {size_before} to {} bytes",
        applied.chapters,
        applied.changes,
        timed.threads,
        total.as_secs_f64() * 1e3,
        applied.report.phases.sync_temp.as_secs_f64() * 1e3,
        applied.report.phases.hash.as_secs_f64() * 1e3,
        applied.report.phases.write.as_secs_f64() * 1e3,
        applied.report.phases.read.as_secs_f64() * 1e3,
        applied.report.phases.verify.as_secs_f64() * 1e3,
        fs::metadata(place.book_path()).unwrap().len(),
    );
    assert_eq!(applied.chapters, 6_000);
    if let Some(keep) = std::env::var_os("SS_KEEP_REPLACED") {
        fs::copy(place.book_path(), keep).unwrap();
    }
}

fn records(path: &Path) -> HashMap<String, Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let file = fs::File::open(path).unwrap();
    let zip = crate::zip::ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
    zip.entries()
        .iter()
        .map(|entry| {
            let mut bytes = vec![0; (entry.record.end - entry.record.start) as usize];
            file.read_exact_at(&mut bytes, entry.record.start).unwrap();
            (entry.name.as_str().to_owned(), bytes)
        })
        .collect()
}

const CLEAN_CHAPTERS: &[&str] = &[
    "<h1>Chapter 1: Dawn</h1><p>Chapter 1: Dawn</p><p>The sun rose.</p><p>TL: Greywind</p>",
    "<h1>Chapter 2: Noon</h1><div>Chapter 2: Noon<br/><br/>He ate.<br/><br/>Read more at novelsite.com<br/><br/></div>",
    "<p>Nothing to clean here.</p><p>“Support me on Patreon,” he joked.</p>",
    "<h1>Chapter 4</h1><p>Story.</p><p>Support us on Patreon!</p>",
];

fn clean_all() -> ReplaceRequest {
    ReplaceRequest::clean(vec![
        CleanRule::Watermark,
        CleanRule::Notes,
        CleanRule::Heading,
    ])
}

#[test]
fn a_clean_preview_counts_each_rule_and_lists_the_units() {
    let (_place, book) = Place::tiny("clean-count", CLEAN_CHAPTERS);
    let shown = preview(&book, &clean_all());
    assert_eq!((shown.matches, shown.changes, shown.chapters), (5, 5, 3));
    assert_eq!((shown.skipped, shown.spanning, shown.same), (0, 0, 0));
    let tally = |rule| *shown.rules.iter().find(|tally| tally.rule == rule).unwrap();
    assert_eq!(shown.rules.len(), 3);
    assert_eq!(shown.rules[0].rule, CleanRule::Watermark);
    assert_eq!(
        (
            tally(CleanRule::Watermark).matches,
            tally(CleanRule::Watermark).chapters
        ),
        (1, 1)
    );
    assert_eq!(
        (
            tally(CleanRule::Notes).matches,
            tally(CleanRule::Notes).chapters
        ),
        (2, 2)
    );
    assert_eq!(
        (
            tally(CleanRule::Heading).matches,
            tally(CleanRule::Heading).chapters
        ),
        (2, 2)
    );
    let rows = rows_of(&book, &clean_all(), 100);
    let row = |chapter: u32, ordinal: usize| {
        let hits: Vec<&Hit> = rows
            .iter()
            .filter(|(at, _)| *at == ChapterIx(chapter))
            .map(|(_, hit)| hit)
            .collect();
        (
            hits[ordinal].rule,
            hits[ordinal].matched.clone(),
            hits[ordinal].replaced.clone(),
        )
    };
    assert_eq!(
        row(1, 1),
        (
            Some(CleanRule::Watermark),
            "Read more at novelsite.com".to_owned(),
            Some(String::new())
        )
    );
    assert_eq!(row(0, 0).0, Some(CleanRule::Heading));
    assert!(
        shown
            .touched
            .iter()
            .all(|touched| touched.chapter != ChapterIx(2))
    );
    assert!(preview(&book, &plain("Dawn", "Dusk")).rules.is_empty());
}

#[test]
fn a_clean_applies_as_one_save_honours_exclusions_and_restores_exactly() {
    let (place, book) = Place::tiny("clean-apply", CLEAN_CHAPTERS);
    let before: Vec<Vec<u8>> = (0..4).map(|ix| chapter_bytes(&book, ix)).collect();
    let old_book = place.0.join("before.epub");
    fs::copy(place.book_path(), &old_book).unwrap();
    let shown = preview(&book, &clean_all());
    let applied = apply(
        &book,
        &shown,
        &[
            ReplaceExclusion::Hit {
                chapter: ChapterIx(0),
                ordinal: 1,
            },
            ReplaceExclusion::Rule {
                rule: CleanRule::Watermark,
            },
        ],
    );
    assert_eq!((applied.changes, applied.chapters), (3, 3));
    assert_eq!(
        chapter_text(&book, 0),
        String::from_utf8(before[0].clone())
            .unwrap()
            .replace("<p>Chapter 1: Dawn</p>", "")
    );
    assert_eq!(
        chapter_text(&book, 1),
        String::from_utf8(before[1].clone())
            .unwrap()
            .replace("Chapter 2: Noon<br/><br/>", "")
    );
    assert_eq!(
        chapter_text(&book, 3),
        String::from_utf8(before[3].clone())
            .unwrap()
            .replace("<p>Support us on Patreon!</p>", "")
    );
    let (old, new) = (records(&old_book), records(&place.book_path()));
    for name in ["c2.xhtml", "content.opf", "META-INF/container.xml"] {
        assert_eq!(old[name], new[name], "{name} kept its record");
    }
    let points = book.history().unwrap();
    assert_eq!(points.len(), 1, "one save, one history point");
    book.restore_point(points[0].id, RevertTarget::WholeBook)
        .unwrap();
    for (ix, bytes) in before.iter().enumerate() {
        assert_eq!(
            &chapter_bytes(&book, ix as u32),
            bytes,
            "chapter {ix} restored"
        );
    }
}

fn clean_with_titles() -> ReplaceRequest {
    ReplaceRequest::clean(vec![
        CleanRule::Watermark,
        CleanRule::Notes,
        CleanRule::Heading,
        CleanRule::Titles,
    ])
}

fn labels(book: &Book) -> Vec<Vec<String>> {
    book.read()
        .toc_docs()
        .unwrap()
        .into_iter()
        .flatten()
        .map(|(_, doc)| {
            doc.entries
                .iter()
                .map(|entry| entry.label.clone())
                .collect()
        })
        .collect()
}

#[test]
fn the_planted_junk_counts_match_the_truth_and_titles_reach_the_contents_in_one_save() {
    let truth = truth("planted-junk");
    let clean = &truth["clean"];
    let (place, book) = Place::copy_of("clean-titles", "planted-junk");
    let original = fs::read(place.book_path()).unwrap();
    let shown = preview(&book, &clean_with_titles());
    for (rule, key) in [
        (CleanRule::Watermark, "watermark"),
        (CleanRule::Notes, "notes"),
        (CleanRule::Heading, "heading"),
        (CleanRule::Titles, "titles"),
    ] {
        let tally = shown.rules.iter().find(|tally| tally.rule == rule).unwrap();
        assert_eq!(
            (tally.matches, u64::from(tally.chapters)),
            (
                clean["matches"][key].as_u64().unwrap(),
                clean["chapters"][key].as_u64().unwrap()
            ),
            "{key}"
        );
    }
    let fixes = clean["titles"].as_array().unwrap();
    let rows = rows_of(&book, &clean_with_titles(), u32::MAX);
    for fix in fixes {
        let chapter = ChapterIx(fix["chapter"].as_u64().unwrap() as u32);
        let row = rows
            .iter()
            .find(|(at, hit)| *at == chapter && hit.rule == Some(CleanRule::Titles))
            .map(|(_, hit)| (hit.matched.as_str(), hit.replaced.as_deref()))
            .unwrap();
        assert_eq!(row, (fix["from"].as_str().unwrap(), fix["to"].as_str()));
    }
    let before = labels(&book);
    assert_eq!(before.len(), 2, "a nav and an NCX");
    let skipped = fixes[0]["chapter"].as_u64().unwrap() as u32;
    let applied = apply(
        &book,
        &shown,
        &[ReplaceExclusion::Hit {
            chapter: ChapterIx(skipped),
            ordinal: 0,
        }],
    );
    let planted = clean["planted"].as_array().unwrap().len() as u64;
    assert_eq!(
        (applied.changes, applied.titles),
        (planted, fixes.len() as u64 - 1)
    );
    let after = labels(&book);
    for doc in &after {
        for fix in fixes {
            let ordinal = fix["chapter"].as_u64().unwrap() as usize;
            let wanted = if ordinal == skipped as usize {
                fix["from"].as_str().unwrap()
            } else {
                fix["to"].as_str().unwrap()
            };
            assert!(
                doc.iter().any(|label| label == wanted),
                "{wanted} is in the contents: {doc:?}"
            );
        }
    }
    for fix in fixes.iter().skip(1) {
        let ix = fix["chapter"].as_u64().unwrap() as u32;
        let text = chapter_text(&book, ix);
        let to = fix["to"].as_str().unwrap();
        assert!(text.contains(&format!("<title>{to}</title>")), "{text}");
        assert!(text.contains(&format!("<h1>{to}</h1>")), "{text}");
        if let Some(pulled) = fix["pulled"].as_str() {
            assert!(!text.contains(pulled), "{text}");
        }
    }
    let points = book.history().unwrap();
    assert_eq!(points.len(), 1, "one save, one history point");
    book.restore_point(points[0].id, RevertTarget::WholeBook)
        .unwrap();
    assert_eq!(labels(&book), before);
    assert_eq!(fs::read(place.book_path()).unwrap(), original);
}

#[test]
fn a_title_fix_that_changed_since_its_preview_saves_nothing() {
    let (place, book) = Place::copy_of("clean-titles-stale", "planted-junk");
    let mut shown = preview(&book, &clean_with_titles());
    let chapter = *shown.titles.keys().next().unwrap();
    shown.titles.get_mut(&chapter).unwrap().1 = "1 Something Else".to_owned();
    let hash = place.hash();
    let refused = book.apply_replace(
        &shown,
        &[],
        &options(),
        &CancelToken::default(),
        &mut |_, _| {},
    );
    assert!(matches!(refused, Err(Error::WouldBreak(_))), "{refused:?}");
    assert_eq!(place.hash(), hash);
}

#[test]
fn a_clean_that_changed_since_its_preview_saves_nothing() {
    let (place, book) = Place::tiny("clean-stale", CLEAN_CHAPTERS);
    let shown = preview(&book, &clean_all());
    let other = preview(&book, &plain("sun", "moon"));
    apply(&book, &other, &[]);
    let hash = place.hash();
    let refused = book.apply_replace(
        &shown,
        &[],
        &options(),
        &CancelToken::default(),
        &mut |_, _| {},
    );
    assert!(matches!(refused, Err(Error::PlanStale)));
    assert_eq!(place.hash(), hash);
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --lib clean_applies_to_every_real_book -- --ignored --nocapture"]
fn clean_applies_to_every_real_book() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut books: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "epub"))
        .collect();
    books.sort();
    assert!(!books.is_empty(), "fixtures/real has books");
    for path in books {
        let place = Place::new("clean-real");
        fs::write(place.book_path(), fs::read(&path).unwrap()).unwrap();
        let book = place.open();
        let started = Instant::now();
        let shown = preview(&book, &clean_all());
        let previewed = started.elapsed();
        let started = Instant::now();
        let applied = apply(&book, &shown, &[]);
        println!(
            "{}: {} removed in {} chapters, preview {} ms, apply {} ms",
            path.file_name().unwrap().to_string_lossy(),
            applied.changes,
            applied.chapters,
            previewed.as_millis(),
            started.elapsed().as_millis()
        );
        assert_eq!(applied.changes, shown.changes);
        for (chapter, hit) in rows_of(&book, &clean_all(), 20) {
            println!("  again: {} {:?} {}", chapter.0, hit.rule, hit.matched);
        }
    }
}
