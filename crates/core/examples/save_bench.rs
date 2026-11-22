use jadeslip_core::{
    Book, BookKey, CommitPhases, CommitReport, DEFAULT_LIMITS, EntryName, Extracted, NewEntry,
    Reader,
};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const ROUNDS: usize = 7;
const NEIGHBOUR_SAMPLE: usize = 100;

struct Chapter {
    name: EntryName,
    bytes: Vec<u8>,
}

fn chapters(reader: &Reader<File>) -> Vec<Chapter> {
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter_map(|row| match reader.extract(row.ix) {
            Ok(Extracted::Document(bytes))
                if memchr::memmem::rfind(&bytes, b"</body>").is_some() =>
            {
                Some(Chapter {
                    name: row.href.clone()?,
                    bytes,
                })
            }
            _ => None,
        })
        .collect()
}

fn with_marker(bytes: &[u8], round: usize) -> Vec<u8> {
    let at = memchr::memmem::rfind(bytes, b"</body>").expect("a body end");
    [
        &bytes[..at],
        format!("<p>bench edit {round}</p>").as_bytes(),
        &bytes[at..],
    ]
    .concat()
}

fn deflate_all(chapters: &[Chapter], round: usize) -> Vec<NewEntry> {
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let per_thread = chapters.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let workers: Vec<_> = chapters
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|chapter| {
                            NewEntry::deflate(
                                chapter.name.clone(),
                                &with_marker(&chapter.bytes, round),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a deflate thread does not panic"))
            .collect()
    })
}

#[derive(Default)]
struct Series {
    prepare: Vec<Duration>,
    reports: Vec<CommitPhases>,
    bytes: u64,
    rewritten: u32,
}

fn stats(values: &mut [f64]) -> (f64, f64, f64) {
    values.sort_by(f64::total_cmp);
    (
        values[values.len() / 2],
        values[0],
        values[values.len() - 1],
    )
}

fn row(label: &str, mut values: Vec<f64>) {
    let (median, low, high) = stats(&mut values);
    println!("  {label:16} {median:9.2} ms  range {low:.2} to {high:.2}");
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn print_series(label: &str, series: &Series) {
    println!(
        "{label}: runs={} entries_rewritten={} bytes_written={}",
        series.reports.len(),
        series.rewritten,
        series.bytes
    );
    let of = |pick: fn(&CommitPhases) -> Duration| -> Vec<f64> {
        series
            .reports
            .iter()
            .map(|phases| ms(pick(phases)))
            .collect()
    };
    row(
        "deflate_prepare",
        series.prepare.iter().copied().map(ms).collect(),
    );
    row("commit_total", of(|p| p.total));
    row("backup_check", of(|p| p.backup));
    row("read_source", of(|p| p.read));
    row("hash", of(|p| p.hash));
    row("write_temp", of(|p| p.write));
    row("fsync_temp", of(|p| p.sync_temp));
    row("verify_reopen", of(|p| p.verify));
    row("ledger_pending", of(|p| p.ledger_pending));
    row("recheck_hash", of(|p| p.recheck));
    row("rename", of(|p| p.rename));
    row("fsync_dir", of(|p| p.sync_dir));
    row("ledger_head", of(|p| p.ledger_head));
    row("history", of(|p| p.history));
}

fn check(report: &CommitReport, path: &Path) {
    assert!(report.warnings.is_empty(), "a save warned");
    assert_eq!(
        fs::metadata(path).unwrap().len(),
        report.bytes_written,
        "bytes on disk match the report"
    );
    Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).expect("the saved book opens");
}

fn disk_counters() -> Option<(u64, u64)> {
    let device = std::env::var("SAVE_BENCH_DEVICE").ok()?;
    let stats = fs::read_to_string("/proc/diskstats").ok()?;
    stats.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.get(2) == Some(&device.as_str()))
            .then(|| Some((fields.get(9)?.parse().ok()?, fields.get(18)?.parse().ok()?)))?
    })
}

fn evict(path: &Path) {
    let status = Command::new("dd")
        .arg(format!("of={}", path.display()))
        .args([
            "oflag=nocache",
            "conv=notrunc,fdatasync",
            "count=0",
            "status=none",
        ])
        .status()
        .expect("dd runs");
    assert!(status.success(), "dd evicts the file");
}

fn bench_book(source: &Path, root: &Path) {
    let run_dir = root.join("run");
    let _ = fs::remove_dir_all(&run_dir);
    fs::create_dir_all(&run_dir).unwrap();
    let copy = run_dir.join("book.epub");
    fs::write(&copy, fs::read(source).unwrap()).unwrap();
    let data = run_dir.join("data");
    let opening = Instant::now();
    let book = Book::open(&copy, &data, BookKey(1), DEFAULT_LIMITS).unwrap();
    println!("book_open: {:.2} ms", ms(opening.elapsed()));
    let all = chapters(&book.read());
    let middle = all.len() / 2;
    println!(
        "file_size_bytes={} rows={} editable={}",
        fs::metadata(&copy).unwrap().len(),
        book.read().rows().len(),
        all.len()
    );

    evict(&copy);
    let first_started = Instant::now();
    let first = book
        .save_entries(vec![NewEntry::deflate(
            all[middle].name.clone(),
            &with_marker(&all[middle].bytes, 0),
        )])
        .unwrap();
    println!(
        "first_save_with_backup: total {:.2} ms, backup phase {:.2} ms, wall {:.2} ms",
        ms(first.phases.total),
        ms(first.phases.backup),
        ms(first_started.elapsed())
    );
    check(&first, &copy);

    let mut one = Series::default();
    let mut every = Series::default();
    let mut cold = Series::default();
    for round in 1..=ROUNDS {
        let started = Instant::now();
        let entry = NewEntry::deflate(
            all[middle].name.clone(),
            &with_marker(&all[middle].bytes, round),
        );
        one.prepare.push(started.elapsed());
        let before = disk_counters();
        let report = book.save_entries(vec![entry]).unwrap();
        if let (Some(before), Some(after)) = (before, disk_counters()) {
            println!(
                "device_during_one_chapter_save: written_MB={:.1} flushes={}",
                (after.0 - before.0) as f64 * 512.0 / 1e6,
                after.1 - before.1
            );
        }
        check(&report, &copy);
        one.bytes = report.bytes_written;
        one.rewritten = report.entries_rewritten;
        one.reports.push(report.phases);

        let started = Instant::now();
        let entries = deflate_all(&all, round);
        every.prepare.push(started.elapsed());
        let report = book.save_entries(entries).unwrap();
        check(&report, &copy);
        every.bytes = report.bytes_written;
        every.rewritten = report.entries_rewritten;
        every.reports.push(report.phases);

        let started = Instant::now();
        let entry = NewEntry::deflate(
            all[middle].name.clone(),
            &with_marker(&all[middle].bytes, round + ROUNDS),
        );
        cold.prepare.push(started.elapsed());
        evict(&copy);
        let report = book.save_entries(vec![entry]).unwrap();
        check(&report, &copy);
        cold.bytes = report.bytes_written;
        cold.rewritten = report.entries_rewritten;
        cold.reports.push(report.phases);
    }
    print_series("one_chapter", &one);
    print_series("one_chapter_cold_source", &cold);
    print_series("all_chapters", &every);

    let reader = book.read();
    let neighbours: Vec<_> = reader
        .rows()
        .iter()
        .filter(|row| row.linear_ordinal.is_some())
        .skip(middle)
        .take(NEIGHBOUR_SAMPLE)
        .map(|row| row.ix)
        .collect();
    let (mut cold, mut warm, mut failed) = (Vec::new(), Vec::new(), 0);
    for ix in &neighbours {
        evict(&copy);
        let started = Instant::now();
        let first = reader.render(*ix);
        let cold_ms = ms(started.elapsed());
        let started = Instant::now();
        let second = reader.render(*ix);
        let warm_ms = ms(started.elapsed());
        match (first, second) {
            (Ok(a), Ok(b)) if a.xhtml == b.xhtml && !a.xhtml.is_empty() => {
                cold.push(cold_ms);
                warm.push(warm_ms);
            }
            _ => failed += 1,
        }
    }
    println!("neighbour_render: chapters={} failed={failed}", cold.len());
    row("render_cold_file", cold);
    row("render_warm_file", warm);
    drop(reader);
    drop(book);
    fs::remove_dir_all(&run_dir).unwrap();
}

fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("a scratch folder on the target disk"),
    );
    fs::create_dir_all(&root).unwrap();
    for path in std::env::args().skip(2) {
        println!(
            "=== {}",
            Path::new(&path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .rsplit('-')
                .next()
                .unwrap()
        );
        bench_book(Path::new(&path), &root);
    }
}
