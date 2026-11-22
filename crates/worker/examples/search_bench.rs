use jadeslip_core::{
    Book, CHAPTER_BYTE_BUDGET, CancelToken, Case, DEFAULT_LIMITS, Extracted, Query, Reader,
    SearchEnd, SearchEvent, SearchOptions, SearchScope,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn status_kib(pid: u32, field: &str) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with(field))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn worker_pids(parent: u32) -> Vec<u32> {
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

fn phases(path: &PathBuf) {
    let reader =
        Reader::open(fs::File::open(path).expect("book file"), DEFAULT_LIMITS).expect("book opens");
    let chapters: Vec<_> = reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .map(|row| row.ix)
        .collect();
    let started = Instant::now();
    let mut inflated = 0;
    for chapter in &chapters {
        if let Ok(Extracted::Document(bytes)) = reader.extract(*chapter) {
            inflated += bytes.len();
        }
    }
    let inflate_ms = started.elapsed().as_secs_f64() * 1e3;
    let started = Instant::now();
    let mut visible = 0;
    let mut texts = Vec::new();
    for chapter in &chapters {
        if let Ok(text) = reader.visible_text(*chapter) {
            visible += text.as_str().len();
            texts.push(text);
        }
    }
    let model_ms = started.elapsed().as_secs_f64() * 1e3;
    let needle = regex::Regex::new("zzzqqqz").expect("regex");
    let started = Instant::now();
    let found: usize = texts
        .iter()
        .map(|text| needle.find_iter(text.as_str()).count())
        .sum();
    let scan_ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "phases one core: inflate_only_ms={inflate_ms:.1} inflate_plus_model_ms={model_ms:.1} regex_scan_ms={scan_ms:.1} inflated_mib={:.1} visible_mib={:.1} found={found}",
        inflated as f64 / 1048576.0,
        visible as f64 / 1048576.0,
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("book path"));
    let mode = args.next().expect("phases, literal, word, fold or regex");
    if mode == "phases" {
        return phases(&path);
    }
    let text = args.next().expect("query");
    let threads: usize = args
        .next()
        .map_or(0, |value| value.parse().expect("threads"));
    let query = match mode.as_str() {
        "literal" => Query::Literal {
            text,
            case: Case::Sensitive,
            whole_word: false,
        },
        "fold" => Query::Literal {
            text,
            case: Case::Insensitive,
            whole_word: false,
        },
        "word" => Query::Literal {
            text,
            case: Case::Insensitive,
            whole_word: true,
        },
        "regex" => Query::Regex {
            pattern: text,
            case: Case::Sensitive,
        },
        _ => panic!("unknown mode"),
    };
    let data = jadeslip_fixtures::scratch::scratch("bench");
    let book = Book::open_by_fingerprint(&path, &data, DEFAULT_LIMITS).expect("book opens");
    let inflated = book.read().summary().inflated_spine_bytes;
    let worker = std::env::current_exe()
        .expect("own path")
        .parent()
        .and_then(|dir| dir.parent())
        .expect("examples sits inside the profile folder")
        .join("jadeslip-worker");
    let defaults = SearchOptions::default();
    let options = SearchOptions {
        max_hits: u32::MAX,
        worker_exe: Some(worker),
        threads: if threads == 0 {
            defaults.threads
        } else {
            threads
        },
        ..defaults
    };
    let me = std::process::id();
    let rss_before = status_kib(me, "VmRSS");
    let hwm_before = status_kib(me, "VmHWM");
    let peak_workers = Arc::new(AtomicU64::new(0));
    let peak_one = Arc::new(AtomicU64::new(0));
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler = {
        let (peak_workers, peak_one, sampling) =
            (peak_workers.clone(), peak_one.clone(), sampling.clone());
        std::thread::spawn(move || {
            while sampling.load(Ordering::Relaxed) {
                let sizes: Vec<u64> = worker_pids(me)
                    .into_iter()
                    .map(|pid| status_kib(pid, "VmRSS"))
                    .collect();
                peak_workers.fetch_max(sizes.iter().sum(), Ordering::Relaxed);
                peak_one.fetch_max(sizes.into_iter().max().unwrap_or(0), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let started = Instant::now();
    let mut hits = 0u64;
    let mut hit_bytes = 0u64;
    let mut chapters = 0u32;
    let mut limited = 0u32;
    let mut unreadable = 0u32;
    let mut first = None;
    let mut done = 0;
    let end = book.search(
        &query,
        SearchScope::Whole,
        &options,
        &CancelToken::default(),
        &mut |event| match event {
            SearchEvent::Hits { hits: found, .. } => {
                first.get_or_insert(started.elapsed());
                hits += found.len() as u64;
                hit_bytes += found
                    .iter()
                    .map(|hit| (hit.before.len() + hit.matched.len() + hit.after.len()) as u64)
                    .sum::<u64>();
                chapters += 1;
            }
            SearchEvent::Progress { done: at, .. } => done = at,
            SearchEvent::ChapterLimited { .. } => limited += 1,
            SearchEvent::ChapterUnreadable { .. } => unreadable += 1,
            SearchEvent::CapReached => {}
        },
    );
    let elapsed = started.elapsed();
    sampling.store(false, Ordering::Relaxed);
    sampler.join().expect("sampler ends");
    let end = match end {
        SearchEnd::BadPattern(_) => "BadPattern".to_owned(),
        SearchEnd::Failed(_) => "Failed".to_owned(),
        other => format!("{other:?}"),
    };
    println!(
        "{mode} threads={} ms={:.1} first_ms={:.1} end={end} scanned={done} hits={hits} hit_chapters={chapters} limited={limited} unreadable={unreadable} snippet_mib={:.1} parent_rss_before_mib={:.1} parent_hwm_before_mib={:.1} parent_hwm_after_mib={:.1} workers_peak_sum_mib={:.1} worker_peak_one_mib={:.1} budget_high_water_mib={:.2} budget_cap_mib={} inflated_mib={:.1}",
        options.threads,
        elapsed.as_secs_f64() * 1e3,
        first.map_or(0.0, |at| at.as_secs_f64() * 1e3),
        hit_bytes as f64 / 1048576.0,
        rss_before as f64 / 1024.0,
        hwm_before as f64 / 1024.0,
        status_kib(me, "VmHWM") as f64 / 1024.0,
        peak_workers.load(Ordering::Relaxed) as f64 / 1024.0,
        peak_one.load(Ordering::Relaxed) as f64 / 1024.0,
        book.search_high_water() as f64 / 1048576.0,
        CHAPTER_BYTE_BUDGET >> 20,
        inflated as f64 / 1048576.0,
    );
}
