mod common;

use common::{books, fixture};
use jadeslip_core::{ChapterIx, Limits, ReadAt, Reader, RecordingReader};
use miniz_oxide::inflate::decompress_to_vec_with_limit;
use std::collections::BTreeMap;
use std::fs::File;
use std::ops::Range;

struct Layout {
    directory_to_eof: Range<u64>,
    records: BTreeMap<String, Range<u64>>,
    bytes: Vec<u8>,
}

fn le16(bytes: &[u8], at: usize) -> u64 {
    u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn le32(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
    ]))
}

fn layout(book: &str) -> Layout {
    let bytes = std::fs::read(fixture(book)).unwrap();
    let eocd = bytes.len() - 22;
    assert_eq!(
        &bytes[eocd..eocd + 4],
        b"PK\x05\x06",
        "{book} has no trailing comment"
    );
    let count = le16(&bytes, eocd + 10);
    let cd_start = le32(&bytes, eocd + 16);
    let mut at = cd_start as usize;
    let mut starts = Vec::new();
    for _ in 0..count {
        assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
        let name_len = le16(&bytes, at + 28) as usize;
        let name = String::from_utf8_lossy(&bytes[at + 46..at + 46 + name_len]).into_owned();
        starts.push((le32(&bytes, at + 42), name));
        at += 46 + name_len + le16(&bytes, at + 30) as usize + le16(&bytes, at + 32) as usize;
    }
    starts.sort();
    let mut records = BTreeMap::new();
    for (ix, (start, name)) in starts.iter().enumerate() {
        let end = starts.get(ix + 1).map_or(cd_start, |next| next.0);
        records.insert(name.clone(), *start..end);
    }
    Layout {
        directory_to_eof: cd_start..bytes.len() as u64,
        records,
        bytes,
    }
}

fn entry_text(layout: &Layout, name: &str) -> String {
    let record = &layout.records[name];
    let at = record.start as usize;
    let header = &layout.bytes[at..at + 30];
    let method = le16(header, 8);
    let compressed = le32(header, 18) as usize;
    let data = at + 30 + le16(header, 26) as usize + le16(header, 28) as usize;
    let body = &layout.bytes[data..data + compressed];
    let raw = if method == 0 {
        body.to_vec()
    } else {
        decompress_to_vec_with_limit(body, 1 << 30).unwrap()
    };
    String::from_utf8_lossy(&raw).into_owned()
}

fn allowed_regions(layout: &Layout) -> Vec<Range<u64>> {
    let mut allowed = vec![layout.directory_to_eof.clone()];
    allowed.push(layout.records["META-INF/container.xml"].clone());
    let container = entry_text(layout, "META-INF/container.xml");
    let declared = container
        .split("full-path=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("container names an OPF");
    let opf = match layout.records.get(declared) {
        Some(record) => record.clone(),
        None => {
            let mut sole = layout
                .records
                .iter()
                .filter(|(name, _)| name.ends_with(".opf"));
            let (_, record) = sole.next().expect("book has an OPF");
            assert!(sole.next().is_none());
            record.clone()
        }
    };
    allowed.push(opf);
    allowed
}

fn stray_read(reads: &[Range<u64>], allowed: &[Range<u64>]) -> Option<Range<u64>> {
    reads
        .iter()
        .find(|read| {
            !allowed
                .iter()
                .any(|region| region.start <= read.start && read.end <= region.end)
        })
        .cloned()
}

fn reads_past_end_record_scan(recording: &RecordingReader<File>, book: &str) -> Vec<Range<u64>> {
    let len = std::fs::metadata(fixture(book)).unwrap().len();
    let end_record_scan = len.saturating_sub(22 + 0xFFFF)..len;
    recording
        .ranges()
        .into_iter()
        .filter(|read| *read != end_record_scan)
        .collect()
}

const OPENABLE_LIMITS: Limits = jadeslip_core::DEFAULT_LIMITS;

struct OpenTrace {
    reads: Vec<Range<u64>>,
    spine_records: Vec<Range<u64>>,
}

fn opens_with_recording(book: &str) -> Option<OpenTrace> {
    let recording = RecordingReader::new(File::open(fixture(book)).unwrap());
    let reader = Reader::open(&recording, OPENABLE_LIMITS).ok()?;
    let reads = reads_past_end_record_scan(&recording, book);
    let spine_records = reader
        .rows()
        .iter()
        .map(|row| reader.entry_record_range(row.ix))
        .filter(|record| !record.is_empty())
        .collect();
    Some(OpenTrace {
        reads,
        spine_records,
    })
}

#[test]
fn open_reads_no_chapter_bytes() {
    let mut proved = 0;
    for book in books() {
        let Some(OpenTrace {
            reads,
            spine_records: records,
        }) = opens_with_recording(book)
        else {
            continue;
        };
        assert!(!reads.is_empty(), "{book}: recorder saw the open");
        let layout = layout(book);
        let allowed = allowed_regions(&layout);
        assert_eq!(
            stray_read(&reads, &allowed),
            None,
            "{book}: read outside the allowed regions"
        );
        for record in &records {
            assert!(
                reads
                    .iter()
                    .all(|read| read.end <= record.start || read.start >= record.end),
                "{book}: open touched spine record {record:?}"
            );
        }
        if book.starts_with("large-") {
            assert!(
                records.len() >= 6_000,
                "{book}: {} spine records",
                records.len()
            );
            let total: u64 = reads.iter().map(|read| read.end - read.start).sum();
            println!(
                "{book}: open made {} reads totalling {total} of {} file bytes",
                reads.len(),
                layout.bytes.len()
            );
        }
        proved += 1;
    }
    assert!(proved >= 20, "only {proved} fixtures opened");
}

#[test]
fn the_read_check_catches_a_deliberate_chapter_read() {
    for book in ["large-h1-p", "large-h1-div-br", "toc-none"] {
        let recording = RecordingReader::new(File::open(fixture(book)).unwrap());
        let reader = Reader::open(&recording, OPENABLE_LIMITS).unwrap();
        let layout = layout(book);
        let allowed = allowed_regions(&layout);
        assert_eq!(
            stray_read(&reads_past_end_record_scan(&recording, book), &allowed),
            None,
            "{book}: clean before the read"
        );
        let chapter = reader
            .rows()
            .iter()
            .position(|row| !row.is_nav && row.rev.is_some())
            .expect("a readable chapter");
        reader.extract(ChapterIx(chapter as u32)).unwrap();
        let caught = stray_read(&reads_past_end_record_scan(&recording, book), &allowed);
        assert!(caught.is_some(), "{book}: a chapter read went unnoticed");
    }
}

#[test]
fn the_recorder_records_every_read_it_serves() {
    let recording = RecordingReader::new(File::open(fixture("toc-none")).unwrap());
    let mut buffer = [0u8; 4];
    recording.read_exact_at(&mut buffer, 10).unwrap();
    assert_eq!(recording.ranges(), vec![10..14]);
}
