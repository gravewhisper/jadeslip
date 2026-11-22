mod common;

use common::{books, fixture, scratch, tiny_book, truth};
use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, CancelToken, Case, ChapterIx, ChapterRev,
    DEFAULT_LIMITS, Hit, Query, Reader, SearchEnd, SearchEvent, SearchOptions, SearchScope,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_micros(16_700);

#[derive(Default)]
struct Seen {
    order: Vec<u32>,
    revs: BTreeMap<u32, ChapterRev>,
    hits: BTreeMap<u32, Vec<Hit>>,
    limited: Vec<u32>,
    unreadable: Vec<(u32, String)>,
    capped: bool,
    last_progress: Option<(u32, u32)>,
}

impl Seen {
    fn take(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::Hits { chapter, rev, hits } => {
                self.order.push(chapter.0);
                self.revs.insert(chapter.0, rev);
                self.hits.insert(chapter.0, hits);
            }
            SearchEvent::Progress { done, total } => {
                if let Some((before, _)) = self.last_progress {
                    assert!(done >= before, "progress went back");
                }
                self.last_progress = Some((done, total));
            }
            SearchEvent::ChapterLimited { chapter } => self.limited.push(chapter.0),
            SearchEvent::ChapterUnreadable { chapter, message } => {
                self.unreadable.push((chapter.0, message))
            }
            SearchEvent::CapReached => self.capped = true,
        }
    }

    fn total(&self) -> usize {
        self.hits.values().map(Vec::len).sum()
    }

    fn counts(&self) -> BTreeMap<u32, u64> {
        self.hits
            .iter()
            .map(|(ix, hits)| (*ix, hits.len() as u64))
            .collect()
    }
}

fn open_book(path: &Path, name: &str) -> Book {
    Book::open_by_fingerprint(path, &scratch(name), DEFAULT_LIMITS).expect("book opens")
}

fn literal(text: &str, case: Case, whole_word: bool) -> Query {
    Query::Literal {
        text: text.to_owned(),
        case,
        whole_word,
    }
}

fn run(book: &Book, query: &Query, options: &SearchOptions) -> (Seen, SearchEnd) {
    run_in(book, query, SearchScope::Whole, options)
}

fn run_in(
    book: &Book,
    query: &Query,
    scope: SearchScope,
    options: &SearchOptions,
) -> (Seen, SearchEnd) {
    let mut seen = Seen::default();
    let end = book.search(
        query,
        scope,
        options,
        &CancelToken::default(),
        &mut |event| seen.take(event),
    );
    (seen, end)
}

fn plenty() -> SearchOptions {
    SearchOptions {
        max_hits: u32::MAX,
        ..SearchOptions::default()
    }
}

fn truth_counts(facts: &serde_json::Value, key: &str) -> BTreeMap<u32, u64> {
    facts[key]
        .as_object()
        .unwrap()
        .iter()
        .map(|(ix, count)| (ix.parse().unwrap(), count.as_u64().unwrap()))
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn check_truth(name: &str) -> usize {
    let truth = truth(name);
    let terms = truth["terms"].as_object().unwrap();
    if terms.is_empty() {
        return 0;
    }
    let Ok(book) = Book::open_by_fingerprint(&fixture(name), &scratch(name), DEFAULT_LIMITS) else {
        return 0;
    };
    for (term, facts) in terms {
        let (exact, end) = run(&book, &literal(term, Case::Sensitive, false), &plenty());
        assert_eq!(end, SearchEnd::Done, "{name} {term}");
        assert_eq!(
            exact.counts(),
            truth_counts(facts, "exact"),
            "{name} exact {term}"
        );
        let (folded, _) = run(&book, &literal(term, Case::Insensitive, false), &plenty());
        assert_eq!(
            folded.counts(),
            truth_counts(facts, "insensitive"),
            "{name} folded {term}"
        );
        assert!(folded.order.windows(2).all(|pair| pair[0] < pair[1]));
    }
    terms.len()
}

#[test]
fn literal_counts_match_planted_truth_in_every_small_book() {
    let mut checked = 0;
    for name in books() {
        if !name.starts_with("large") {
            checked += check_truth(name);
        }
    }
    assert!(checked > 100, "only {checked} terms checked");
}

#[test]
fn literal_counts_match_planted_truth_in_the_large_books() {
    assert!(check_truth("large-h1-p") > 10);
    assert!(check_truth("large-h1-div-br") > 10);
}

#[test]
fn the_nav_document_is_never_searched() {
    let book = open_book(&fixture("images-mixed"), "nav");
    let reader = book.read();
    let nav = reader.rows().iter().find(|row| row.is_nav);
    let nav = nav.expect("this book keeps its nav in the spine");
    let text = reader.visible_text(nav.ix).unwrap();
    let word = text
        .as_str()
        .split_whitespace()
        .next()
        .expect("nav has text");
    let (seen, _) = run(&book, &literal(word, Case::Sensitive, false), &plenty());
    assert!(!seen.hits.contains_key(&nav.ix.0));
    let (done, total) = seen.last_progress.unwrap();
    assert_eq!(done, total);
    assert_eq!(total as usize, reader.rows().len() - 1);
}

fn five_chapters(name: &str) -> Book {
    let path = scratch(name).join("book.epub");
    tiny_book(
        &path,
        &[
            "<p>mark zero</p>",
            "<p>mark one</p>",
            "<p>mark two</p>",
            "<p>mark three</p>",
            "<p>mark four</p>",
        ],
    );
    open_book(&path, &format!("{name}-data"))
}

#[test]
fn up_to_here_includes_the_current_chapter_and_counts_only_those() {
    let book = five_chapters("up-to");
    let scope = SearchScope::UpTo { here: ChapterIx(2) };
    let (seen, end) = run_in(
        &book,
        &literal("mark", Case::Sensitive, false),
        scope,
        &plenty(),
    );
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.order, [0, 1, 2]);
    assert_eq!(seen.last_progress, Some((3, 3)));
}

#[test]
fn from_here_searches_the_current_chapter_through_the_end() {
    let book = five_chapters("from");
    let scope = SearchScope::From { here: ChapterIx(2) };
    let (seen, end) = run_in(
        &book,
        &literal("mark", Case::Sensitive, false),
        scope,
        &plenty(),
    );
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.order, [2, 3, 4]);
    assert_eq!(seen.last_progress, Some((3, 3)));
}

#[test]
fn only_here_searches_just_the_current_chapter() {
    let book = five_chapters("only");
    let scope = SearchScope::Only { here: ChapterIx(2) };
    let (seen, end) = run_in(
        &book,
        &literal("mark", Case::Sensitive, false),
        scope,
        &plenty(),
    );
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.order, [2]);
    assert_eq!(seen.last_progress, Some((1, 1)));
}

#[test]
fn a_scope_never_searches_the_nav_document() {
    let book = open_book(&fixture("images-mixed"), "scoped-nav");
    let reader = book.read();
    let nav = reader.rows().iter().find(|row| row.is_nav);
    let nav = nav.expect("this book keeps its nav in the spine");
    let text = reader.visible_text(nav.ix).unwrap();
    let word = text
        .as_str()
        .split_whitespace()
        .next()
        .expect("nav has text");
    let query = literal(word, Case::Sensitive, false);
    let len = reader.rows().len() as u32;
    let sides = [
        (SearchScope::UpTo { here: nav.ix }, nav.ix.0),
        (SearchScope::From { here: nav.ix }, len - nav.ix.0 - 1),
        (SearchScope::Only { here: nav.ix }, 0),
    ];
    for (scope, others) in sides {
        let (seen, end) = run_in(&book, &query, scope, &plenty());
        assert_eq!(end, SearchEnd::Done, "{scope:?}");
        assert!(!seen.hits.contains_key(&nav.ix.0), "{scope:?}");
        assert_eq!(
            seen.last_progress.unwrap_or((0, 0)),
            (others, others),
            "{scope:?}"
        );
    }
}

#[test]
fn a_scope_past_the_end_is_clamped() {
    let book = five_chapters("past-end");
    let query = literal("mark", Case::Sensitive, false);
    let (whole, _) = run(&book, &query, &plenty());
    let up_to = SearchScope::UpTo {
        here: ChapterIx(99),
    };
    let (clamped, end) = run_in(&book, &query, up_to, &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(clamped.order, whole.order);
    assert_eq!(clamped.last_progress, whole.last_progress);
    let from = SearchScope::From {
        here: ChapterIx(99),
    };
    let (nothing, end) = run_in(&book, &query, from, &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert!(nothing.order.is_empty());
    assert_eq!(nothing.last_progress.unwrap_or((0, 0)), (0, 0));
    let only = SearchScope::Only {
        here: ChapterIx(99),
    };
    let (nothing, end) = run_in(&book, &query, only, &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert!(nothing.order.is_empty());
}

#[test]
fn scopes_arrive_from_the_pane_as_tagged_json() {
    let parse = |json: &str| serde_json::from_str::<SearchScope>(json);
    assert_eq!(parse(r#"{"kind":"whole"}"#).unwrap(), SearchScope::Whole);
    assert_eq!(
        parse(r#"{"kind":"upTo","here":3}"#).unwrap(),
        SearchScope::UpTo { here: ChapterIx(3) }
    );
    assert_eq!(
        parse(r#"{"kind":"from","here":3}"#).unwrap(),
        SearchScope::From { here: ChapterIx(3) }
    );
    assert_eq!(
        parse(r#"{"kind":"only","here":3}"#).unwrap(),
        SearchScope::Only { here: ChapterIx(3) }
    );
    assert!(parse(r#"{"kind":"before","here":3}"#).is_err());
    assert!(parse(r#"{"kind":"from"}"#).is_err());
}

#[test]
fn whole_word_and_case_mean_what_they_say() {
    let dir = scratch("words");
    let path = dir.join("book.epub");
    tiny_book(
        &path,
        &[
            "<p>the The THE other bathe the-end the_end (the) été ÉTÉ étéé C++ C++x aC++</p>",
            "<p>no match</p>",
        ],
    );
    let book = open_book(&path, "words-data");
    let count = |query: Query| run(&book, &query, &plenty()).0.total();
    assert_eq!(count(literal("the", Case::Sensitive, false)), 6);
    assert_eq!(count(literal("the", Case::Insensitive, false)), 8);
    assert_eq!(count(literal("the", Case::Sensitive, true)), 3);
    assert_eq!(count(literal("the", Case::Insensitive, true)), 5);
    assert_eq!(count(literal("été", Case::Insensitive, true)), 2);
    assert_eq!(count(literal("C++", Case::Sensitive, false)), 3);
    assert_eq!(count(literal("C++", Case::Sensitive, true)), 1);
    assert_eq!(count(literal("(the)", Case::Sensitive, true)), 1);
}

#[test]
fn a_hit_carries_its_exact_range_context_and_tag_note() {
    let dir = scratch("hit");
    let path = dir.join("book.epub");
    tiny_book(
        &path,
        &["<p>foo <b>bar</b> baz &amp; qux</p><p>next block</p>"],
    );
    let book = open_book(&path, "hit-data");
    let visible = book.read().visible_text(ChapterIx(0)).unwrap();
    let text = visible.as_str().to_owned();
    let query = |term: &str| {
        let (seen, _) = run(&book, &literal(term, Case::Sensitive, false), &plenty());
        seen.hits[&0][0].clone()
    };
    let within = query("baz");
    assert!(!within.spans_tags);
    assert_eq!(
        &text[within.range.start as usize..within.range.end as usize],
        "baz"
    );
    assert_eq!(within.before, "foo bar ");
    assert!(within.after.starts_with(" & qux"));
    assert_eq!(within.replaced, None);
    assert!(query("o ba").spans_tags);
    assert!(query("qux\nnext").spans_tags);
    assert!(!query("& qux").spans_tags);
}

#[test]
fn context_and_matched_text_stay_bounded() {
    let dir = scratch("bounded");
    let path = dir.join("book.epub");
    let long = "é".repeat(5_000);
    tiny_book(&path, &[&format!("<p>{long}</p>")]);
    let book = open_book(&path, "bounded-data");
    let (seen, end) = run(
        &book,
        &literal(&"é".repeat(4_000), Case::Sensitive, false),
        &plenty(),
    );
    assert_eq!(end, SearchEnd::Done);
    let hit = &seen.hits[&0][0];
    assert_eq!(hit.range.end - hit.range.start, 8_000);
    assert_eq!(hit.matched.chars().count(), 200);
    assert_eq!(hit.after.chars().count(), 60);
}

#[test]
fn results_stay_in_reading_order_when_early_chapters_are_the_slowest() {
    let book = open_book(&fixture("large-h1-p"), "order");
    let query = literal("the", Case::Insensitive, true);
    let options = SearchOptions {
        max_hits: u32::MAX,
        delay: Some(Arc::new(|chapter: ChapterIx| {
            Duration::from_millis(u64::from(40u32.saturating_sub(chapter.0)))
        })),
        ..SearchOptions::default()
    };
    let (delayed, end) = run(&book, &query, &options);
    let (straight, _) = run(&book, &query, &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert!(delayed.order.len() >= 6_000);
    assert!(delayed.order.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(delayed.hits, straight.hits);
}

#[test]
fn hits_within_a_chapter_are_ascending_and_never_overlap() {
    let book = open_book(&fixture("large-h1-div-br"), "ascending");
    let (seen, _) = run(&book, &literal("e", Case::Insensitive, false), &plenty());
    assert!(seen.total() > 1_000_000);
    for hits in seen.hits.values() {
        assert!(
            hits.windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
    }
}

fn cancel_after_first_event(book: &Arc<Book>, options: SearchOptions) -> (Duration, Duration) {
    let (fired, seen_event) = mpsc::channel();
    let token = CancelToken::default();
    let search_token = token.clone();
    let book = book.clone();
    let searching = std::thread::spawn(move || {
        let query = literal("the", Case::Insensitive, false);
        let mut fired = Some(fired);
        let end = book.search(
            &query,
            SearchScope::Whole,
            &options,
            &search_token,
            &mut |event| {
                if matches!(event, SearchEvent::Progress { .. })
                    && let Some(fired) = fired.take()
                {
                    let _ = fired.send(());
                }
            },
        );
        (end, Instant::now())
    });
    seen_event.recv().expect("the search makes progress");
    std::thread::sleep(Duration::from_millis(1));
    let cancelled_at = Instant::now();
    token.cancel();
    let (end, returned_at) = searching.join().unwrap();
    assert_eq!(end, SearchEnd::Cancelled);
    (returned_at - cancelled_at, cancelled_at.elapsed())
}

#[test]
fn cancelling_returns_within_a_frame_even_while_threads_are_busy() {
    let book = Arc::new(open_book(&fixture("large-h1-p"), "cancel"));
    let slow_after_start = Arc::new(|chapter: ChapterIx| {
        if chapter.0 < 2 {
            Duration::ZERO
        } else {
            Duration::from_millis(500)
        }
    });
    let mut latencies = Vec::new();
    for _ in 0..7 {
        let options = SearchOptions {
            delay: Some(slow_after_start.clone()),
            ..SearchOptions::default()
        };
        latencies.push(cancel_after_first_event(&book, options).0);
    }
    latencies.sort();
    println!("cancel latency {latencies:?}");
    assert!(latencies[3] < FRAME, "median {:?}", latencies[3]);
}

#[test]
fn a_new_search_after_a_cancel_is_correct_and_not_mixed_with_the_old_one() {
    let book = Arc::new(open_book(&fixture("large-h1-p"), "restart"));
    let (straight, _) = run(&book, &literal("the", Case::Sensitive, true), &plenty());
    for _ in 0..5 {
        cancel_after_first_event(&book, SearchOptions::default());
    }
    let (again, _) = run(&book, &literal("the", Case::Sensitive, true), &plenty());
    assert_eq!(straight.hits, again.hits);
}

#[test]
fn the_cap_stops_the_search_and_says_so() {
    let book = open_book(&fixture("large-h1-p"), "cap");
    let options = SearchOptions {
        max_hits: 1_234,
        ..SearchOptions::default()
    };
    let (seen, end) = run(&book, &literal("the", Case::Insensitive, false), &options);
    assert_eq!(end, SearchEnd::Done);
    assert!(seen.capped);
    assert_eq!(seen.total(), 1_234);
    assert!(seen.order.len() < 100);
    assert_eq!(seen.order[0], 0);
}

#[test]
fn an_unreadable_chapter_is_reported_and_the_rest_is_searched() {
    let book = open_book(&fixture("malformed-missing-chapter"), "unreadable");
    let (seen, end) = run(&book, &literal("the", Case::Insensitive, false), &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.unreadable.len(), 1);
    assert!(!seen.unreadable[0].1.is_empty());
    assert!(!seen.hits.is_empty());
    let (done, total) = seen.last_progress.unwrap();
    assert_eq!(done, total);
}

#[test]
fn an_oversize_query_is_refused_readably() {
    let book = open_book(&fixture("toc-none"), "oversize");
    let (_, end) = run(
        &book,
        &literal(&"x".repeat(100_000), Case::Sensitive, false),
        &plenty(),
    );
    assert!(matches!(end, SearchEnd::BadPattern(message) if !message.is_empty()));
    let (seen, end) = run(&book, &literal("", Case::Sensitive, false), &plenty());
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.total(), 0);
}

#[test]
fn regex_without_a_worker_gives_a_readable_failure() {
    let book = open_book(&fixture("toc-none"), "noworker");
    let query = Query::Regex {
        pattern: "a".to_owned(),
        case: Case::Sensitive,
    };
    let (_, end) = run(&book, &query, &plenty());
    assert!(matches!(end, SearchEnd::Failed(message) if !message.is_empty()));
    let options = SearchOptions {
        worker_exe: Some("/nonexistent/jadeslip-worker".into()),
        ..plenty()
    };
    let (_, end) = run(&book, &query, &options);
    assert!(matches!(end, SearchEnd::Failed(message) if !message.is_empty()));
}

#[test]
fn a_save_during_a_search_succeeds_and_the_search_stays_on_its_snapshot() {
    let dir = scratch("race");
    let path = dir.join("copy.epub");
    std::fs::copy(fixture("large-h1-p"), &path).unwrap();
    let book = Arc::new(open_book(&path, "race-data"));
    let query = literal("the", Case::Insensitive, false);
    let (before, _) = run(&book, &query, &plenty());
    let original_rev = book.read().render(ChapterIx(5)).unwrap().rev;
    let (first_chapter, reached) = mpsc::channel();
    let searching = {
        let book = book.clone();
        let query = query.clone();
        let options = SearchOptions {
            delay: Some(Arc::new(|_| Duration::from_micros(800))),
            ..plenty()
        };
        std::thread::spawn(move || {
            let mut seen = Seen::default();
            let mut first = Some(first_chapter);
            let end = book.search(
                &query,
                SearchScope::Whole,
                &options,
                &CancelToken::default(),
                &mut |event| {
                    if let Some(first) = first.take() {
                        let _ = first.send(());
                    }
                    seen.take(event);
                },
            );
            (seen, end)
        })
    };
    reached.recv().unwrap();
    let saved = book
        .save_chapter(
            ChapterIx(5),
            &BlockEdits {
                base: original_rev,
                changes: vec![BlockChange::Replace {
                    of: BlockIx(1),
                    xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">the edited the the</p>"
                        .to_owned(),
                }],
            },
        )
        .expect("the save is not blocked by the search");
    assert_ne!(saved.rev, original_rev);
    let (during, end) = searching.join().unwrap();
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(during.hits, before.hits);
    assert_eq!(during.revs[&5], original_rev);
    let (after, _) = run(&book, &query, &plenty());
    assert_ne!(after.hits[&5], before.hits[&5]);
    assert_eq!(after.revs[&5], saved.rev);
}

#[test]
fn memory_for_loaded_chapters_stays_inside_the_budget() {
    let book = open_book(&fixture("large-h1-p"), "budget");
    let reader = Reader::open(File::open(book.path()).unwrap(), DEFAULT_LIMITS).unwrap();
    let inflated = reader.summary().inflated_spine_bytes;
    let (seen, _) = run(
        &book,
        &literal("zzzzqqqq", Case::Sensitive, false),
        &plenty(),
    );
    assert_eq!(seen.total(), 0);
    let high_water = book.search_high_water();
    println!("budget high water {high_water} of {inflated} inflated");
    assert!(high_water > 0);
    assert!(high_water < inflated / 4);
}
