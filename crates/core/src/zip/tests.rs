use super::*;
use crate::DEFAULT_LIMITS;
use crate::common::fixture;

fn index(book: &str) -> Result<ZipIndex, Error> {
    ZipIndex::parse(&File::open(fixture(book)).unwrap(), &DEFAULT_LIMITS)
}

fn expect_book_error(book: &str, words: &str) {
    match index(book) {
        Err(Error::Book(message)) => assert!(message.contains(words), "{book}: {message}"),
        Err(other) => panic!("{book}: wrong error class {other:?}"),
        Ok(_) => panic!("{book}: opened"),
    }
}

fn expect_limit_error(book: &str) {
    assert!(matches!(index(book), Err(Error::Limit(_))), "{book}");
}

fn read_named(book: &str, name: &str) -> Result<Vec<u8>, Error> {
    let file = File::open(fixture(book)).unwrap();
    let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
    let entry = zip.entry(&EntryName::parse(name).unwrap()).unwrap();
    zip.read_entry(&file, entry, &DEFAULT_LIMITS)
}

#[test]
fn names_that_escape_the_book_are_refused() {
    for name in [
        "../x",
        "a/../b",
        "/etc/passwd",
        "a\\b",
        "a\0b",
        "./a",
        "a//b",
        "C:/x",
    ] {
        assert!(EntryName::parse(name).is_err(), "{name:?}");
    }
    for name in ["mimetype", "OEBPS/Text/c1.xhtml", "OEBPS/", "a..b/c"] {
        assert!(EntryName::parse(name).is_ok(), "{name:?}");
    }
}

#[test]
fn traversal_bombs_are_refused_by_the_directory_parse() {
    expect_book_error("bomb-traversal-dotdot", "unsafe path");
    expect_book_error("bomb-traversal-absolute", "unsafe path");
    expect_book_error("bomb-traversal-backslash", "unsafe path");
    expect_book_error("bomb-traversal-nul", "unsafe path");
}

#[test]
fn overlapping_and_past_end_records_are_refused_without_reading_them() {
    expect_book_error("bomb-overlap-shared", "overlap");
    expect_book_error("bomb-overlap-nested", "overlap");
    expect_book_error("bomb-lying-extent", "past the end");
}

#[test]
fn entry_count_past_limit_is_refused_through_zip64_end_records() {
    expect_limit_error("bomb-huge-entry-count");
}

#[test]
fn declared_book_total_past_limit_is_refused_because_entries_cannot_outgrow_their_declaration() {
    expect_limit_error("bomb-book-total");
}

#[test]
fn zip64_end_records_parse_when_the_count_is_under_the_limit() {
    let path = fixture("bomb-huge-entry-count");
    let limits = Limits {
        max_entries: 300_000,
        max_book_inflated: u64::MAX,
        ..DEFAULT_LIMITS
    };
    let zip = ZipIndex::parse(&File::open(path).unwrap(), &limits).unwrap();
    assert!(zip.entries().len() > 200_001);
}

#[test]
fn inflate_enforces_declared_size_and_crc_on_bytes_produced() {
    let book = "bomb-lying-sizes";
    assert!(
        read_named(book, "OEBPS/c0.xhtml").is_ok(),
        "honest entry reads"
    );
    for ix in 1..=4 {
        let name = format!("OEBPS/c{ix}.xhtml");
        assert!(
            matches!(read_named(book, &name), Err(Error::Book(_))),
            "{name}"
        );
    }
}

#[test]
fn entry_past_limit_is_refused_on_load_not_on_open() {
    assert!(index("bomb-oversize-entry").is_ok());
    assert!(matches!(
        read_named("bomb-oversize-entry", "OEBPS/c0.xhtml"),
        Err(Error::Limit(_))
    ));
}

#[test]
fn inflate_stops_at_the_entry_limit_when_the_directory_lies_small() {
    let path = fixture("bomb-lying-sizes");
    let file = File::open(path).unwrap();
    let limits = Limits {
        max_entry_inflated: 512,
        ..DEFAULT_LIMITS
    };
    let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
    let entry = zip
        .entry(&EntryName::parse("OEBPS/c1.xhtml").unwrap())
        .unwrap();
    assert!(zip.read_entry(&file, entry, &limits).is_err());
}

struct Bytes(Vec<u8>);

impl ReadAt for Bytes {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| std::io::ErrorKind::UnexpectedEof)?;
        let slice = self
            .0
            .get(start..start.saturating_add(buf.len()))
            .ok_or(std::io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(slice);
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.0.len() as u64)
    }
}

fn with_comment(book: &str, comment: &[u8]) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture(book)).unwrap();
    let at = bytes.len() - 2;
    bytes[at..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    bytes
}

#[test]
fn a_zip_comment_is_tolerated_and_a_second_end_record_inside_it_is_refused() {
    let plain = with_comment("toc-none", b"a harmless comment");
    assert!(ZipIndex::parse(&Bytes(plain), &DEFAULT_LIMITS).is_ok());

    let mut fake_end = EOCD_SIG.to_vec();
    fake_end.extend_from_slice(&[0u8; 18]);
    let smuggled = with_comment("toc-none", &fake_end);
    assert!(matches!(
        ZipIndex::parse(&Bytes(smuggled), &DEFAULT_LIMITS),
        Err(Error::Book(_))
    ));

    let mut bytes = std::fs::read(fixture("toc-none")).unwrap();
    let end_record = bytes.split_off(bytes.len() - 22);
    let directory_at = le32(&end_record, 16) as usize;
    let inner_at = directory_at + CENTRAL_LEN;
    bytes[inner_at..inner_at + 4].copy_from_slice(EOCD_SIG);
    let comment_len = (bytes.len() + 22 - inner_at - 22) as u16;
    bytes[inner_at + 20..inner_at + 22].copy_from_slice(&comment_len.to_le_bytes());
    bytes.extend_from_slice(&end_record);
    match ZipIndex::parse(&Bytes(bytes), &DEFAULT_LIMITS) {
        Err(Error::Book(message)) => assert!(message.contains("more than one"), "{message}"),
        _ => panic!("an end record hidden in the directory was accepted"),
    }
}

#[test]
fn garbage_and_empty_files_give_readable_errors() {
    for bytes in [
        Vec::new(),
        vec![0u8; 21],
        vec![0u8; 5000],
        b"PK\x05\x06".to_vec(),
    ] {
        assert!(matches!(
            ZipIndex::parse(&Bytes(bytes), &DEFAULT_LIMITS),
            Err(Error::Book(_))
        ));
    }
}

fn one_stored_entry_zip(data: &[u8], archive_comment: &[u8]) -> Vec<u8> {
    let name = b"a.txt";
    let crc = crc32fast::hash(data);
    let mut out = Vec::new();
    out.extend_from_slice(LOCAL_SIG);
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&METHOD_STORED.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0x21, 0]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
    let cd_start = out.len() as u32;
    out.extend_from_slice(CENTRAL_SIG);
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&METHOD_STORED.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0x21, 0]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(name);
    let cd_len = out.len() as u32 - cd_start;
    out.extend_from_slice(EOCD_SIG);
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_start.to_le_bytes());
    out.extend_from_slice(&(archive_comment.len() as u16).to_le_bytes());
    out.extend_from_slice(archive_comment);
    out
}

fn smuggle_end_record_in_entry_data(archive_comment: &[u8]) -> Vec<u8> {
    let fake_at = 30 + 5;
    let mut fake = EOCD_SIG.to_vec();
    fake.extend_from_slice(&[0u8; 18]);
    let probe = one_stored_entry_zip(&fake, archive_comment);
    let reach = (probe.len() - (fake_at + 22)) as u16;
    fake[20..22].copy_from_slice(&reach.to_le_bytes());
    let bytes = one_stored_entry_zip(&fake, archive_comment);
    assert_eq!(&bytes[fake_at..fake_at + 4], EOCD_SIG);
    assert_eq!(
        fake_at + 22 + usize::from(le16(&bytes, fake_at + 20)),
        bytes.len()
    );
    bytes
}

fn expect_second_end_record_refused(bytes: Vec<u8>, case: &str) {
    match ZipIndex::parse(&Bytes(bytes), &DEFAULT_LIMITS) {
        Err(Error::Book(message)) if message.contains("more than one") => {}
        Err(other) => panic!("{case}: refused for another reason: {other:?}"),
        Ok(_) => panic!("{case}: a second end record was accepted"),
    }
}

#[test]
fn a_second_end_record_in_entry_data_is_refused_with_or_without_an_archive_comment() {
    expect_second_end_record_refused(smuggle_end_record_in_entry_data(b"x"), "with comment");
    expect_second_end_record_refused(smuggle_end_record_in_entry_data(b""), "without comment");
}

fn zip64_with_gap_record(gap_sig: &[u8; 4], archive_comment: &[u8]) -> Vec<u8> {
    let mut bytes = one_stored_entry_zip(b"hello", b"");
    let end_record = bytes.split_off(bytes.len() - 22);
    let cd_len = u64::from(le32(&end_record, 12));
    let cd_start = u64::from(le32(&end_record, 16));
    let eocd64_at = bytes.len() as u64;
    bytes.extend_from_slice(ZIP64_EOCD_SIG);
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&cd_len.to_le_bytes());
    bytes.extend_from_slice(&cd_start.to_le_bytes());
    let reach = (ZIP64_LOCATOR_LEN + EOCD_LEN) as usize + archive_comment.len();
    bytes.extend_from_slice(gap_sig);
    bytes.extend_from_slice(&[0u8; 16]);
    bytes.extend_from_slice(&(reach as u16).to_le_bytes());
    bytes.extend_from_slice(ZIP64_LOCATOR_SIG);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&eocd64_at.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(EOCD_SIG);
    bytes.extend_from_slice(&[0u8; 4]);
    bytes.extend_from_slice(&0xFFFFu16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFFu16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    bytes.extend_from_slice(&(archive_comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(archive_comment);
    bytes
}

#[test]
fn a_second_end_record_between_the_zip64_end_records_is_refused_with_or_without_an_archive_comment()
{
    let honest = ZipIndex::parse(&Bytes(zip64_with_gap_record(b"gap!", b"")), &DEFAULT_LIMITS)
        .expect("control: a ZIP64 book with bytes before its locator opens");
    assert_eq!(honest.entries().len(), 1);
    expect_second_end_record_refused(zip64_with_gap_record(EOCD_SIG, b"x"), "with comment");
    expect_second_end_record_refused(zip64_with_gap_record(EOCD_SIG, b""), "without comment");
}
