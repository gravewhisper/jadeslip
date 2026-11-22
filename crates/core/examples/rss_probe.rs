use jadeslip_core::{DEFAULT_LIMITS, Error, Reader};
use std::fs::File;
use std::time::Instant;

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))
                .and_then(|value| value.trim().trim_end_matches(" kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

fn label(error: &Error) -> &'static str {
    match error {
        Error::Limit(_) => "limit",
        Error::Book(_) => "book",
        Error::Io(_) => "io",
        _ => "other",
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("a path to an EPUB");
    let started = Instant::now();
    let reader = match Reader::open(File::open(&path).expect("file opens"), DEFAULT_LIMITS) {
        Ok(reader) => reader,
        Err(error) => {
            println!(
                "open refused ({}) in {:?}, peak RSS {} KiB",
                label(&error),
                started.elapsed(),
                peak_rss_kib()
            );
            return;
        }
    };
    let (mut loaded, mut limit, mut book, mut visible) = (0u64, 0u64, 0u64, 0u64);
    for row in reader.rows() {
        match reader.visible_text(row.ix) {
            Ok(text) => {
                loaded += 1;
                visible += text.as_str().len() as u64;
            }
            Err(Error::Limit(_)) => limit += 1,
            Err(_) => book += 1,
        }
    }
    println!(
        "opened, {loaded} chapters loaded, {limit} over a limit, {book} refused, {visible} visible bytes, {:?}, peak RSS {} KiB",
        started.elapsed(),
        peak_rss_kib()
    );
}
