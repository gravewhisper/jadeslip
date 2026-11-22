use jadeslip_core::{DEFAULT_LIMITS, Reader};
use std::fs::File;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("a path to an EPUB");
    let started = Instant::now();
    let reader =
        Reader::open(File::open(&path).expect("file opens"), DEFAULT_LIMITS).expect("book opens");
    let open_ms = started.elapsed().as_secs_f64() * 1e3;
    println!("{open_ms:.2} {}", reader.rows().len());
}
