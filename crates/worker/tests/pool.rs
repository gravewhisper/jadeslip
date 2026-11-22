#[path = "../../core/tests/common/mod.rs"]
mod common;

use common::{fixture, scratch, tiny_book, truth};
use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, CancelToken, Case, ChapterIx, DEFAULT_LIMITS, Hit,
    Query, Reader, SearchEnd, SearchEvent, SearchOptions, SearchScope, probe_worker,
};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_micros(16_700);
const QUADRATIC: &str = "(?s)(?=.*QQQ)x";

fn worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jadeslip-worker"))
}

fn options() -> SearchOptions {
    SearchOptions {
        max_hits: u32::MAX,
        worker_exe: Some(worker_exe()),
        book_deadline: Duration::from_secs(120),
        ..SearchOptions::default()
    }
}

fn regex(pattern: &str, case: Case) -> Query {
    Query::Regex {
        pattern: pattern.to_owned(),
        case,
    }
}

#[derive(Default)]
struct Seen {
    order: Vec<u32>,
    hits: BTreeMap<u32, Vec<Hit>>,
    limited: Vec<u32>,
    unreadable: Vec<u32>,
    done: u32,
    total: u32,
}

impl Seen {
    fn take(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::Hits { chapter, hits, .. } => {
                self.order.push(chapter.0);
                self.hits.insert(chapter.0, hits);
            }
            SearchEvent::ChapterLimited { chapter } => self.limited.push(chapter.0),
            SearchEvent::ChapterUnreadable { chapter, .. } => self.unreadable.push(chapter.0),
            SearchEvent::Progress { done, total } => {
                self.done = done;
                self.total = total;
            }
            SearchEvent::CapReached => {}
        }
    }

    fn total_hits(&self) -> usize {
        self.hits.values().map(Vec::len).sum()
    }
}

fn open_book(path: &Path, name: &str) -> Book {
    Book::open_by_fingerprint(path, &scratch(name), DEFAULT_LIMITS).expect("book opens")
}

fn run(book: &Book, query: &Query, options: &SearchOptions) -> (Seen, SearchEnd) {
    let mut seen = Seen::default();
    let end = book.search(
        query,
        SearchScope::Whole,
        options,
        &CancelToken::default(),
        &mut |event| seen.take(event),
    );
    (seen, end)
}

fn tiny(name: &str, chapters: &[&str]) -> Book {
    let dir = scratch(name);
    let path = dir.join("book.epub");
    tiny_book(&path, chapters);
    open_book(&path, &format!("{name}-data"))
}

fn children_of_me() -> Vec<u32> {
    children_of(std::process::id())
}

fn children_of(parent: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let Ok(tasks) = fs::read_dir(format!("/proc/{parent}/task")) else {
        return found;
    };
    for task in tasks.flatten() {
        if let Ok(text) = fs::read_to_string(task.path().join("children")) {
            found.extend(
                text.split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok()),
            );
        }
    }
    found
}

fn is_worker(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .is_ok_and(|comm| comm.starts_with("jadeslip-wor"))
}

fn workers_of_me() -> Vec<u32> {
    children_of_me()
        .into_iter()
        .filter(|pid| is_worker(*pid))
        .collect()
}

fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            != Some("Z")
    })
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    done()
}

static SERIAL: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn escape_for_regex(term: &str) -> String {
    let mut out = String::new();
    for c in term.chars() {
        if "\\.+*?()|[]{}^$#&-~ ".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[test]
fn escaped_terms_give_the_planted_counts_through_the_worker() {
    let _serial = one_at_a_time();
    let truth = truth("gbk-xml-decl");
    let book = open_book(&fixture("gbk-xml-decl"), "truth-gbk");
    let mut checked = 0;
    for (term, facts) in truth["terms"].as_object().unwrap() {
        let (seen, end) = run(
            &book,
            &regex(&escape_for_regex(term), Case::Sensitive),
            &options(),
        );
        assert_eq!(end, SearchEnd::Done, "{term}");
        let counts: u64 = facts["exact_total"].as_u64().unwrap();
        assert_eq!(seen.total_hits() as u64, counts, "{term}");
        checked += 1;
    }
    assert!(checked > 10);
}

#[test]
fn the_worker_and_the_literal_engine_agree_hit_for_hit_on_a_large_book() {
    let _serial = one_at_a_time();
    let book = open_book(&fixture("large-h1-div-br"), "agree");
    let (through_worker, end) = run(&book, &regex(r"\bthe\b", Case::Insensitive), &options());
    assert_eq!(end, SearchEnd::Done);
    let literal = Query::Literal {
        text: "the".to_owned(),
        case: Case::Insensitive,
        whole_word: true,
    };
    let (threads, _) = run(&book, &literal, &options());
    assert!(through_worker.total_hits() > 100_000);
    assert_eq!(through_worker.hits, threads.hits);
    assert!(
        through_worker
            .order
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
}

#[test]
fn lookaround_and_non_ascii_work_over_visible_text() {
    let _serial = one_at_a_time();
    let book = tiny(
        "features",
        &["<p>foo<b>bar</b> été &amp; ÉTÉ 漢字 foobaz</p><p>aa aa</p><script>foobar</script>"],
    );
    let count = |pattern: &str, case| {
        let (seen, end) = run(&book, &regex(pattern, case), &options());
        assert_eq!(end, SearchEnd::Done, "{pattern}");
        seen.total_hits()
    };
    assert_eq!(count("foo(?=bar)", Case::Sensitive), 1);
    assert_eq!(count("foo(?!bar)", Case::Sensitive), 1);
    assert_eq!(count("(?<=foo)bar", Case::Sensitive), 1);
    assert_eq!(count("été", Case::Insensitive), 2);
    assert_eq!(count("été", Case::Sensitive), 1);
    assert_eq!(count(r"\p{Han}+", Case::Sensitive), 1);
    assert_eq!(count(r"(\w)\1 ", Case::Sensitive), 1);
    assert_eq!(count(r"& É", Case::Sensitive), 1);
    assert_eq!(count(r"baz.aa", Case::Sensitive), 0);
    assert_eq!(count(r"(?s)baz.aa", Case::Sensitive), 1);
}

#[test]
fn empty_matches_are_results_at_their_position() {
    let _serial = one_at_a_time();
    let book = tiny("empty", &["<p>aaa bbb</p>"]);
    let (seen, end) = run(&book, &regex("x*", Case::Sensitive), &options());
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.total_hits(), 8);
    let (seen, _) = run(&book, &regex("a*", Case::Sensitive), &options());
    assert_eq!(seen.total_hits(), 5);
}

#[test]
fn a_bad_pattern_gives_a_readable_message_and_leaves_no_worker() {
    let _serial = one_at_a_time();
    let book = tiny("bad", &["<p>text</p>"]);
    for pattern in ["(", "[", r"\", "(?<=a", r"\p{Nope}", "(?z)"] {
        let (seen, end) = run(&book, &regex(pattern, Case::Sensitive), &options());
        let SearchEnd::BadPattern(message) = end else {
            panic!("{pattern}: {end:?}");
        };
        assert!(message.len() > 5, "{pattern}: {message}");
        assert_eq!(seen.total_hits(), 0);
    }
    assert!(wait_until(Duration::from_secs(2), || workers_of_me().is_empty()));
}

#[test]
fn hostile_patterns_cannot_hurt_the_parent() {
    let _serial = one_at_a_time();
    let text = format!(
        "<p>{}!</p><p>{}</p>",
        "a".repeat(60_000),
        "ab ".repeat(20_000)
    );
    let book = tiny("hostile", &[&text, "<p>second chapter</p>"]);
    let deep = "(".repeat(200_000) + &")".repeat(200_000);
    let deep_short = "(".repeat(5_000) + &")".repeat(5_000);
    let wide = "(a|".repeat(5_000) + "b" + &")".repeat(5_000);
    let hostile = [
        "(a+)+$",
        "(a|aa)+$",
        r"(a+)+\1b",
        "^(?:a|a)*(?!a)b",
        "(?s)(?=.*QQQ)a",
        "(?s).*",
        "(?s).",
        "a{100000}{100000}{100000}",
        "(a{1,1000}){1,1000}",
        r"\p{L}{1000}{1000}",
        "(?<=a+)b",
        &deep,
        &deep_short,
        &wide,
    ];
    let quick = SearchOptions {
        chapter_deadline: Duration::from_millis(200),
        max_hits: 5_000,
        ..options()
    };
    for pattern in hostile {
        let label: String = pattern.chars().take(24).collect();
        let started = Instant::now();
        let (seen, end) = run(&book, &regex(pattern, Case::Sensitive), &quick);
        println!(
            "{label}: {end:?} hits {} limited {} in {:?}",
            seen.total_hits(),
            seen.limited.len(),
            started.elapsed()
        );
        assert!(started.elapsed() < Duration::from_secs(20), "{label}");
        assert!(
            !matches!(end, SearchEnd::Cancelled | SearchEnd::Failed(_)),
            "{label}"
        );
    }
    assert!(wait_until(Duration::from_secs(2), || workers_of_me().is_empty()));
}

#[test]
fn results_stay_in_reading_order_through_the_pool_under_injected_delay() {
    let _serial = one_at_a_time();
    let book = open_book(&fixture("toc-nested-nav"), "order");
    let query = regex(r"\b(?=[a-z])\w+", Case::Sensitive);
    let (straight, _) = run(&book, &query, &options());
    let delayed_options = SearchOptions {
        delay: Some(Arc::new(|chapter: ChapterIx| {
            Duration::from_millis(u64::from(150u32.saturating_sub(chapter.0 * 30)))
        })),
        ..options()
    };
    let (delayed, end) = run(&book, &query, &delayed_options);
    assert_eq!(end, SearchEnd::Done);
    assert!(delayed.order.len() >= 3);
    assert!(delayed.order.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(delayed.hits, straight.hits);
}

#[test]
fn a_stuck_chapter_is_killed_alone_and_the_rest_of_the_book_is_searched() {
    let _serial = one_at_a_time();
    let chapters: Vec<String> = (0..12).map(|ix| format!("<p>marker {ix}</p>")).collect();
    let refs: Vec<&str> = chapters.iter().map(String::as_str).collect();
    let book = tiny("stuck", &refs);
    let stuck = SearchOptions {
        chapter_deadline: Duration::from_millis(100),
        threads: 3,
        delay: Some(Arc::new(|chapter: ChapterIx| {
            if chapter.0 == 4 {
                Duration::from_secs(600)
            } else {
                Duration::ZERO
            }
        })),
        ..options()
    };
    let started = Instant::now();
    let (seen, end) = run(&book, &regex("marker", Case::Sensitive), &stuck);
    assert_eq!(end, SearchEnd::TooSlow { limited: 1 });
    assert_eq!(seen.limited, vec![4]);
    assert_eq!(seen.total_hits(), 11);
    assert!(!seen.hits.contains_key(&4));
    assert_eq!(seen.done, 12);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(wait_until(Duration::from_secs(2), || workers_of_me().is_empty()));
}

#[test]
fn the_quadratic_lookahead_is_limited_and_the_others_still_finish() {
    let _serial = one_at_a_time();
    let big = "x".repeat(500_000);
    let book = tiny(
        "quadratic",
        &[
            "<p>short x one</p>",
            &format!("<p>{big}</p>"),
            "<p>QQQ x QQQ</p>",
            "<p>last</p>",
        ],
    );
    let quick = SearchOptions {
        chapter_deadline: Duration::from_millis(300),
        ..options()
    };
    let started = Instant::now();
    let (seen, end) = run(&book, &regex(QUADRATIC, Case::Sensitive), &quick);
    assert_eq!(end, SearchEnd::TooSlow { limited: 1 });
    assert_eq!(seen.limited, vec![1]);
    assert_eq!(seen.done, 4);
    assert_eq!(seen.hits.get(&2).map(Vec::len), Some(1));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn the_whole_book_deadline_ends_the_search_and_counts_what_was_left() {
    let _serial = one_at_a_time();
    let chapters: Vec<String> = (0..40).map(|ix| format!("<p>marker {ix}</p>")).collect();
    let refs: Vec<&str> = chapters.iter().map(String::as_str).collect();
    let book = tiny("book-deadline", &refs);
    let short = SearchOptions {
        threads: 2,
        book_deadline: Duration::from_millis(300),
        delay: Some(Arc::new(|_| Duration::from_millis(40))),
        ..options()
    };
    let started = Instant::now();
    let (seen, end) = run(&book, &regex("marker", Case::Sensitive), &short);
    let SearchEnd::TooSlow { limited } = end else {
        panic!("{end:?}");
    };
    assert!(limited > 0);
    assert_eq!(limited as usize, seen.limited.len());
    assert_eq!(seen.total_hits() + seen.limited.len(), 40);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_outside_kill_of_a_worker_only_limits_its_chapter() {
    let _serial = one_at_a_time();
    let chapters: Vec<String> = (0..8).map(|ix| format!("<p>marker {ix}</p>")).collect();
    let refs: Vec<&str> = chapters.iter().map(String::as_str).collect();
    let book = Arc::new(tiny("outside-kill", &refs));
    let stuck = SearchOptions {
        threads: 1,
        chapter_deadline: Duration::from_secs(30),
        delay: Some(Arc::new(|chapter: ChapterIx| {
            if chapter.0 == 2 {
                Duration::from_secs(60)
            } else {
                Duration::ZERO
            }
        })),
        ..options()
    };
    let searching = {
        let book = book.clone();
        std::thread::spawn(move || run(&book, &regex("marker", Case::Sensitive), &stuck))
    };
    assert!(wait_until(Duration::from_secs(5), || !workers_of_me().is_empty()));
    std::thread::sleep(Duration::from_millis(200));
    for pid in workers_of_me() {
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    let (seen, end) = searching.join().unwrap();
    assert_eq!(end, SearchEnd::TooSlow { limited: 1 });
    assert_eq!(seen.limited, vec![2]);
    assert_eq!(seen.total_hits(), 7);
}

#[test]
fn cancelling_returns_within_a_frame_and_kills_every_worker() {
    let _serial = one_at_a_time();
    let book = Arc::new(open_book(&fixture("large-h1-p"), "cancel"));
    let mut latencies = Vec::new();
    let mut gone_in = Vec::new();
    for _ in 0..7 {
        let token = CancelToken::default();
        let (first_tx, first) = mpsc::channel();
        let searching = {
            let book = book.clone();
            let token = token.clone();
            let slow = SearchOptions {
                threads: 6,
                delay: Some(Arc::new(|chapter: ChapterIx| {
                    if chapter.0 == 0 {
                        Duration::ZERO
                    } else {
                        Duration::from_secs(60)
                    }
                })),
                ..options()
            };
            std::thread::spawn(move || {
                let mut first_tx = Some(first_tx);
                let end = book.search(
                    &regex("(?=the)the", Case::Sensitive),
                    SearchScope::Whole,
                    &slow,
                    &token,
                    &mut |_| {
                        if let Some(first) = first_tx.take() {
                            let _ = first.send(());
                        }
                    },
                );
                (end, Instant::now())
            })
        };
        first.recv().expect("first chapter arrives");
        assert!(wait_until(Duration::from_secs(5), || workers_of_me().len() >= 6));
        let cancelled_at = Instant::now();
        token.cancel();
        let (end, returned_at) = searching.join().unwrap();
        assert_eq!(end, SearchEnd::Cancelled);
        latencies.push(returned_at - cancelled_at);
        assert!(wait_until(Duration::from_secs(2), || workers_of_me().is_empty()));
        gone_in.push(cancelled_at.elapsed());
    }
    latencies.sort();
    gone_in.sort();
    println!("cancel returned {latencies:?}, workers gone {gone_in:?}");
    assert!(latencies[3] < FRAME, "median {:?}", latencies[3]);
    assert!(
        gone_in[3] < Duration::from_millis(100),
        "median {:?}",
        gone_in[3]
    );
}

fn worker_limit(pid: u32, name: &str) -> String {
    let limits = fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    limits
        .lines()
        .find(|line| line.starts_with(name))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn the_allocation_cap_really_refuses_and_a_memory_eater_only_loses_its_chapter() {
    let _serial = one_at_a_time();
    let reader = Reader::open(File::open(fixture("toc-none")).unwrap(), DEFAULT_LIMITS).unwrap();
    let cap = 256u64 << 20;
    let (small_refused, _) = probe_worker(&worker_exe(), &reader, cap, 64 << 20).unwrap();
    assert!(!small_refused, "an allocation under the cap must succeed");
    let (big_refused, _) = probe_worker(&worker_exe(), &reader, cap, 512 << 20).unwrap();
    assert!(big_refused, "an allocation over the cap must be refused");
    let (uncapped_refused, _) = probe_worker(&worker_exe(), &reader, 4 << 30, 512 << 20).unwrap();
    assert!(!uncapped_refused, "the refusal comes from the cap");

    let text = "ab ".repeat(2_000_000);
    let book = tiny(
        "eater",
        &[
            "<p>small ab</p>",
            &format!("<p>{text}</p>"),
            "<p>tail ab</p>",
        ],
    );
    let tight = SearchOptions {
        memory_cap: 96 << 20,
        chapter_deadline: Duration::from_secs(20),
        threads: 1,
        ..options()
    };
    let (seen, end) = run(&book, &regex(r"(?s)(?:(?=.).)*Q", Case::Sensitive), &tight);
    println!("eater: {end:?} limited {:?}", seen.limited);
    assert_eq!(end, SearchEnd::TooSlow { limited: 1 });
    assert_eq!(seen.limited, vec![1]);
    assert_eq!(seen.done, 3);
}

#[test]
fn a_worker_has_only_its_pipes_and_the_book_open() {
    let _serial = one_at_a_time();
    let dir = scratch("fds");
    let path = dir.join("copy.epub");
    fs::copy(fixture("toc-none"), &path).unwrap();
    let lock = dir.join("held");
    let _holding_a_private_file = File::create(&lock).unwrap();
    let book = Arc::new(open_book(&path, "fds-data"));
    let token = CancelToken::default();
    let searching = {
        let book = book.clone();
        let token = token.clone();
        let slow = SearchOptions {
            threads: 1,
            delay: Some(Arc::new(|_| Duration::from_secs(60))),
            ..options()
        };
        std::thread::spawn(move || {
            book.search(
                &regex("a", Case::Sensitive),
                SearchScope::Whole,
                &slow,
                &token,
                &mut |_| {},
            )
        })
    };
    assert!(wait_until(Duration::from_secs(5), || !workers_of_me().is_empty()));
    let pid = workers_of_me()[0];
    assert!(wait_until(Duration::from_secs(2), || {
        fs::read_dir(format!("/proc/{pid}/fd"))
            .is_ok_and(|fds| fds.flatten().any(|fd| fd.file_name() == "3"))
    }));
    let mut names: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .flatten()
        .map(|fd| fd.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let links: Vec<String> = names
        .iter()
        .filter_map(|fd| fs::read_link(format!("/proc/{pid}/fd/{fd}")).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["0", "1", "2", "3"], "{links:?}");
    assert!(links[3].ends_with("copy.epub"), "{}", links[3]);
    assert!(links.iter().all(|target| !target.contains("commit.lock")));
    assert!(links.iter().all(|target| !target.contains("held")));
    let memory = worker_limit(pid, "Max address space");
    assert!(memory.contains(&(1u64 << 30).to_string()), "{memory}");
    assert!(worker_limit(pid, "Max core file size").contains(" 0 "));
    token.cancel();
    assert_eq!(searching.join().unwrap(), SearchEnd::Cancelled);
}

#[test]
fn saves_race_a_running_regex_search_without_lock_failures() {
    let _serial = one_at_a_time();
    let dir = scratch("race");
    let path = dir.join("copy.epub");
    fs::copy(fixture("large-h1-p"), &path).unwrap();
    let book = Arc::new(open_book(&path, "race-data"));
    let query = regex(r"\bthe(?= )", Case::Sensitive);
    let patient = SearchOptions {
        chapter_deadline: Duration::from_secs(60),
        book_deadline: Duration::from_secs(600),
        ..options()
    };
    let (before, end) = run(&book, &query, &patient);
    assert_eq!(end, SearchEnd::Done);
    let saving = Arc::new(AtomicBool::new(true));
    let (first_tx, first) = mpsc::channel();
    let searching = {
        let book = book.clone();
        let query = query.clone();
        let saving = saving.clone();
        let patient = patient.clone();
        std::thread::spawn(move || {
            let mut seen = Seen::default();
            let mut first_tx = Some(first_tx);
            let slow = SearchOptions {
                threads: 4,
                delay: Some(Arc::new(move |_| {
                    if saving.load(Ordering::SeqCst) {
                        Duration::from_millis(20)
                    } else {
                        Duration::ZERO
                    }
                })),
                ..patient
            };
            let end = book.search(
                &query,
                SearchScope::Whole,
                &slow,
                &CancelToken::default(),
                &mut |event| {
                    if let Some(first) = first_tx.take() {
                        let _ = first.send(());
                    }
                    seen.take(event);
                },
            );
            (seen, end)
        })
    };
    first.recv().unwrap();
    let mut saved = 0;
    for ix in [7u32, 8, 9, 10, 11] {
        let base = book.read().render(ChapterIx(ix)).unwrap().rev;
        book.save_chapter(
            ChapterIx(ix),
            &BlockEdits {
                base,
                changes: vec![BlockChange::Replace {
                    of: BlockIx(1),
                    xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">changed the words</p>"
                        .to_owned(),
                }],
            },
        )
        .expect("a save racing the search is not refused");
        saved += 1;
    }
    assert_eq!(saved, 5);
    assert!(
        !searching.is_finished(),
        "the search was still running when the last save ended"
    );
    saving.store(false, Ordering::SeqCst);
    let (during, end) = searching.join().unwrap();
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(during.hits, before.hits, "the search stays on its snapshot");
    let (after, _) = run(&book, &query, &patient);
    assert_ne!(after.hits[&7], before.hits[&7]);
}

#[test]
fn saves_succeed_while_workers_are_killed_and_respawned_constantly() {
    let _serial = one_at_a_time();
    let dir = scratch("respawn");
    let path = dir.join("copy.epub");
    fs::copy(fixture("large-h1-p"), &path).unwrap();
    let book = Arc::new(open_book(&path, "respawn-data"));
    let token = CancelToken::default();
    let searching = {
        let book = book.clone();
        let token = token.clone();
        let churn = SearchOptions {
            threads: 8,
            chapter_deadline: Duration::from_millis(1),
            book_deadline: Duration::from_secs(120),
            delay: Some(Arc::new(|_| Duration::from_secs(60))),
            ..options()
        };
        std::thread::spawn(move || {
            let mut killed = 0u32;
            let end = book.search(
                &regex("the", Case::Sensitive),
                SearchScope::Whole,
                &churn,
                &token,
                &mut |event| {
                    if matches!(event, SearchEvent::ChapterLimited { .. }) {
                        killed += 1;
                    }
                },
            );
            (end, killed)
        })
    };
    assert!(wait_until(Duration::from_secs(5), || workers_of_me().len() >= 4));
    let mut slowest = Duration::ZERO;
    for ix in 20..50u32 {
        let base = book.read().render(ChapterIx(ix)).unwrap().rev;
        let started = Instant::now();
        book.save_chapter(
            ChapterIx(ix),
            &BlockEdits {
                base,
                changes: vec![BlockChange::Replace {
                    of: BlockIx(1),
                    xml: "<p xmlns=\"http://www.w3.org/1999/xhtml\">changed the words</p>"
                        .to_owned(),
                }],
            },
        )
        .expect("the commit lock is never lost to a forked child");
        slowest = slowest.max(started.elapsed());
    }
    token.cancel();
    let (end, killed) = searching.join().unwrap();
    assert_eq!(end, SearchEnd::Cancelled);
    assert!(
        killed >= 30,
        "only {killed} workers were killed during the saves"
    );
    println!("30 saves against {killed} worker kills, slowest {slowest:?}");
}

#[test]
#[ignore = "only runs as the parent process of the kill proof"]
fn parent_role() {
    let book = std::env::var("SS_PARENT_BOOK").expect("started by the kill proof");
    let book = open_book(Path::new(&book), "parent-role");
    let stuck = SearchOptions {
        threads: 4,
        delay: Some(Arc::new(|_| Duration::from_secs(600))),
        ..options()
    };
    book.search(
        &regex("a", Case::Sensitive),
        SearchScope::Whole,
        &stuck,
        &CancelToken::default(),
        &mut |_| {},
    );
}

#[test]
fn a_parent_killed_with_sigkill_leaves_no_worker() {
    let _serial = one_at_a_time();
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "parent_role", "--ignored", "--nocapture"])
        .env("SS_PARENT_BOOK", fixture("large-h1-p"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let parent_pid = parent.id();
    let workers = || {
        children_of(parent_pid)
            .into_iter()
            .filter(|pid| is_worker(*pid))
            .collect::<Vec<u32>>()
    };
    assert!(wait_until(Duration::from_secs(10), || workers().len() == 4));
    let before = workers();
    unsafe { libc::kill(parent_pid as i32, libc::SIGKILL) };
    parent.wait().unwrap();
    let killed_at = Instant::now();
    assert!(
        wait_until(Duration::from_secs(2), || before
            .iter()
            .all(|pid| !alive(*pid))),
        "workers survived their parent"
    );
    println!("workers gone {:?} after the kill", killed_at.elapsed());
}

fn peak_rss_mib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let line = status
        .lines()
        .find(|line| line.starts_with("VmHWM"))
        .unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap()
        / 1024
}

#[test]
#[ignore = "slow: run with --release, builds a book of huge all-entity chapters"]
fn huge_all_entity_chapters_stay_inside_the_byte_budget() {
    let _serial = one_at_a_time();
    let dir = scratch("entities");
    let path = dir.join("book.epub");
    let chapter = format!("<p>{}needle</p>", "&amp;".repeat(12 << 20));
    let chapters: Vec<&str> = std::iter::repeat_n(chapter.as_str(), 8).collect();
    tiny_book(&path, &chapters);
    drop(chapter);
    let book = open_book(&path, "entities-data");
    let literal = Query::Literal {
        text: "needle".to_owned(),
        case: Case::Sensitive,
        whole_word: false,
    };
    let started = Instant::now();
    let (seen, end) = run(&book, &literal, &options());
    let literal_ms = started.elapsed().as_millis();
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.total_hits(), 8);
    let high_water = book.search_high_water();
    println!(
        "literal {literal_ms} ms, budget high water {} MiB, parent peak {} MiB",
        high_water >> 20,
        peak_rss_mib()
    );
    assert!(high_water <= jadeslip_core::CHAPTER_BYTE_BUDGET);
    assert!(peak_rss_mib() < 1200, "{} MiB", peak_rss_mib());

    let started = Instant::now();
    let generous = SearchOptions {
        book_deadline: Duration::from_secs(120),
        ..options()
    };
    let (seen, end) = run(&book, &regex("needle(?=$)", Case::Sensitive), &generous);
    println!(
        "regex {} ms, end {end:?}, hits {}, limited {}",
        started.elapsed().as_millis(),
        seen.total_hits(),
        seen.limited.len()
    );
    assert_eq!(end, SearchEnd::Done);
    assert_eq!(seen.total_hits(), 8);
    assert!(book.search_high_water() <= jadeslip_core::CHAPTER_BYTE_BUDGET);
}
