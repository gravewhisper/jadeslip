use jadeslip_core::{
    Book, BookKey, CancelToken, Case, ChapterIx, DEFAULT_LIMITS, EntryName, Extracted, NewEntry,
    Query, Reader, ReplaceRequest, SearchOptions, SearchScope,
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const ROUNDS: usize = 5;
const FEW_CHAPTERS: u32 = 20;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn middle(values: &mut [f64]) -> (f64, f64, f64) {
    values.sort_by(f64::total_cmp);
    (
        values[values.len() / 2],
        values[0],
        values[values.len() - 1],
    )
}

fn peak_kib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        .unwrap()
}

fn rss_kib(status: &str) -> u64 {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

fn tree_rss_kib() -> u64 {
    let own = rss_kib(&fs::read_to_string("/proc/self/status").unwrap_or_default());
    let mut children = 0;
    for task in fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
    {
        let listed = fs::read_to_string(task.path().join("children")).unwrap_or_default();
        for pid in listed.split_whitespace() {
            children +=
                rss_kib(&fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default());
        }
    }
    own + children
}

fn cpu_ms() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let of = |time: libc::timeval| time.tv_sec as f64 * 1e3 + time.tv_usec as f64 / 1e3;
    of(usage.ru_utime) + of(usage.ru_stime)
}

struct TreeSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: std::thread::JoinHandle<()>,
}

impl TreeSampler {
    fn start() -> TreeSampler {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let (stopping, highest) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                highest.fetch_max(tree_rss_kib(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        TreeSampler { stop, peak, thread }
    }

    fn finish(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap();
        self.peak.load(Ordering::Relaxed)
    }
}

fn reset_peak() {
    let _ = fs::write("/proc/self/clear_refs", "5");
}

fn options() -> SearchOptions {
    let worker = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("jadeslip-worker");
    assert!(worker.exists(), "build the worker first");
    SearchOptions {
        worker_exe: Some(worker),
        ..SearchOptions::default()
    }
}

fn literal(term: &str, replacement: &str) -> ReplaceRequest {
    ReplaceRequest::new(
        Query::Literal {
            text: term.to_owned(),
            case: Case::Sensitive,
            whole_word: true,
        },
        replacement.to_owned(),
    )
}

fn rare_word(book: &Book) -> String {
    let reader = book.read();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for row in reader.rows().iter().filter(|row| !row.is_nav) {
        let Ok(text) = reader.visible_text(row.ix) else {
            continue;
        };
        let mut seen: Vec<&str> = text
            .as_str()
            .split(|ch: char| !ch.is_alphabetic())
            .filter(|word| word.len() >= 8)
            .collect();
        seen.sort_unstable();
        seen.dedup();
        for word in seen {
            *counts.entry(word.to_owned()).or_default() += 1;
        }
    }
    let mut candidates: Vec<(u32, String)> = counts
        .into_iter()
        .filter(|(_, chapters)| (FEW_CHAPTERS / 2..=FEW_CHAPTERS).contains(chapters))
        .map(|(word, chapters)| (chapters, word))
        .collect();
    candidates.sort();
    candidates.pop().expect("a rare word").1
}

#[derive(Default)]
struct Outcome {
    preview: Vec<f64>,
    apply: Vec<f64>,
    cpu: Vec<f64>,
    changes: u64,
    chapters: u32,
    peak: u64,
    tree: u64,
    phases: Vec<[f64; 5]>,
    one_thread: f64,
}

struct Scenario {
    name: &'static str,
    request: ReplaceRequest,
}

fn bench(source: &Path, label: &str, root: &Path) {
    let opts = options();
    let cancel = CancelToken::default();
    let probe_dir = root.join("probe");
    let _ = fs::remove_dir_all(&probe_dir);
    fs::create_dir_all(&probe_dir).unwrap();
    let probe = probe_dir.join("book.epub");
    fs::write(&probe, fs::read(source).unwrap()).unwrap();
    let opened = Book::open(&probe, &probe_dir.join("data"), BookKey(1), DEFAULT_LIMITS).unwrap();
    let word = rare_word(&opened);
    drop(opened);
    let scenarios = [
        Scenario {
            name: "few-literal",
            request: literal(&word, "QXZQXZ"),
        },
        Scenario {
            name: "most-literal",
            request: literal("the", "QXZ"),
        },
        Scenario {
            name: "most-regex",
            request: ReplaceRequest::new(
                Query::Regex {
                    pattern: r"\b(\w+)ing\b(?= )".to_owned(),
                    case: Case::Sensitive,
                },
                "${1}ING".to_owned(),
            ),
        },
    ];
    let mut results: Vec<Outcome> = scenarios.iter().map(|_| Outcome::default()).collect();
    for _round in 0..ROUNDS {
        for (scenario, result) in scenarios.iter().zip(&mut results) {
            let dir = root.join("run");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let copy = dir.join("book.epub");
            fs::write(&copy, fs::read(source).unwrap()).unwrap();
            let book = Book::open(&copy, &dir.join("data"), BookKey(2), DEFAULT_LIMITS).unwrap();
            reset_peak();
            let sampler = TreeSampler::start();
            let started = Instant::now();
            let shown = book
                .preview_replace(
                    &scenario.request,
                    SearchScope::Whole,
                    5_000,
                    &opts,
                    &cancel,
                    &mut |_| {},
                )
                .unwrap();
            result.preview.push(ms(started.elapsed()));
            assert_eq!(shown.limited, 0, "{} limited", scenario.name);
            let cpu_before = cpu_ms();
            let started = Instant::now();
            let applied = book
                .apply_replace(&shown, &[], &opts, &cancel, &mut |_, _| {})
                .unwrap();
            result.apply.push(ms(started.elapsed()));
            result.cpu.push(cpu_ms() - cpu_before);
            result.tree = result.tree.max(sampler.finish());
            result.peak = result.peak.max(peak_kib());
            assert_eq!(applied.changes, shown.changes);
            assert_eq!(
                applied.report.entries_rewritten,
                applied.chapters + u32::from(applied.report.mimetype_normalised)
            );
            Reader::open(File::open(&copy).unwrap(), DEFAULT_LIMITS).unwrap();
            let phases = &applied.report.phases;
            result.phases.push([
                ms(phases.backup),
                ms(phases.write),
                ms(phases.sync_temp),
                ms(phases.verify),
                ms(phases.total),
            ]);
            result.changes = applied.changes;
            result.chapters = applied.chapters;
        }
    }
    for (scenario, result) in scenarios.iter().zip(&mut results) {
        let dir = root.join("run");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("book.epub");
        fs::write(&copy, fs::read(source).unwrap()).unwrap();
        let book = Book::open(&copy, &dir.join("data"), BookKey(2), DEFAULT_LIMITS).unwrap();
        let single = SearchOptions {
            threads: 1,
            chapter_deadline: Duration::from_secs(60),
            book_deadline: Duration::from_secs(600),
            ..opts.clone()
        };
        let shown = book
            .preview_replace(
                &scenario.request,
                SearchScope::Whole,
                1,
                &single,
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        let started = Instant::now();
        book.apply_replace(&shown, &[], &single, &cancel, &mut |_, _| {})
            .unwrap();
        result.one_thread = ms(started.elapsed());
    }
    for (scenario, mut result) in scenarios.iter().zip(results) {
        let (p, p_low, p_high) = middle(&mut result.preview);
        let (a, a_low, a_high) = middle(&mut result.apply);
        let (cpu, _, _) = middle(&mut result.cpu);
        let columns: Vec<String> = ["backup", "write", "fsync", "verify", "total"]
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let mut values: Vec<f64> = result.phases.iter().map(|phases| phases[at]).collect();
                format!("{name}={:.0}", middle(&mut values).0)
            })
            .collect();
        println!(
            "{label} {}: changes={} chapters={} preview {p:.0} ms [{p_low:.0}..{p_high:.0}] apply {a:.0} ms [{a_low:.0}..{a_high:.0}] one_thread_apply={:.0} apply_cpu={cpu:.0} ms parent_peak={:.0} MiB tree_peak={:.0} MiB commit {}",
            scenario.name,
            result.changes,
            result.chapters,
            result.one_thread,
            result.peak as f64 / 1024.0,
            result.tree as f64 / 1024.0,
            columns.join(" ")
        );
    }
}

fn chapters_of(reader: &Reader<File>) -> Vec<(EntryName, Vec<u8>)> {
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter_map(|row| match reader.extract(ChapterIx(row.ix.0)) {
            Ok(Extracted::Document(bytes)) => Some((row.href.clone()?, bytes)),
            _ => None,
        })
        .collect()
}

fn deflate_gap(source: &Path, label: &str) {
    let reader = Reader::open(File::open(source).unwrap(), DEFAULT_LIMITS).unwrap();
    let chapters = chapters_of(&reader);
    let bytes: u64 = chapters.iter().map(|(_, body)| body.len() as u64).sum();
    let thread_counts = [1usize, 4, 8, 16];
    let mut series: Vec<Vec<f64>> = vec![Vec::new(); thread_counts.len()];
    let mut out_bytes = 0u64;
    for _round in 0..ROUNDS {
        for (at, threads) in thread_counts.iter().enumerate() {
            let per = chapters.len().div_ceil(*threads);
            let started = Instant::now();
            let sizes: u64 = std::thread::scope(|scope| {
                let workers: Vec<_> = chapters
                    .chunks(per)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|(name, body)| {
                                    NewEntry::deflate(name.clone(), body);
                                    body.len() as u64
                                })
                                .sum::<u64>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|worker| worker.join().unwrap())
                    .sum()
            });
            series[at].push(ms(started.elapsed()));
            out_bytes = sizes;
        }
    }
    assert_eq!(out_bytes, bytes);
    for (at, threads) in thread_counts.iter().enumerate() {
        let (median, low, high) = middle(&mut series[at]);
        println!(
            "{label} deflate threads={threads}: chapters={} MiB_in={:.1} {median:.0} ms [{low:.0}..{high:.0}]",
            chapters.len(),
            bytes as f64 / (1 << 20) as f64
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    for source in &args[1..] {
        let path = PathBuf::from(source);
        let label = path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('-')
            .next()
            .unwrap()
            .to_owned();
        if args.len() > 4 {
            deflate_gap(&path, &label);
        }
        bench(&path, &label, &root);
    }
}
