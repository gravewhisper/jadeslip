use jadeslip_core::{DEFAULT_LIMITS, Error, Extracted, Reader};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn real_books() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "epub"));
    paths.sort();
    paths
}

fn id_of(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy();
    stem.rsplit('-').next().unwrap_or("").to_owned()
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test real_books -- --ignored --nocapture"]
fn every_real_book_opens_and_scans_every_spine_item() {
    let books = real_books();
    assert!(!books.is_empty(), "fixtures/real has books");
    println!(
        "book     size_MB  rows linear   open_ms  inflate_ms model_ms  inflated_MB visible_MB MB/s  failed limit"
    );
    let mut total_failures = 0;
    for path in &books {
        let size = std::fs::metadata(path).unwrap().len();
        let started = Instant::now();
        let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS)
            .unwrap_or_else(|error| panic!("{} refused: class {}", id_of(path), class(&error)));
        let open_ms = started.elapsed().as_secs_f64() * 1e3;

        let started = Instant::now();
        let mut inflated = 0u64;
        for row in reader.rows() {
            match reader.extract(row.ix) {
                Ok(Extracted::Document(bytes)) => inflated += bytes.len() as u64,
                Ok(Extracted::Image { .. }) | Err(_) => {}
            }
        }
        let inflate_ms = started.elapsed().as_secs_f64() * 1e3;

        let started = Instant::now();
        let (mut visible, mut failed, mut limit) = (0u64, 0, 0);
        for row in reader.rows() {
            match reader.visible_text(row.ix) {
                Ok(text) => visible += text.as_str().len() as u64,
                Err(Error::Limit(_)) => limit += 1,
                Err(_) => failed += 1,
            }
        }
        let full_ms = started.elapsed().as_secs_f64() * 1e3;
        total_failures += failed + limit;
        println!(
            "{:8} {:7.1} {:5} {:6} {:9.2} {:10.1} {:8.1} {:11.1} {:10.1} {:5.0} {:6} {:5}",
            id_of(path),
            size as f64 / 1e6,
            reader.rows().len(),
            reader.summary().linear_chapters,
            open_ms,
            inflate_ms,
            full_ms - inflate_ms,
            inflated as f64 / 1e6,
            visible as f64 / 1e6,
            inflated as f64 / 1e6 / (full_ms / 1e3),
            failed,
            limit,
        );
    }
    assert_eq!(total_failures, 0);
}

fn class(error: &Error) -> &'static str {
    match error {
        Error::Limit(_) => "limit",
        Error::Book(_) => "book",
        Error::Io(_) => "io",
        _ => "other",
    }
}

#[test]
#[ignore = "repeatable open timing, alternating books: cargo test --release -p jadeslip-core --test real_books open_timing -- --ignored --nocapture"]
fn open_timing_alternating_the_largest_real_book_and_the_large_generated_book() {
    let serial = real_books()
        .into_iter()
        .max_by_key(|path| {
            Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS)
                .map_or(0, |reader| reader.rows().len())
        })
        .unwrap();
    let generated =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated/large-h1-div-br.epub");
    let mut samples = [Vec::new(), Vec::new()];
    let mut rows = [0, 0];
    for round in 0..41 {
        for (which, path) in [&serial, &generated].into_iter().enumerate() {
            let started = Instant::now();
            let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
            let elapsed = started.elapsed();
            rows[which] = reader.rows().len();
            if round > 0 {
                samples[which].push(elapsed.as_secs_f64() * 1e3);
            }
        }
    }
    for (which, name) in ["largest real book", "large-h1-div-br"]
        .into_iter()
        .enumerate()
    {
        samples[which].sort_by(f64::total_cmp);
        let runs = &samples[which];
        println!(
            "{name}: {} rows, warm open median {:.2} ms, min {:.2}, max {:.2}, {} runs",
            rows[which],
            runs[runs.len() / 2],
            runs[0],
            runs[runs.len() - 1],
            runs.len()
        );
    }
}
