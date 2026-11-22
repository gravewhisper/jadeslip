use super::*;
use crate::backup::hash_of;
use crate::common::fixture;
use crate::zip::{EntryFate, RecordingReader, RevertKind};
use crate::{DEFAULT_LIMITS, Extracted, PointId, PointKind, Pruned};
use seam::{LedgerWrite, install};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU32;
use std::time::Duration;

const SMALL: &str = "images-mixed";
const KEY: BookKey = BookKey(0x5150);

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/commit-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("books")).unwrap();
        fs::create_dir_all(dir.join("data")).unwrap();
        Scratch(dir)
    }

    fn book_path(&self) -> PathBuf {
        self.0.join("books/book.epub")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn place(&self, source: &Path) -> PathBuf {
        let path = self.book_path();
        fs::write(&path, fs::read(source).unwrap()).unwrap();
        path
    }

    fn open(&self) -> Book {
        Book::open(&self.book_path(), &self.data(), KEY, DEFAULT_LIMITS).unwrap()
    }

    fn slot_dirs(&self) -> Vec<PathBuf> {
        let dir = self.data().join(format!("books/{:032x}/backups", KEY.0));
        fs::read_dir(dir).map_or(Vec::new(), |entries| {
            entries.map(|entry| entry.unwrap().path()).collect()
        })
    }

    fn stray_temps(&self) -> usize {
        let beside = fs::read_dir(self.0.join("books"))
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        let in_slots: usize = self
            .slot_dirs()
            .iter()
            .map(|dir| {
                fs::read_dir(dir)
                    .unwrap()
                    .filter(|entry| {
                        entry
                            .as_ref()
                            .unwrap()
                            .file_name()
                            .to_string_lossy()
                            .ends_with(".tmp")
                    })
                    .count()
            })
            .sum();
        beside + in_slots
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha(bytes: &[u8]) -> FileHash {
    FileHash(Sha256::digest(bytes).into())
}

fn first_document(reader: &Reader<File>) -> ChapterTarget {
    let row = reader
        .rows()
        .iter()
        .find(|row| {
            row.linear_ordinal.is_some()
                && matches!(reader.extract(row.ix), Ok(Extracted::Document(_)))
        })
        .expect("a document chapter");
    let Ok(Extracted::Document(bytes)) = reader.extract(row.ix) else {
        unreachable!()
    };
    ChapterTarget {
        name: row.href.clone().unwrap(),
        bytes,
    }
}

struct ChapterTarget {
    name: EntryName,
    bytes: Vec<u8>,
}

fn with_marker(bytes: &[u8], marker: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let at = text.rfind("</body>").expect("a body end");
    format!("{}<p>{marker}</p>{}", &text[..at], &text[at..]).into_bytes()
}

fn edit_first_chapter(book: &Book, marker: &str) -> CommitReport {
    let target = first_document(&book.read());
    let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, marker));
    book.save_entries(vec![entry]).unwrap()
}

fn zero_offset(row: &[u8]) -> Vec<u8> {
    let mut row = row.to_vec();
    row[42..46].fill(0);
    row
}

fn assert_only_named_changed(
    old: &Path,
    new: &Path,
    changed: &BTreeSet<&str>,
    exempt_mimetype: bool,
) -> usize {
    let old_file = File::open(old).unwrap();
    let new_file = File::open(new).unwrap();
    let old_zip = ZipIndex::parse(&old_file, &DEFAULT_LIMITS).unwrap();
    let new_zip = ZipIndex::parse(&new_file, &DEFAULT_LIMITS).unwrap();
    let mut compared = 0;
    for entry in old_zip.entries() {
        let name = entry.name.as_str();
        if changed.contains(name) || (exempt_mimetype && name == "mimetype") {
            continue;
        }
        let new_entry = new_zip.entry(&entry.name).expect("kept entry");
        let mut old_bytes = vec![0; (entry.record.end - entry.record.start) as usize];
        let mut new_bytes = vec![0; (new_entry.record.end - new_entry.record.start) as usize];
        old_file
            .read_exact_at(&mut old_bytes, entry.record.start)
            .unwrap();
        new_file
            .read_exact_at(&mut new_bytes, new_entry.record.start)
            .unwrap();
        assert!(old_bytes == new_bytes, "record bytes differ");
        assert_eq!(
            zero_offset(entry.raw_central_for_test()),
            zero_offset(new_entry.raw_central_for_test()),
            "central row differs beyond its offset"
        );
        compared += 1;
    }
    compared
}

fn python_records(old: &Path, new: &Path, changed: &[&str], scratch: &Scratch) -> String {
    let list = scratch.0.join("changed.txt");
    fs::write(&list, changed.join("\n")).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/zip_records.py");
    let output = Command::new("python3")
        .arg("-I")
        .arg(script)
        .args([old, new, &list])
        .output()
        .unwrap();
    assert!(output.status.success(), "zip_records.py failed");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn one_chapter_save_leaves_every_other_record_byte_equal_on_small_and_large_books() {
    for name in [SMALL, "large-h1-p", "large-h1-div-br"] {
        let scratch = Scratch::new("byte-equal");
        let source = fixture(name);
        scratch.place(&source);
        let book = scratch.open();
        let total = book.read().zip().entries().len();
        let target = first_document(&book.read());
        let target_name = target.name.as_str().to_owned();
        let report = edit_first_chapter(&book, "edited");
        assert_eq!(report.entries_rewritten, 1);
        assert_eq!(report.entries_copied as usize, total - 1);
        let changed: BTreeSet<&str> = [target_name.as_str()].into();
        let compared = assert_only_named_changed(&source, &scratch.book_path(), &changed, false);
        assert_eq!(compared, total - 1, "{name}");
        let python = python_records(&source, &scratch.book_path(), &[&target_name], &scratch);
        assert!(python.contains("differ_unexpected=0"), "{python}");
        assert!(python.contains("differ_expected=1"), "{python}");
        assert!(python.contains("crc_failed=0"), "{python}");
        assert!(python.contains("mimetype_first_stored=True"), "{python}");
        let on_disk = fs::read(scratch.book_path()).unwrap();
        assert_eq!(report.bytes_written, on_disk.len() as u64);
    }
}

#[test]
fn saved_chapter_reads_back_and_old_snapshot_keeps_the_old_bytes() {
    let scratch = Scratch::new("snapshot");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let old_snapshot = book.read();
    let before = first_document(&old_snapshot);
    edit_first_chapter(&book, "edited");
    let after = first_document(&book.read());
    assert_eq!(after.bytes, with_marker(&before.bytes, "edited"));
    assert_eq!(first_document(&old_snapshot).bytes, before.bytes);
    let fresh = Reader::open(File::open(scratch.book_path()).unwrap(), DEFAULT_LIMITS).unwrap();
    assert_eq!(first_document(&fresh).bytes, after.bytes);
}

fn rebuilt_with_mimetype(mimetype_method: u16, mimetype_first: bool, into: &Path) {
    let source = File::open(fixture(SMALL)).unwrap();
    let zip = ZipIndex::parse(&source, &DEFAULT_LIMITS).unwrap();
    let mut writer = jadeslip_fixtures::zip::ZipWriter::new(File::create(into).unwrap());
    let mimetype = b"application/epub+zip";
    if mimetype_first {
        writer.add(b"mimetype", mimetype, mimetype_method).unwrap();
    }
    for entry in zip.entries() {
        if entry.name.as_str() != "mimetype" {
            let bytes = zip.read_entry(&source, entry, &DEFAULT_LIMITS).unwrap();
            writer
                .add(
                    entry.name.as_str().as_bytes(),
                    &bytes,
                    jadeslip_fixtures::zip::DEFLATE,
                )
                .unwrap();
        }
    }
    if !mimetype_first {
        writer.add(b"mimetype", mimetype, mimetype_method).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn every_nonconforming_mimetype_shape_is_normalised_and_a_conforming_one_is_not() {
    use jadeslip_fixtures::zip::{DEFLATE, STORED};
    for (method, first, expect_normalised) in [
        (STORED, false, true),
        (DEFLATE, false, true),
        (DEFLATE, true, true),
        (STORED, true, false),
    ] {
        let scratch = Scratch::new("mimetype-shape");
        rebuilt_with_mimetype(method, first, &scratch.book_path());
        let book = scratch.open();
        let report = edit_first_chapter(&book, "edited");
        assert_eq!(
            report.mimetype_normalised, expect_normalised,
            "method {method} first {first}"
        );
        let file = File::open(scratch.book_path()).unwrap();
        let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
        assert!(
            zip.mimetype_is_first_and_stored(),
            "method {method} first {first}"
        );
        let entry = zip.entry(&EntryName::parse("mimetype").unwrap()).unwrap();
        assert_eq!(
            zip.read_entry(&file, entry, &DEFAULT_LIMITS).unwrap(),
            b"application/epub+zip"
        );
    }
}

#[test]
fn a_book_with_mimetype_out_of_place_comes_out_first_and_stored_and_the_rest_byte_equal() {
    let scratch = Scratch::new("mimetype");
    let source = fixture("mimetype-not-first");
    scratch.place(&source);
    let book = scratch.open();
    let total = book.read().zip().entries().len();
    let target_name = first_document(&book.read()).name;
    edit_first_chapter(&book, "edited");
    let zip_file = File::open(scratch.book_path()).unwrap();
    let zip = ZipIndex::parse(&zip_file, &DEFAULT_LIMITS).unwrap();
    assert!(zip.mimetype_is_first_and_stored());
    let changed: BTreeSet<&str> = [target_name.as_str()].into();
    let compared = assert_only_named_changed(&source, &scratch.book_path(), &changed, true);
    assert_eq!(compared, total - 2);
}

struct DropAndAdd {
    drop_name: EntryName,
    add: Option<NewEntry>,
}

impl Changes for DropAndAdd {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error> {
        Ok(if entry.name == self.drop_name {
            EntryFate::Drop
        } else {
            EntryFate::RawCopy
        })
    }

    fn added(&mut self) -> Vec<NewEntry> {
        self.add.take().into_iter().collect()
    }
}

#[test]
fn a_drop_and_an_add_keep_the_rest_byte_equal() {
    let scratch = Scratch::new("drop-add");
    let source = fixture(SMALL);
    scratch.place(&source);
    let book = scratch.open();
    let reader = book.read();
    let unused = reader
        .zip()
        .entries()
        .iter()
        .find(|entry| entry.name.as_str().ends_with(".png"))
        .expect("an image entry")
        .name
        .clone();
    let mut changes = DropAndAdd {
        drop_name: unused.clone(),
        add: Some(NewEntry::deflate(
            EntryName::parse("OEBPS/added-é.txt").unwrap(),
            b"added",
        )),
    };
    let report = book.save_changes(&mut changes).unwrap();
    assert_eq!(report.entries_rewritten, 1);
    let new_reader = book.read();
    assert!(new_reader.zip().entry(&unused).is_none());
    let added = EntryName::parse("OEBPS/added-é.txt").unwrap();
    let entry = new_reader.zip().entry(&added).unwrap();
    assert_eq!(
        new_reader
            .zip()
            .read_entry(new_reader.source(), entry, &DEFAULT_LIMITS)
            .unwrap(),
        b"added"
    );
    let changed: BTreeSet<&str> = [unused.as_str()].into();
    let compared = assert_only_named_changed(&source, &scratch.book_path(), &changed, false);
    assert_eq!(compared, reader.zip().entries().len() - 1);
}

fn book_with_zip64_marks(source: &Path) -> Vec<u8> {
    let bytes = fs::read(source).unwrap();
    let file = File::open(source).unwrap();
    let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
    let start = zip
        .entries()
        .iter()
        .map(|entry| entry.record.end)
        .max()
        .unwrap();
    let mut out = bytes[..start as usize].to_vec();
    let cd_start = out.len() as u64;
    let mut rows: Vec<(u64, &ZipEntry)> = zip
        .entries()
        .iter()
        .map(|entry| (entry.record.start, entry))
        .collect();
    rows.reverse();
    for (offset, entry) in rows {
        let raw = entry.raw_central_for_test();
        let name_len = usize::from(u16::from_le_bytes([raw[28], raw[29]]));
        let mut row = raw[..46 + name_len].to_vec();
        row[30..32].copy_from_slice(&12u16.to_le_bytes());
        row[32..34].fill(0);
        row[42..46].copy_from_slice(&u32::MAX.to_le_bytes());
        row.extend_from_slice(&1u16.to_le_bytes());
        row.extend_from_slice(&8u16.to_le_bytes());
        row.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&row);
    }
    let size = out.len() as u64 - cd_start;
    let end = crate::zip::end_records_for_test(
        zip.entries().len() as u64,
        size,
        cd_start,
        b"kept comment",
    );
    out.extend_from_slice(&end);
    out
}

#[test]
fn zip64_marked_offsets_are_patched_in_their_extra_field_and_the_archive_comment_survives() {
    let scratch = Scratch::new("zip64-mark");
    let marked = book_with_zip64_marks(&fixture(SMALL));
    fs::write(scratch.book_path(), &marked).unwrap();
    let book = scratch.open();
    let total = book.read().zip().entries().len();
    let target = first_document(&book.read());
    let target_name = target.name.as_str().to_owned();
    let longer: Vec<u8> = with_marker(&target.bytes, &"grown ".repeat(2000));
    book.save_entries(vec![NewEntry::deflate(target.name.clone(), &longer)])
        .unwrap();
    let reader = book.read();
    let mut read = 0;
    for entry in reader.zip().entries() {
        reader
            .zip()
            .read_entry(reader.source(), entry, &DEFAULT_LIMITS)
            .unwrap();
        read += 1;
    }
    assert_eq!(read, total);
    assert_eq!(first_document(&reader).bytes, longer);
    let on_disk = fs::read(scratch.book_path()).unwrap();
    assert!(on_disk.ends_with(b"kept comment"));
    let python = python_records(
        &scratch.book_path(),
        &scratch.book_path(),
        &[&target_name],
        &scratch,
    );
    assert!(python.contains("crc_failed=0"), "{python}");
}

#[test]
fn an_entry_count_past_the_16_bit_field_writes_zip64_end_records() {
    let scratch = Scratch::new("zip64-count");
    let target = fixture(SMALL);
    let many = scratch.0.join("books/many.epub");
    {
        let file = File::create(&many).unwrap();
        let mut writer = jadeslip_fixtures::zip::ZipWriter::new(file);
        let source_file = File::open(&target).unwrap();
        let zip = ZipIndex::parse(&source_file, &DEFAULT_LIMITS).unwrap();
        for entry in zip.entries() {
            let bytes = zip
                .read_entry(&source_file, entry, &DEFAULT_LIMITS)
                .unwrap();
            let method = if entry.name.as_str() == "mimetype" {
                jadeslip_fixtures::zip::STORED
            } else {
                jadeslip_fixtures::zip::DEFLATE
            };
            writer
                .add(entry.name.as_str().as_bytes(), &bytes, method)
                .unwrap();
        }
        for ix in 0..65_600 {
            writer
                .add(
                    format!("filler/{ix}").as_bytes(),
                    b"",
                    jadeslip_fixtures::zip::STORED,
                )
                .unwrap();
        }
        writer.finish().unwrap();
    }
    fs::rename(&many, scratch.book_path()).unwrap();
    let book = scratch.open();
    let total = book.read().zip().entries().len();
    assert!(total > 65_535);
    edit_first_chapter(&book, "edited");
    let on_disk = fs::read(scratch.book_path()).unwrap();
    let eocd = on_disk.len() - 22;
    assert_eq!(&on_disk[eocd..eocd + 4], b"PK\x05\x06");
    assert_eq!(&on_disk[eocd + 10..eocd + 12], &[0xFF, 0xFF]);
    assert_eq!(&on_disk[eocd - 20..eocd - 16], b"PK\x06\x07");
    let reopened = Reader::open(File::open(scratch.book_path()).unwrap(), DEFAULT_LIMITS).unwrap();
    assert_eq!(reopened.zip().entries().len(), total);
    let python = python_records(&scratch.book_path(), &scratch.book_path(), &[], &scratch);
    assert!(python.contains("crc_failed=0"), "{python}");
}

#[test]
fn an_offset_past_32_bits_on_an_unmarked_row_is_refused_not_truncated() {
    let error = crate::zip::push_kept_central_for_test(1 << 32).unwrap_err();
    assert!(matches!(error, Error::Limit(_)), "{error:?}");
}

#[test]
fn a_file_over_4_gib_gets_zip64_end_records_from_the_pure_end_writer() {
    let end = crate::zip::end_records_for_test(3, 100, 5 << 30, b"");
    assert_eq!(&end[..4], b"PK\x06\x06");
    assert_eq!(&end[48..56], &(5u64 << 30).to_le_bytes());
    assert_eq!(&end[56..60], b"PK\x06\x07");
    assert_eq!(&end[64..72], &((5u64 << 30) + 100).to_le_bytes());
    assert_eq!(&end[76..80], b"PK\x05\x06");
    assert_eq!(&end[92..96], &u32::MAX.to_le_bytes());
    assert_eq!(end.len(), 98);
}

fn first_commit_trace() -> Vec<Step> {
    use LedgerWrite::{Created, Head, Pending};
    let ledger = |stage| {
        [
            Step::LedgerTempWritten(stage),
            Step::LedgerSynced(stage),
            Step::LedgerRenamed(stage),
            Step::LedgerDirSynced(stage),
        ]
    };
    let mut trace = vec![
        Step::RecoveryDirSynced,
        Step::BackupCopied,
        Step::BackupSynced,
        Step::BackupRenamed,
        Step::BackupDirSynced,
    ];
    trace.extend(ledger(Created));
    trace.extend(later_commit_prefix());
    trace.extend(ledger(Pending));
    trace.extend([
        Step::HistorySynced,
        Step::StampRechecked,
        Step::BookRenamed,
        Step::BookDirSynced,
    ]);
    trace.extend(ledger(Head));
    trace
}

fn later_commit_prefix() -> Vec<Step> {
    vec![
        Step::TempCreated,
        Step::TempWriting,
        Step::TempWritten,
        Step::HistoryWriting,
        Step::HistoryWritten,
        Step::TempSynced,
        Step::TempVerified,
    ]
}

fn later_commit_trace() -> Vec<Step> {
    let all = first_commit_trace();
    let from = all
        .iter()
        .position(|step| *step == Step::TempCreated)
        .unwrap();
    all[from..].to_vec()
}

fn recording() -> Arc<Mutex<Vec<Step>>> {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let sink = trace.clone();
    install(move |step| {
        let mut trace = sink.lock().unwrap();
        if trace.last() != Some(&step) {
            trace.push(step);
        }
        Ok(())
    });
    trace
}

#[test]
fn sync_calls_run_in_the_required_order() {
    let scratch = Scratch::new("trace");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let trace = recording();
    edit_first_chapter(&book, "one");
    assert_eq!(*trace.lock().unwrap(), first_commit_trace());
    trace.lock().unwrap().clear();
    edit_first_chapter(&book, "two");
    assert_eq!(*trace.lock().unwrap(), later_commit_trace());
    install(|_| Ok(()));
}

#[test]
fn the_ledger_head_is_the_hash_of_the_renamed_file() {
    let scratch = Scratch::new("hash");
    scratch.place(&fixture(SMALL));
    let original = fs::read(scratch.book_path()).unwrap();
    let book = scratch.open();
    edit_first_chapter(&book, "edited");
    let on_disk = fs::read(scratch.book_path()).unwrap();
    let state = book.writer.lock().unwrap();
    let ledger = &state.slot.as_ref().unwrap().ledger;
    assert_eq!(ledger.head, sha(&on_disk));
    assert_eq!(ledger.base, sha(&original));
    assert_eq!(ledger.pending_head, None);
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn file_mode_is_preserved_across_a_save() {
    for mode in [0o640, 0o600, 0o664, 0o604] {
        let scratch = Scratch::new("mode");
        let path = scratch.place(&fixture(SMALL));
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        let book = scratch.open();
        edit_first_chapter(&book, "edited");
        assert_eq!(mode_of(&path), mode, "mode {mode:o}");
    }
}

#[test]
fn a_read_only_book_is_refused_with_a_readable_error_and_left_alone() {
    let scratch = Scratch::new("read-only");
    let path = scratch.place(&fixture(SMALL));
    let original = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let book = scratch.open();
    let target = first_document(&book.read());
    let error = book
        .save_entries(vec![NewEntry::deflate(target.name, b"x")])
        .unwrap_err();
    assert!(matches!(error, Error::NotEditable(_)), "{error:?}");
    assert!(error.user_message().contains("read-only"));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(mode_of(&path), 0o444);
    assert_eq!(scratch.stray_temps(), 0);
    assert!(
        scratch.slot_dirs().is_empty(),
        "no backup for a refused save"
    );
}

#[test]
fn an_unwritable_folder_is_refused_with_a_readable_error_and_no_temp() {
    let scratch = Scratch::new("read-only-dir");
    let path = scratch.place(&fixture(SMALL));
    let original = fs::read(&path).unwrap();
    let book = scratch.open();
    let dir = scratch.0.join("books");
    let target = first_document(&book.read());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let result = book.save_entries(vec![NewEntry::deflate(target.name, b"x")]);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    assert!(matches!(error, Error::NotEditable(_)), "{error:?}");
    assert!(error.user_message().contains("folder"));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(scratch.stray_temps(), 0);
}

#[test]
fn a_symlinked_path_writes_through_to_the_target_and_the_link_survives() {
    let scratch = Scratch::new("symlink");
    let real_dir = scratch.0.join("real");
    fs::create_dir_all(&real_dir).unwrap();
    let real = real_dir.join("real.epub");
    fs::write(&real, fs::read(fixture(SMALL)).unwrap()).unwrap();
    let original = fs::read(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    let link = scratch.book_path();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let book = scratch.open();
    assert_eq!(book.path(), fs::canonicalize(&real).unwrap());
    edit_first_chapter(&book, "edited");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), real);
    assert_ne!(fs::read(&real).unwrap(), original);
    assert_eq!(fs::read(&link).unwrap(), fs::read(&real).unwrap());
    assert_eq!(mode_of(&real), 0o640);
    assert_eq!(fs::read_dir(&real_dir).unwrap().count(), 1);
    assert_eq!(scratch.stray_temps(), 0);
}

#[test]
fn a_second_hard_link_keeps_the_old_bytes_after_a_save() {
    let scratch = Scratch::new("hard-link");
    let path = scratch.place(&fixture(SMALL));
    let other = scratch.0.join("books/other-name.epub");
    fs::hard_link(&path, &other).unwrap();
    let original = fs::read(&path).unwrap();
    let book = scratch.open();
    edit_first_chapter(&book, "edited");
    assert_eq!(fs::read(&other).unwrap(), original);
    assert_ne!(fs::read(&path).unwrap(), original);
}

#[test]
fn a_very_long_file_name_still_saves() {
    let scratch = Scratch::new("long-name");
    let path = scratch
        .0
        .join("books")
        .join(format!("{}.epub", "n".repeat(250)));
    fs::write(&path, fs::read(fixture(SMALL)).unwrap()).unwrap();
    let book = Book::open(&path, &scratch.data(), KEY, DEFAULT_LIMITS).unwrap();
    edit_first_chapter(&book, "edited");
    assert_eq!(scratch.stray_temps(), 0);
}

fn fail_at(step: Step) {
    let mut fired = false;
    install(move |seen| {
        if seen == step && !fired {
            fired = true;
            return Err(io::Error::new(ErrorKind::StorageFull, "injected"));
        }
        Ok(())
    });
}

#[test]
fn a_failure_at_any_step_before_the_rename_leaves_the_book_untouched_and_no_temp() {
    let trace = first_commit_trace();
    let rename_at = trace
        .iter()
        .position(|step| *step == Step::BookRenamed)
        .unwrap();
    let history_steps = [
        Step::HistoryWriting,
        Step::HistoryWritten,
        Step::HistorySynced,
    ];
    for step in trace[..rename_at]
        .iter()
        .filter(|step| !history_steps.contains(step))
    {
        let scratch = Scratch::new("inject");
        let path = scratch.place(&fixture(SMALL));
        let original = fs::read(&path).unwrap();
        let book = scratch.open();
        let snapshot = first_document(&book.read());
        fail_at(*step);
        let target = NewEntry::deflate(snapshot.name.clone(), &with_marker(&snapshot.bytes, "x"));
        let error = book.save_entries(vec![target]).unwrap_err();
        install(|_| Ok(()));
        assert!(matches!(error, Error::SaveFailed(_)), "{step:?} {error:?}");
        assert!(error.user_message().contains("not changed"), "{step:?}");
        assert_eq!(fs::read(&path).unwrap(), original, "{step:?}");
        let beside = fs::read_dir(scratch.0.join("books")).unwrap().count();
        assert_eq!(beside, 1, "{step:?} left a temp beside the book");
        assert_eq!(
            first_document(&book.read()).bytes,
            snapshot.bytes,
            "{step:?}"
        );
        let report = edit_first_chapter(&book, "retry");
        assert_eq!(report.entries_rewritten, 1, "{step:?}");
        assert_eq!(
            first_document(&book.read()).bytes,
            with_marker(&snapshot.bytes, "retry"),
            "{step:?}"
        );
        assert_eq!(scratch.stray_temps(), 0, "{step:?}");
    }
}

#[test]
fn a_failure_after_the_rename_reports_a_warning_and_the_book_is_the_new_file() {
    let trace = first_commit_trace();
    let rename_at = trace
        .iter()
        .position(|step| *step == Step::BookRenamed)
        .unwrap();
    for step in &trace[rename_at..] {
        let scratch = Scratch::new("inject-late");
        let path = scratch.place(&fixture(SMALL));
        let book = scratch.open();
        fail_at(*step);
        let report = edit_first_chapter(&book, "edited");
        install(|_| Ok(()));
        assert!(!report.warnings.is_empty(), "{step:?}");
        let fresh = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
        assert_eq!(
            first_document(&fresh).bytes,
            first_document(&book.read()).bytes
        );
        drop(book);
        let reopened = scratch.open();
        assert!(
            matches!(
                reopened.backup_status().unwrap(),
                BackupStatus::Valid { .. }
            ),
            "{step:?}"
        );
        assert_eq!(scratch.stray_temps(), 0, "{step:?}");
        edit_first_chapter(&reopened, "again");
    }
}

#[test]
fn enospc_child_saves_on_a_full_disk() {
    let Some(dir) = std::env::var_os("SS_ENOSPC_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let book_path = dir.join("book.epub");
    let data = PathBuf::from(std::env::var_os("SS_ENOSPC_DATA").unwrap());
    fs::write(&book_path, fs::read(fixture("large-h1-p")).unwrap()).unwrap();
    let original = sha(&fs::read(&book_path).unwrap());
    let book = Book::open(&book_path, &data, KEY, DEFAULT_LIMITS).unwrap();
    let snapshot = first_document(&book.read());
    let entry = NewEntry::deflate(snapshot.name, &with_marker(&snapshot.bytes, "x"));
    let error = book.save_entries(vec![entry]).unwrap_err();
    assert!(matches!(error, Error::SaveFailed(_)), "{error:?}");
    assert!(
        error.user_message().contains("disk space"),
        "{}",
        error.user_message()
    );
    assert_eq!(
        sha(&fs::read(&book_path).unwrap()),
        original,
        "book untouched"
    );
    let temps = fs::read_dir(&dir)
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            name != "book.epub" && name != "data-dir"
        })
        .count();
    assert_eq!(temps, 0, "no temp left");
    let partial_backups = walk_files(&data)
        .into_iter()
        .filter(|path| path.to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(partial_backups, 0, "no partial backup left");
    println!("ENOSPC_SEEN {}", error.user_message());
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(walk_files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

fn run_enospc(label: &str, mount_mb: u32, data_on_mount: bool) {
    let scratch = Scratch::new(label);
    let mount = scratch.0.join("mnt");
    fs::create_dir_all(&mount).unwrap();
    let data = if data_on_mount {
        mount.join("data-dir")
    } else {
        scratch.data()
    };
    let exe = std::env::current_exe().unwrap();
    let script = format!(
        "mount -t tmpfs -o size={mount_mb}m tmpfs '{}' && exec '{}' --exact commit::tests::enospc_child_saves_on_a_full_disk --nocapture",
        mount.display(),
        exe.display()
    );
    let output = Command::new("unshare")
        .args(["-Urm", "sh", "-c", &script])
        .env("SS_ENOSPC_DIR", &mount)
        .env("SS_ENOSPC_DATA", &data)
        .output();
    let Ok(output) = output else {
        println!("skipped: unshare is not available");
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("ENOSPC_SEEN"),
        "{label}: {stdout} {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_real_full_disk_while_writing_the_temp_leaves_the_book_and_removes_the_temp() {
    run_enospc("enospc-temp", 90, false);
}

#[test]
fn a_real_full_disk_while_writing_the_backup_leaves_the_book_and_removes_the_partial() {
    run_enospc("enospc-backup", 90, true);
}

fn step_name(step: Step) -> String {
    format!("{step:?}")
}

#[test]
#[ignore = "kill child, run by the kill tests"]
fn kill_child() {
    let dir = PathBuf::from(std::env::var_os("SS_KILL_ROOT").unwrap());
    let wanted = std::env::var("SS_KILL_STEP").unwrap();
    let wanted_hit: u32 = std::env::var("SS_KILL_HIT").unwrap().parse().unwrap();
    let sentinel = dir.join("sentinel");
    let book = Book::open(
        &dir.join("books/book.epub"),
        &dir.join("data"),
        KEY,
        DEFAULT_LIMITS,
    )
    .unwrap();
    let mut seen = 0;
    install(move |step| {
        if step_name(step) == wanted {
            seen += 1;
            if seen == wanted_hit {
                fs::write(&sentinel, b"at").unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        }
        Ok(())
    });
    let marker = std::env::var("SS_KILL_MARKER").unwrap();
    edit_first_chapter(&book, &marker);
    fs::write(dir.join("finished"), b"done").unwrap();
}

fn spawn_child(scratch: &Scratch, step: &str, hit: u32, marker: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commit::tests::kill_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SS_KILL_ROOT", &scratch.0)
        .env("SS_KILL_STEP", step)
        .env("SS_KILL_HIT", hit.to_string())
        .env("SS_KILL_MARKER", marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn kill_when_at_sentinel(scratch: &Scratch, child: &mut std::process::Child, label: &str) {
    let sentinel = scratch.0.join("sentinel");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !sentinel.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("{label}: child exited {status:?} before reaching the step");
        }
        assert!(
            Instant::now() < deadline,
            "{label}: child never reached the step"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(9),
        "{label}: not killed by SIGKILL"
    );
    fs::remove_file(sentinel).unwrap();
}

fn assert_no_pending_ledger(scratch: &Scratch, label: &str) {
    for slot in scratch.slot_dirs() {
        if let Ok(text) = fs::read_to_string(slot.join("ledger.json")) {
            assert!(
                text.contains("\"pending_head\":null"),
                "{label}: a pending head survives recovery"
            );
        }
    }
}

fn chapter_of(bytes: &[u8]) -> ChapterTarget {
    let scratch = Scratch::new("chapter-of");
    fs::write(scratch.book_path(), bytes).unwrap();
    first_document(&scratch.open().read())
}

fn assert_converged(
    scratch: &Scratch,
    old: &[u8],
    new: &[u8],
    after_rename: Option<bool>,
    label: &str,
    chapter: &ChapterTarget,
) {
    let on_disk = fs::read(scratch.book_path()).unwrap();
    let is_old = on_disk == old;
    let is_new = on_disk == new;
    assert!(
        is_old || is_new,
        "{label}: book is neither the old nor the new file"
    );
    if let Some(after) = after_rename {
        assert_eq!(is_new, after, "{label}: wrong side of the rename");
    }
    let book = scratch.open();
    assert_eq!(scratch.stray_temps(), 0, "{label}: temps survive recovery");
    assert_no_pending_ledger(scratch, label);
    assert_eq!(
        book.recover().unwrap(),
        0,
        "{label}: recovery did not converge"
    );
    assert_eq!(
        fs::read(scratch.book_path()).unwrap(),
        on_disk,
        "{label}: recovery touched the book"
    );
    assert!(!first_document(&book.read()).bytes.is_empty(), "{label}");
    book.save_entries(vec![NewEntry::deflate(
        chapter.name.clone(),
        &chapter.bytes,
    )])
    .unwrap();
    assert_eq!(
        fs::read(scratch.book_path()).unwrap(),
        new,
        "{label}: re-run differs"
    );
    assert!(
        matches!(book.backup_status().unwrap(), BackupStatus::Valid { .. }),
        "{label}"
    );
    let points = book.history().unwrap();
    assert!(
        points
            .iter()
            .any(|point| point.changed.contains(&chapter.name)),
        "{label}: the history is unreadable or lost the save"
    );
}

fn reference_new(source: &[u8], markers: &[&str]) -> Vec<u8> {
    let scratch = Scratch::new("reference");
    fs::write(scratch.book_path(), source).unwrap();
    let book = scratch.open();
    for marker in markers {
        edit_first_chapter(&book, marker);
    }
    fs::read(scratch.book_path()).unwrap()
}

#[test]
fn a_real_kill_at_every_step_of_a_first_commit_leaves_old_or_new_and_recovery_converges() {
    let source = fs::read(fixture(SMALL)).unwrap();
    let new = reference_new(&source, &["edited"]);
    let chapter = chapter_of(&new);
    let steps = first_commit_trace();
    let rename_at = steps
        .iter()
        .position(|step| *step == Step::BookRenamed)
        .unwrap();
    let mut proven = 0;
    for (ix, step) in steps.iter().enumerate() {
        let name = step_name(*step);
        let scratch = Scratch::new("kill-first");
        fs::write(scratch.book_path(), &source).unwrap();
        let mut child = spawn_child(&scratch, &name, 1, "edited");
        kill_when_at_sentinel(&scratch, &mut child, &name);
        assert_converged(
            &scratch,
            &source,
            &new,
            Some(ix >= rename_at),
            &name,
            &chapter,
        );
        let status = scratch.open();
        status.revert(RevertTarget::WholeBook).unwrap();
        assert_eq!(
            fs::read(scratch.book_path()).unwrap(),
            source,
            "{name}: backup is not the original"
        );
        proven += 1;
    }
    assert_eq!(proven, steps.len());
}

#[test]
fn a_real_kill_at_every_step_of_a_later_commit_keeps_the_first_backup() {
    let source = fs::read(fixture(SMALL)).unwrap();
    let old = reference_new(&source, &["one"]);
    let new = reference_new(&source, &["one", "two"]);
    let chapter = chapter_of(&new);
    let source_entries = {
        let scratch = Scratch::new("kill-later-source");
        entries_of(&scratch.place(&fixture(SMALL)))
    };
    let steps = later_commit_trace();
    let rename_at = steps
        .iter()
        .position(|step| *step == Step::BookRenamed)
        .unwrap();
    for (ix, step) in steps.iter().enumerate() {
        let name = step_name(*step);
        let scratch = Scratch::new("kill-later");
        fs::write(scratch.book_path(), &source).unwrap();
        {
            let book = scratch.open();
            edit_first_chapter(&book, "one");
        }
        assert_eq!(fs::read(scratch.book_path()).unwrap(), old);
        let mut child = spawn_child(&scratch, &name, 1, "two");
        kill_when_at_sentinel(&scratch, &mut child, &name);
        let on_disk = fs::read(scratch.book_path()).unwrap();
        assert_eq!(
            on_disk == new,
            ix >= rename_at,
            "{name}: wrong side of the rename"
        );
        assert!(on_disk == old || on_disk == new, "{name}");
        let book = scratch.open();
        assert_eq!(
            book.recover().unwrap(),
            0,
            "{name}: recovery did not converge"
        );
        assert_eq!(scratch.stray_temps(), 0, "{name}");
        assert_no_pending_ledger(&scratch, &name);
        assert!(
            matches!(book.backup_status().unwrap(), BackupStatus::Valid { .. }),
            "{name}"
        );
        assert_eq!(
            scratch.slot_dirs().len(),
            1,
            "{name}: a second slot appeared"
        );
        book.save_entries(vec![NewEntry::deflate(
            chapter.name.clone(),
            &chapter.bytes,
        )])
        .unwrap();
        assert_eq!(
            fs::read(scratch.book_path()).unwrap(),
            new,
            "{name}: re-run differs"
        );
        let points = book.history().unwrap();
        assert_eq!(points.len(), 2, "{name}: history lost or doubled a save");
        let oldest = points[1].id;
        book.restore_point(oldest, RevertTarget::WholeBook).unwrap();
        assert_eq!(
            entries_of(&scratch.book_path()),
            source_entries,
            "{name}: the first save's old version is lost"
        );
        book.revert(RevertTarget::WholeBook).unwrap();
        assert_eq!(
            fs::read(scratch.book_path()).unwrap(),
            source,
            "{name}: first backup lost"
        );
    }
}

#[test]
fn a_real_kill_in_the_middle_of_writing_the_temp_on_a_large_book() {
    let large = fixture("large-h1-p");
    let source = fs::read(&large).unwrap();
    let new = reference_new(&source, &["edited"]);
    let scratch = Scratch::new("kill-mid-write");
    fs::write(scratch.book_path(), &source).unwrap();
    let mut child = spawn_child(&scratch, "TempWriting", 5, "edited");
    kill_when_at_sentinel(&scratch, &mut child, "mid-write");
    let partial = fs::read_dir(scratch.0.join("books"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".tmp"))
        .expect("a temp is left behind");
    let partial_len = fs::metadata(&partial).unwrap().len();
    assert!(
        partial_len > 0 && partial_len < source.len() as u64,
        "partial temp"
    );
    let chapter = chapter_of(&new);
    assert_converged(&scratch, &source, &new, Some(false), "mid-write", &chapter);
}

#[test]
#[ignore = "timed SIGKILL stress: cargo test -p jadeslip-core commit::tests::timed -- --ignored --nocapture"]
fn timed_sigkills_at_random_moments_always_leave_old_or_new() {
    let source = fs::read(fixture("large-h1-p")).unwrap();
    let new = reference_new(&source, &["edited"]);
    let chapter = chapter_of(&new);
    let mut stats = (0, 0, 0);
    let mut delay_us = 500u64;
    for round in 0..60 {
        let scratch = Scratch::new("kill-timed");
        fs::write(scratch.book_path(), &source).unwrap();
        let mut child = spawn_child(&scratch, "never", 1, "edited");
        std::thread::sleep(Duration::from_micros(delay_us));
        delay_us = (delay_us * 13 / 10 + round * 37) % 900_000 + 500;
        let finished = child.try_wait().unwrap().is_some();
        let _ = child.kill();
        let _ = child.wait();
        let on_disk = fs::read(scratch.book_path()).unwrap();
        assert!(on_disk == source || on_disk == new, "round {round}");
        if finished {
            stats.2 += 1;
        } else if on_disk == source {
            stats.0 += 1;
        } else {
            stats.1 += 1;
        }
        assert_converged(&scratch, &source, &new, None, "timed", &chapter);
    }
    println!(
        "timed kills: old={} new={} finished={}",
        stats.0, stats.1, stats.2
    );
    assert!(stats.0 > 0, "no kill landed before the rename");
}

fn build_shim(out_dir: &Path) -> Option<PathBuf> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/shim/sync_trace.rs");
    let shim = out_dir.join("libsync_trace.so");
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition", "2024", "--crate-type", "cdylib", "-O", "-o"])
        .arg(&shim)
        .arg(source)
        .output()
        .ok()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(shim)
}

#[test]
fn libc_sees_fsync_and_rename_in_the_required_order() {
    let scratch = Scratch::new("shim");
    let Some(shim) = build_shim(&scratch.0) else {
        println!("skipped: rustc is not available");
        return;
    };
    fs::write(scratch.book_path(), fs::read(fixture(SMALL)).unwrap()).unwrap();
    let trace_path = scratch.0.join("trace.tsv");
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commit::tests::kill_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SS_KILL_ROOT", &scratch.0)
        .env("SS_KILL_STEP", "never")
        .env("SS_KILL_HIT", "1")
        .env("SS_KILL_MARKER", "edited")
        .env("LD_PRELOAD", &shim)
        .env("SS_SYNC_TRACE", &trace_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let trace = fs::read_to_string(&trace_path).expect("the shim wrote a trace");
    let classify = |path: &str| -> &'static str {
        if path.contains("/backups/") && path.contains("/book.epub.") && path.ends_with(".tmp") {
            "backup-temp"
        } else if path.ends_with("/book.epub") && path.contains("/backups/") {
            "backup"
        } else if path.contains("/ledger.json.") && path.ends_with(".tmp") {
            "ledger-temp"
        } else if path.ends_with("ledger.json") {
            "ledger"
        } else if path.contains(".jadeslip-") {
            "book-temp"
        } else if path.ends_with("books/book.epub") {
            "book"
        } else if path.ends_with("/books") && !path.ends_with("/data/books") {
            "book-dir"
        } else if path.contains("/backups/") {
            "slot-dir"
        } else {
            "ancestor-dir"
        }
    };
    let events: Vec<String> = trace
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.last(), Some(&"0"), "a traced call failed");
            match fields[0] {
                "fsync" | "fdatasync" => Some(format!("{} {}", fields[0], classify(fields[1]))),
                "rename" => Some(format!(
                    "rename {}>{}",
                    classify(fields[1]),
                    classify(fields[2])
                )),
                _ => None,
            }
        })
        .filter(|event| !event.ends_with("ancestor-dir"))
        .collect();
    let mut deduped = events.clone();
    deduped.dedup();
    let expected = [
        "fsync book-dir",
        "fsync backup-temp",
        "rename backup-temp>backup",
        "fsync slot-dir",
        "fsync ledger-temp",
        "rename ledger-temp>ledger",
        "fsync slot-dir",
        "fsync book-temp",
        "fsync ledger-temp",
        "rename ledger-temp>ledger",
        "fsync slot-dir",
        "rename book-temp>book",
        "fsync book-dir",
        "fsync ledger-temp",
        "rename ledger-temp>ledger",
        "fsync slot-dir",
    ];
    assert_eq!(deduped, expected, "raw trace {events:?}");
    let filesystem = Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    println!(
        "libc trace events={} filesystem={}",
        events.len(),
        String::from_utf8_lossy(&filesystem.stdout).trim()
    );
}

#[test]
fn the_backup_is_the_first_pre_edit_file_and_stays_it_over_many_saves() {
    let scratch = Scratch::new("backup");
    let path = scratch.place(&fixture(SMALL));
    let original = fs::read(&path).unwrap();
    let book = scratch.open();
    assert!(matches!(book.backup_status().unwrap(), BackupStatus::None));
    for ix in 0..5 {
        edit_first_chapter(&book, &format!("edit {ix}"));
    }
    let slots = scratch.slot_dirs();
    assert_eq!(slots.len(), 1);
    assert_eq!(fs::read(slots[0].join("book.epub")).unwrap(), original);
    assert_eq!(slots[0].file_name().unwrap().to_str().unwrap(), {
        let hash = sha(&original);
        hash.0
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    });
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    drop(book);
    let reopened = scratch.open();
    assert!(matches!(
        reopened.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    edit_first_chapter(&reopened, "after restart");
    assert_eq!(scratch.slot_dirs().len(), 1, "a restart reuses the slot");
}

#[test]
fn a_tampered_backup_is_refused_and_a_new_edit_gets_a_new_slot() {
    let scratch = Scratch::new("tampered");
    scratch.place(&fixture(SMALL));
    {
        let book = scratch.open();
        edit_first_chapter(&book, "edited");
    }
    let slot = scratch.slot_dirs().remove(0);
    let backup = slot.join("book.epub");
    let mut bytes = fs::read(&backup).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xFF;
    fs::write(&backup, &bytes).unwrap();
    let book = scratch.open();
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::OtherFileOnly
    ));
    assert!(matches!(
        book.revert(RevertTarget::WholeBook),
        Err(Error::NotEditable(_))
    ));
    assert!(matches!(book.revert_list(), Err(Error::NotEditable(_))));
    edit_first_chapter(&book, "second");
    assert_eq!(scratch.slot_dirs().len(), 2);
    assert_eq!(
        fs::read(&backup).unwrap(),
        bytes,
        "the bad slot is left in place"
    );
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
}

#[test]
fn a_leftover_slot_from_another_file_is_not_offered_for_this_one() {
    let scratch = Scratch::new("leftover");
    scratch.place(&fixture(SMALL));
    {
        let book = scratch.open();
        edit_first_chapter(&book, "edited");
    }
    scratch.place(&fixture("toc-nested-nav"));
    let book = scratch.open();
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::OtherFileOnly
    ));
    assert!(matches!(
        book.revert(RevertTarget::WholeBook),
        Err(Error::NotEditable(_))
    ));
    let before = fs::read(scratch.book_path()).unwrap();
    assert_eq!(
        sha(&before),
        hash_of(&File::open(scratch.book_path()).unwrap()).unwrap()
    );
    edit_first_chapter(&book, "other");
    assert_eq!(scratch.slot_dirs().len(), 2, "both slots stay");
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(fs::read(scratch.book_path()).unwrap(), before);
}

#[test]
fn a_corrupt_or_misnamed_ledger_is_not_trusted() {
    let scratch = Scratch::new("bad-ledger");
    let path = scratch.place(&fixture(SMALL));
    let original = fs::read(&path).unwrap();
    {
        let book = scratch.open();
        edit_first_chapter(&book, "edited");
    }
    let slot = scratch.slot_dirs().remove(0);
    let good = fs::read_to_string(slot.join("ledger.json")).unwrap();
    for bad in [
        String::new(),
        "{not json".to_owned(),
        good.replace("\"version\":1", "\"version\":9"),
    ] {
        fs::write(slot.join("ledger.json"), &bad).unwrap();
        let book = scratch.open();
        assert!(matches!(
            book.backup_status().unwrap(),
            BackupStatus::OtherFileOnly
        ));
        assert!(book.revert(RevertTarget::WholeBook).is_err());
    }
    fs::write(slot.join("ledger.json"), &good).unwrap();
    let renamed = slot.with_file_name("0".repeat(64));
    fs::rename(&slot, &renamed).unwrap();
    let book = scratch.open();
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::OtherFileOnly
    ));
    fs::rename(&renamed, &slot).unwrap();
    let book = scratch.open();
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn an_outside_edit_is_detected_and_a_touch_is_not_an_edit() {
    let scratch = Scratch::new("outside");
    let path = scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "edited");
    assert!(!book.changed_on_disk());
    let same = fs::read(&path).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&path, &same).unwrap();
    assert!(book.changed_on_disk());
    edit_first_chapter(&book, "after touch");
    let mut other = fs::read(&path).unwrap();
    let middle = other.len() / 2;
    other[middle] ^= 1;
    fs::write(&path, &other).unwrap();
    let target = first_document(&book.read());
    let result = book.save_entries(vec![NewEntry::deflate(target.name, b"x")]);
    assert!(matches!(result, Err(Error::ChangedOnDisk)), "{result:?}");
    assert_eq!(fs::read(&path).unwrap(), other);
    assert_eq!(scratch.stray_temps(), 0);
}

#[test]
fn revert_list_compares_directories_and_reads_no_entry() {
    let scratch = Scratch::new("revert-list");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let target = first_document(&book.read());
    edit_first_chapter(&book, "edited");
    let added = EntryName::parse("OEBPS/new.txt").unwrap();
    book.save_entries(vec![NewEntry::deflate(added.clone(), b"new")])
        .unwrap();
    let rows = book.revert_list().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&RevertRow {
        entry: added.clone(),
        kind: RevertKind::Added
    }));
    assert!(rows.contains(&RevertRow {
        entry: target.name.clone(),
        kind: RevertKind::Changed
    }));

    let slot = scratch.slot_dirs().remove(0);
    let recording = RecordingReader::new(File::open(slot.join("book.epub")).unwrap());
    let backup_zip = ZipIndex::parse(&recording, &DEFAULT_LIMITS).unwrap();
    let reads_before_diff = recording.ranges().len();
    let head = book.read();
    let diff = directory_diff(head.zip(), &backup_zip);
    assert_eq!(diff.len(), 2);
    assert_eq!(
        recording.ranges().len(),
        reads_before_diff,
        "the diff reads nothing"
    );
    let opened = File::open(slot.join("book.epub")).unwrap();
    let plain = ZipIndex::parse(&opened, &DEFAULT_LIMITS).unwrap();
    let checked = plain.entries().len();
    let len = opened.metadata().unwrap().len();
    let end_record_scan = len.saturating_sub(22 + 0xFFFF)..len;
    for entry in plain.entries() {
        for range in recording
            .ranges()
            .into_iter()
            .filter(|range| *range != end_record_scan)
        {
            assert!(
                range.end <= entry.record.start || range.start >= entry.record.end,
                "an entry record was read"
            );
        }
    }
    assert!(checked > 0);
}

#[test]
fn reverting_one_chapter_restores_its_bytes_and_keeps_the_other_edit() {
    let scratch = Scratch::new("revert-one");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let reader = book.read();
    let documents: Vec<ChapterTarget> = reader
        .rows()
        .iter()
        .filter_map(|row| match reader.extract(row.ix) {
            Ok(Extracted::Document(bytes))
                if String::from_utf8_lossy(&bytes).contains("</body>") =>
            {
                Some(ChapterTarget {
                    name: row.href.clone()?,
                    bytes,
                })
            }
            _ => None,
        })
        .collect();
    assert!(documents.len() >= 2);
    let (a, b) = (&documents[0], &documents[1]);
    book.save_entries(vec![
        NewEntry::deflate(a.name.clone(), &with_marker(&a.bytes, "a")),
        NewEntry::deflate(b.name.clone(), &with_marker(&b.bytes, "b")),
    ])
    .unwrap();
    assert_eq!(book.revert_list().unwrap().len(), 2);
    let report = book.revert(RevertTarget::Chapter(a.name.clone())).unwrap();
    assert_eq!(report.entries_rewritten, 1);
    let rows = book.revert_list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entry, b.name);
    let head = book.read();
    let read = |name: &EntryName| {
        head.zip()
            .read_entry(
                head.source(),
                head.zip().entry(name).unwrap(),
                &DEFAULT_LIMITS,
            )
            .unwrap()
    };
    assert_eq!(read(&a.name), a.bytes);
    assert_eq!(read(&b.name), with_marker(&b.bytes, "b"));
    let again = book.revert(RevertTarget::Chapter(a.name.clone())).unwrap();
    assert_eq!(
        again.bytes_written, 0,
        "reverting an unchanged chapter writes nothing"
    );
    let missing = EntryName::parse("OEBPS/nope.xhtml").unwrap();
    assert!(matches!(
        book.revert(RevertTarget::Chapter(missing)),
        Err(Error::NotEditable(_))
    ));
}

#[test]
fn whole_book_revert_goes_through_commit_and_is_byte_identical_and_keeps_permissions() {
    let scratch = Scratch::new("revert-all");
    let path = scratch.place(&fixture(SMALL));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let original = fs::read(&path).unwrap();
    let book = scratch.open();
    edit_first_chapter(&book, "edited");
    let trace = recording();
    let report = book.revert(RevertTarget::WholeBook).unwrap();
    let steps = trace.lock().unwrap().clone();
    install(|_| Ok(()));
    assert!(report.bytes_written > 0);
    let history = [
        Step::HistoryWriting,
        Step::HistoryWritten,
        Step::HistorySynced,
    ];
    let mut expected: Vec<Step> = later_commit_trace()
        .into_iter()
        .filter(|step| !history.contains(step))
        .collect();
    let verified = expected
        .iter()
        .position(|step| *step == Step::TempVerified)
        .unwrap();
    expected.splice(verified + 1..verified + 1, history[..2].iter().copied());
    let pending = expected
        .iter()
        .position(|step| *step == Step::LedgerDirSynced(LedgerWrite::Pending))
        .unwrap();
    expected.insert(pending + 1, Step::HistorySynced);
    assert_eq!(steps, expected, "revert runs the full commit path");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(mode_of(&path), 0o640);
    assert!(book.revert_list().unwrap().is_empty());
    let again = book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(again.bytes_written, 0);
    assert_eq!(scratch.slot_dirs().len(), 1);
    edit_first_chapter(&book, "after revert");
    assert_eq!(
        scratch.slot_dirs().len(),
        1,
        "the slot stays in use after a revert"
    );
}

#[test]
fn two_handles_on_one_book_cannot_commit_at_the_same_time() {
    let scratch = Scratch::new("lock");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let other = scratch.open();
    let _held = other.data.lock().unwrap();
    let target = first_document(&book.read());
    let result = book.save_entries(vec![NewEntry::deflate(target.name, b"x")]);
    assert!(matches!(result, Err(Error::NotEditable(_))), "{result:?}");
}

#[test]
fn a_replacement_that_breaks_the_package_is_refused_by_the_final_check() {
    let scratch = Scratch::new("break");
    let path = scratch.place(&fixture(SMALL));
    let original = fs::read(&path).unwrap();
    let book = scratch.open();
    let opf = book
        .read()
        .zip()
        .entries()
        .iter()
        .find(|entry| entry.name.as_str().ends_with(".opf"))
        .expect("an OPF entry")
        .name
        .clone();
    let result = book.save_entries(vec![NewEntry::deflate(opf, b"not a package")]);
    assert!(matches!(result, Err(Error::WouldBreak(_))), "{result:?}");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(scratch.0.join("books")).unwrap().count(), 1);
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
}

fn real_books() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "epub"));
    paths.sort();
    paths
}

fn short_id(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy();
    stem.rsplit('-').next().unwrap_or("").to_owned()
}

fn parses_as_xml(bytes: &[u8]) -> bool {
    let (text, _) = crate::chapter::decode_source(bytes.to_vec());
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(&text, options).is_ok()
}

fn editable_chapters(reader: &Reader<File>) -> Vec<ChapterTarget> {
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter_map(|row| match reader.extract(row.ix) {
            Ok(Extracted::Document(bytes))
                if parses_as_xml(&bytes) && memchr::memmem::rfind(&bytes, b"</body>").is_some() =>
            {
                Some(ChapterTarget {
                    name: row.href.clone()?,
                    bytes,
                })
            }
            _ => None,
        })
        .collect()
}

fn marker_in_place(bytes: &[u8], marker: &[u8]) -> Vec<u8> {
    let at = memchr::memmem::rfind(bytes, b"</body>").expect("a body end");
    [&bytes[..at], marker, &bytes[at..]].concat()
}

fn metadata_output(path: &Path) -> (bool, Vec<u8>) {
    let output = Command::new("ebook-meta")
        .arg(path)
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("ebook-meta runs");
    let lines: Vec<u8> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.starts_with("Last modified"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    let has_title = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.starts_with("Title"));
    (output.status.success() && has_title, lines)
}

fn prove_saved_book(label: &str, source: &Path) {
    let scratch = Scratch::new("real");
    let copy = scratch.book_path();
    fs::copy(source, &copy).unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o644)).unwrap();
    let original_hash = sha(&fs::read(&copy).unwrap());
    let (meta_ok, original_meta) = metadata_output(&copy);
    assert!(meta_ok, "{label}: ebook-meta opens the original");

    let book = scratch.open();
    let total = book.read().zip().entries().len();
    let chapters = editable_chapters(&book.read());
    assert!(!chapters.is_empty(), "{label}: an editable chapter exists");
    let middle = &chapters[chapters.len() / 2];
    let report = book
        .save_entries(vec![NewEntry::deflate(
            middle.name.clone(),
            &marker_in_place(&middle.bytes, b"<p>edited</p>"),
        )])
        .unwrap();
    let normalised = report.mimetype_normalised;
    let changed: BTreeSet<&str> = [middle.name.as_str()].into();
    let one_compared = assert_only_named_changed(source, &copy, &changed, normalised);
    assert_eq!(
        one_compared,
        total - 1 - usize::from(normalised),
        "{label}: compared count"
    );
    let mut named = vec![middle.name.as_str()];
    if normalised {
        named.push("mimetype");
    }
    let python = python_records(source, &copy, &named, &scratch);
    assert!(python.contains("differ_unexpected=0"), "{label}: {python}");
    assert!(python.contains("crc_failed=0"), "{label}: {python}");
    assert!(
        python.contains("mimetype_first_stored=True"),
        "{label}: {python}"
    );
    let reopened = Reader::open(File::open(&copy).unwrap(), DEFAULT_LIMITS).unwrap();
    let saved = reopened
        .zip()
        .read_entry(
            reopened.source(),
            reopened.zip().entry(&middle.name).unwrap(),
            &DEFAULT_LIMITS,
        )
        .unwrap();
    assert!(parses_as_xml(&saved), "{label}: changed chapter parses");
    let (saved_ok, saved_meta) = metadata_output(&copy);
    assert!(saved_ok, "{label}: ebook-meta opens the saved book");
    assert_eq!(saved_meta, original_meta, "{label}: metadata unchanged");

    let all: Vec<NewEntry> = chapters
        .iter()
        .map(|chapter| {
            NewEntry::deflate(
                chapter.name.clone(),
                &marker_in_place(&chapter.bytes, b"<p>all</p>"),
            )
        })
        .collect();
    let all_count = all.len();
    let report = book.save_entries(all).unwrap();
    assert_eq!(report.entries_rewritten as usize, all_count);
    let names: BTreeSet<&str> = chapters
        .iter()
        .map(|chapter| chapter.name.as_str())
        .collect();
    let all_compared = assert_only_named_changed(source, &copy, &names, normalised);
    assert_eq!(
        all_compared,
        total - all_count - usize::from(normalised),
        "{label}: compared count"
    );
    let reopened = Reader::open(File::open(&copy).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut parsed = 0;
    for chapter in &chapters {
        let entry = reopened.zip().entry(&chapter.name).unwrap();
        let bytes = reopened
            .zip()
            .read_entry(reopened.source(), entry, &DEFAULT_LIMITS)
            .unwrap();
        assert!(parses_as_xml(&bytes), "{label}: a changed chapter parses");
        parsed += 1;
    }
    let (all_ok, all_meta) = metadata_output(&copy);
    assert!(all_ok, "{label}: ebook-meta opens the all-chapters save");
    assert_eq!(all_meta, original_meta, "{label}: metadata unchanged");

    let rows = book.revert_list().unwrap().len();
    assert_eq!(rows, all_count, "{label}: revert list");
    book.revert(RevertTarget::Chapter(middle.name.clone()))
        .unwrap();
    assert_eq!(book.revert_list().unwrap().len(), all_count - 1, "{label}");
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(
        sha(&fs::read(&copy).unwrap()),
        original_hash,
        "{label}: revert"
    );
    assert!(book.revert_list().unwrap().is_empty(), "{label}");

    println!(
        "{label:12} entries={total} one_compared={one_compared} all_changed={all_count} all_compared={all_compared} xml_parsed={parsed} normalised={normalised} python_one[{python}] revert=byte-identical ebook_meta=ok"
    );
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --lib commit::tests::every_real -- --ignored --nocapture"]
fn every_real_book_copy_and_large_book_saves_with_records_byte_equal() {
    let books = real_books();
    assert_eq!(books.len(), 8, "fixtures/real has its 8 books");
    let before: Vec<FileHash> = books
        .iter()
        .map(|path| sha(&fs::read(path).unwrap()))
        .collect();
    for path in &books {
        prove_saved_book(&short_id(path), path);
    }
    for name in ["large-h1-p", "large-h1-div-br"] {
        prove_saved_book(name, &fixture(name));
    }
    let after: Vec<FileHash> = books
        .iter()
        .map(|path| sha(&fs::read(path).unwrap()))
        .collect();
    assert_eq!(before, after, "fixtures/real stayed read-only");
}

#[test]
fn a_child_process_starting_at_the_wrong_moment_does_not_look_like_a_second_window() {
    let scratch = Scratch::new("lock-fork");
    let data = BookData::new(&scratch.data(), KEY);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spawners: Vec<_> = (0..4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = Command::new("true").status();
                }
            })
        })
        .collect();
    let mut refused = 0;
    for _ in 0..3000 {
        if data.lock().is_err() {
            refused += 1;
        }
    }
    stop.store(true, Ordering::Relaxed);
    for spawner in spawners {
        spawner.join().unwrap();
    }
    assert_eq!(refused, 0);
}

fn entries_of(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let file = File::open(path).unwrap();
    let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
    zip.entries()
        .iter()
        .map(|entry| {
            (
                entry.name.as_str().to_owned(),
                zip.read_entry(&file, entry, &DEFAULT_LIMITS).unwrap(),
            )
        })
        .collect()
}

fn chapter_bytes(book: &Book, name: &EntryName) -> Vec<u8> {
    let reader = book.read();
    let entry = reader.zip().entry(name).unwrap();
    reader
        .zip()
        .read_entry(reader.source(), entry, &DEFAULT_LIMITS)
        .unwrap()
}

#[test]
fn every_save_keeps_the_old_chapter_and_restores_bring_back_exact_bytes() {
    let scratch = Scratch::new("history-restore");
    scratch.place(&fixture(SMALL));
    let original = entries_of(&scratch.book_path());
    let book = scratch.open();
    let name = first_document(&book.read()).name;
    assert!(book.history().unwrap().is_empty());

    edit_first_chapter(&book, "QQQONE");
    let named = book.take_snapshot("  before rewrite  ").unwrap();
    let after_one = chapter_bytes(&book, &name);
    edit_first_chapter(&book, "QQQTWO");

    let points = book.history().unwrap();
    let kinds: Vec<&PointKind> = points.iter().map(|point| &point.kind).collect();
    assert_eq!(
        kinds,
        [
            &PointKind::Save,
            &PointKind::Named {
                name: "before rewrite".to_owned()
            },
            &PointKind::Save,
        ]
    );
    assert_eq!(points[0].changed, std::slice::from_ref(&name));
    assert_eq!(points[1].id, named);
    let first = points[2].id;

    let differs = book.history_changes(first).unwrap();
    assert!(differs.complete);
    assert_eq!(
        differs.rows,
        [RevertRow {
            entry: name.clone(),
            kind: RevertKind::Changed
        }]
    );
    let diff = book.history_diff(first, &name).unwrap();
    let added: String = diff
        .iter()
        .filter(|part| part.kind == crate::DiffKind::New)
        .map(|part| part.text.as_str())
        .collect();
    assert!(
        added.contains("QQQONE") && added.contains("QQQTWO"),
        "{added}"
    );
    assert!(diff.iter().all(|part| part.kind != crate::DiffKind::Gone));

    let report = book
        .restore_point(first, RevertTarget::Chapter(name.clone()))
        .unwrap();
    assert!(report.bytes_written > 0);
    assert_eq!(chapter_bytes(&book, &name), original[name.as_str()]);
    assert_eq!(
        book.history().unwrap()[0].kind,
        PointKind::Restore { to: Some(first) }
    );

    book.restore_point(named, RevertTarget::WholeBook).unwrap();
    assert_eq!(chapter_bytes(&book, &name), after_one);

    book.restore_point(first, RevertTarget::WholeBook).unwrap();
    assert_eq!(entries_of(&scratch.book_path()), original);

    let undo = book.history().unwrap()[0].id;
    book.restore_point(undo, RevertTarget::WholeBook).unwrap();
    assert_eq!(
        chapter_bytes(&book, &name),
        after_one,
        "a restore can be undone"
    );

    drop(book);
    let reopened = scratch.open();
    assert_eq!(reopened.history().unwrap().len(), 7);
}

#[test]
fn a_whole_book_restore_brings_back_a_deleted_chapter() {
    let scratch = Scratch::new("history-delete");
    scratch.place(&fixture(SMALL));
    let original = entries_of(&scratch.book_path());
    let book = scratch.open();
    let rows = book.read().rows().len();
    let start = book.take_snapshot("start").unwrap();
    let chapter = book
        .read()
        .rows()
        .iter()
        .find(|row| row.linear_ordinal.is_some())
        .unwrap()
        .ix;
    book.manage(ChapterOp::Delete { chapter }, None).unwrap();
    assert_eq!(book.read().rows().len(), rows - 1);
    assert!(
        book.history_changes(start)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.kind == RevertKind::Removed)
    );
    book.restore_point(start, RevertTarget::WholeBook).unwrap();
    assert_eq!(book.read().rows().len(), rows);
    assert_eq!(entries_of(&scratch.book_path()), original);
}

#[test]
fn a_failed_history_write_still_saves_the_book_with_a_warning() {
    let scratch = Scratch::new("history-fails");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = first_document(&book.read()).name;
    install(|step| {
        if step == Step::HistoryWriting {
            Err(io::Error::other("injected"))
        } else {
            Ok(())
        }
    });
    let report = edit_first_chapter(&book, "QQQSAVED");
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("history")),
        "{:?}",
        report.warnings
    );
    assert!(String::from_utf8_lossy(&chapter_bytes(&book, &name)).contains("QQQSAVED"));
    install(|_| Ok(()));
    let report = edit_first_chapter(&book, "QQQNEXT");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let points = scratch.open().history().unwrap();
    assert_eq!(
        points.len(),
        1,
        "the torn write is ignored and the next save lands"
    );
    let before = book.history_diff(points[0].id, &name).unwrap();
    assert!(
        before
            .iter()
            .any(|part| part.kind == crate::DiffKind::New && part.text.contains("QQQNEXT"))
    );
}

#[test]
fn a_save_that_fails_after_its_history_write_leaves_no_point() {
    let scratch = Scratch::new("history-phantom");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    install(|step| {
        if step == Step::StampRechecked {
            Err(io::Error::other("injected"))
        } else {
            Ok(())
        }
    });
    let target = first_document(&book.read());
    let entry = NewEntry::deflate(target.name.clone(), &with_marker(&target.bytes, "QQQLOST"));
    assert!(book.save_entries(vec![entry]).is_err());
    install(|_| Ok(()));
    assert!(book.history().unwrap().is_empty());
    edit_first_chapter(&book, "QQQKEPT");
    let points = book.history().unwrap();
    assert_eq!(points.len(), 1);
    assert!(book.history_changes(points[0].id).unwrap().complete);
}

fn history_log(scratch: &Scratch) -> PathBuf {
    scratch
        .data()
        .join(format!("books/{:032x}/history/log", KEY.0))
}

fn history_len(scratch: &Scratch) -> u64 {
    fs::metadata(history_log(scratch)).map_or(0, |meta| meta.len())
}

#[derive(Clone, Copy)]
enum Loss {
    WholeFrames,
    BlobBodies,
}

fn lose_history(scratch: &Scratch, from: u64, to: u64, loss: Loss) {
    let mut bytes = fs::read(history_log(scratch)).unwrap();
    let mut at = from as usize;
    while at < to as usize {
        let kind = bytes[at + 4];
        let len = u32::from_le_bytes(bytes[at + 5..at + 9].try_into().unwrap()) as usize;
        let body = at + 13;
        match loss {
            Loss::WholeFrames => bytes[at..body + len].fill(0),
            Loss::BlobBodies if kind == 1 => bytes[body + 46..body + len].fill(0),
            Loss::BlobBodies => {}
        }
        at = body + len;
    }
    fs::write(history_log(scratch), &bytes).unwrap();
}

#[test]
fn a_failed_snapshot_flush_does_not_cost_the_saves_after_it() {
    let scratch = Scratch::new("history-flush-hides");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "QQQONE");
    let before = history_len(&scratch);
    fail_at(Step::HistorySynced);
    assert!(book.take_snapshot("snap").is_err());
    install(|_| Ok(()));
    let after = history_len(&scratch);
    let report = edit_first_chapter(&book, "QQQTWO");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    drop(book);
    lose_history(&scratch, before, after, Loss::WholeFrames);
    let reopened = scratch.open();
    assert_eq!(
        reopened.history().unwrap().len(),
        2,
        "both saves after reopen"
    );
    edit_first_chapter(&reopened, "QQQTHREE");
    drop(reopened);
    assert_eq!(
        scratch.open().history().unwrap().len(),
        3,
        "all three saves after one more save"
    );
}

#[test]
fn a_version_lost_in_a_failed_flush_is_stored_again_by_a_later_save() {
    let scratch = Scratch::new("history-flush-reuse");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = first_document(&book.read()).name;
    let start = history_len(&scratch);
    fail_at(Step::HistorySynced);
    let report = edit_first_chapter(&book, "QQQONE");
    install(|_| Ok(()));
    assert!(!report.warnings.is_empty());
    let end = history_len(&scratch);
    book.revert(RevertTarget::WholeBook).unwrap();
    let report = edit_first_chapter(&book, "QQQTWO");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    lose_history(&scratch, start, end, Loss::BlobBodies);
    let latest = book.history().unwrap()[0].id;
    book.restore_point(latest, RevertTarget::Chapter(name.clone()))
        .unwrap();
    assert!(!String::from_utf8_lossy(&chapter_bytes(&book, &name)).contains("QQQ"));
}

#[test]
fn a_version_torn_by_a_power_cut_is_stored_again_by_the_next_save() {
    let scratch = Scratch::new("history-torn-reuse");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = first_document(&book.read()).name;
    let start = history_len(&scratch);
    fail_at(Step::TempSynced);
    let target = first_document(&book.read());
    let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, "QQQLOST"));
    assert!(book.save_entries(vec![entry]).is_err());
    install(|_| Ok(()));
    let end = history_len(&scratch);
    drop(book);
    lose_history(&scratch, start, end, Loss::BlobBodies);
    let reopened = scratch.open();
    let report = edit_first_chapter(&reopened, "QQQTWO");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let latest = reopened.history().unwrap()[0].id;
    reopened
        .restore_point(latest, RevertTarget::Chapter(name.clone()))
        .unwrap();
    assert!(!String::from_utf8_lossy(&chapter_bytes(&reopened, &name)).contains("QQQ"));
}

#[test]
fn damage_in_the_middle_of_the_history_keeps_the_saves_after_it() {
    let scratch = Scratch::new("history-middle-damage");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "QQQONE");
    let middle = history_len(&scratch);
    edit_first_chapter(&book, "QQQTWO");
    let end = history_len(&scratch);
    edit_first_chapter(&book, "QQQTHREE");
    drop(book);
    lose_history(&scratch, middle, end, Loss::WholeFrames);
    let reopened = scratch.open();
    assert_eq!(
        reopened.history().unwrap().len(),
        2,
        "the saves around the damage"
    );
    edit_first_chapter(&reopened, "QQQFOUR");
    drop(reopened);
    assert_eq!(
        scratch.open().history().unwrap().len(),
        3,
        "nothing cut by the next save"
    );
}

struct Durability {
    survivors: std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>,
    flush_fails: std::rc::Rc<std::cell::Cell<bool>>,
    fail_once: std::rc::Rc<std::cell::Cell<Option<Step>>>,
    path: PathBuf,
}

impl Durability {
    fn track(path: &Path) -> Durability {
        let survivors = std::rc::Rc::new(std::cell::RefCell::new(vec![fs::read(path).unwrap()]));
        let flush_fails = std::rc::Rc::new(std::cell::Cell::new(false));
        let fail_once = std::rc::Rc::new(std::cell::Cell::new(None));
        let (seen, failing, book) = (survivors.clone(), flush_fails.clone(), path.to_owned());
        let once = fail_once.clone();
        install(move |step| {
            if once.get() == Some(step) {
                once.set(None);
                return Err(io::Error::other("injected failure"));
            }
            match step {
                Step::BookRenamed => seen.borrow_mut().push(fs::read(&book).unwrap()),
                Step::BookDirSynced | Step::RecoveryDirSynced if failing.get() => {
                    return Err(io::Error::other("injected folder sync failure"));
                }
                Step::BookDirSynced | Step::RecoveryDirSynced => {
                    *seen.borrow_mut() = vec![fs::read(&book).unwrap()];
                }
                _ => {}
            }
            Ok(())
        });
        Durability {
            survivors,
            flush_fails,
            fail_once,
            path: path.to_owned(),
        }
    }

    fn renamed_elsewhere(&self) {
        self.survivors
            .borrow_mut()
            .push(fs::read(&self.path).unwrap());
    }

    fn survivor_count(&self) -> usize {
        self.survivors.borrow().len()
    }

    fn power_cut_keeping(self, survivor: usize) {
        let kept = self.survivors.borrow()[survivor].clone();
        install(|_| Ok(()));
        fs::write(&self.path, kept).unwrap();
    }
}

fn assert_revert_reaches(scratch: &Scratch, original: &[u8], label: &str) {
    let book = scratch.open();
    match book.revert(RevertTarget::WholeBook) {
        Ok(_) => assert_eq!(
            fs::read(scratch.book_path()).unwrap(),
            original,
            "{label}: Revert did not restore the original book"
        ),
        Err(error) => panic!(
            "{label}: Revert to backup is gone after the power cut: {}",
            error.user_message()
        ),
    }
}

#[test]
fn a_power_cut_after_a_failed_folder_flush_keeps_revert_to_the_original() {
    let scratch = Scratch::new("flush-fails");
    let original = fs::read(scratch.place(&fixture(SMALL))).unwrap();
    let book = scratch.open();
    let durability = Durability::track(&scratch.book_path());
    edit_first_chapter(&book, "first");
    durability.flush_fails.set(true);
    let report = edit_first_chapter(&book, "second");
    assert!(!report.warnings.is_empty(), "the failed flush is reported");
    drop(book);
    durability.power_cut_keeping(0);
    assert_revert_reaches(&scratch, &original, "flush failed");
}

#[test]
fn a_power_cut_after_a_crash_and_reopen_keeps_revert_to_the_original() {
    let scratch = Scratch::new("crash-reopen-cut");
    let original = fs::read(scratch.place(&fixture(SMALL))).unwrap();
    let durability = Durability::track(&scratch.book_path());
    edit_first_chapter(&scratch.open(), "first");
    let mut child = spawn_child(&scratch, &step_name(Step::BookRenamed), 1, "second");
    kill_when_at_sentinel(&scratch, &mut child, "crash after rename");
    durability.renamed_elsewhere();
    drop(scratch.open());
    durability.power_cut_keeping(0);
    assert_revert_reaches(&scratch, &original, "crash, reopen, power cut");
}

#[test]
fn a_power_cut_after_two_failed_folder_flushes_keeps_revert_to_the_original() {
    let mut survivors = 1;
    let mut kept = 0;
    while kept < survivors {
        let scratch = Scratch::new("flush-fails-twice");
        let original = fs::read(scratch.place(&fixture(SMALL))).unwrap();
        let book = scratch.open();
        let durability = Durability::track(&scratch.book_path());
        edit_first_chapter(&book, "first");
        durability.flush_fails.set(true);
        edit_first_chapter(&book, "second");
        let target = first_document(&book.read());
        let third = NewEntry::deflate(target.name, &with_marker(&target.bytes, "third"));
        let _ = book.save_entries(vec![third]);
        drop(book);
        survivors = durability.survivor_count();
        durability.power_cut_keeping(kept);
        assert_revert_reaches(
            &scratch,
            &original,
            &format!("survivor {kept} of {survivors}"),
        );
        kept += 1;
    }
}

#[test]
fn a_power_cut_after_a_crash_another_window_did_not_recover_keeps_revert_to_the_original() {
    let scratch = Scratch::new("busy-open-cut");
    let original = fs::read(scratch.place(&fixture(SMALL))).unwrap();
    let durability = Durability::track(&scratch.book_path());
    edit_first_chapter(&scratch.open(), "first");
    let mut child = spawn_child(&scratch, &step_name(Step::BookRenamed), 1, "second");
    kill_when_at_sentinel(&scratch, &mut child, "crash after rename");
    durability.renamed_elsewhere();
    let saving_elsewhere = File::options()
        .write(true)
        .open(
            scratch
                .data()
                .join(format!("books/{:032x}/commit.lock", KEY.0)),
        )
        .unwrap();
    saving_elsewhere.lock().unwrap();
    let book = scratch.open();
    drop(saving_elsewhere);
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    durability.fail_once.set(Some(Step::StampRechecked));
    let target = first_document(&book.read());
    let third = NewEntry::deflate(target.name, &with_marker(&target.bytes, "third"));
    assert!(book.save_entries(vec![third]).is_err());
    drop(book);
    durability.power_cut_keeping(0);
    assert_revert_reaches(&scratch, &original, "crash seen only by another window");
}

#[test]
fn a_power_cut_after_another_window_reverted_keeps_revert_to_the_original() {
    let scratch = Scratch::new("aba-cut");
    let original = fs::read(scratch.place(&fixture(SMALL))).unwrap();
    let durability = Durability::track(&scratch.book_path());
    let first = scratch.open();
    edit_first_chapter(&first, "a");
    first.revert(RevertTarget::WholeBook).unwrap();
    let second = scratch.open();
    assert!(matches!(
        second.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    edit_first_chapter(&first, "b");
    durability.flush_fails.set(true);
    first.revert(RevertTarget::WholeBook).unwrap();
    let target = first_document(&second.read());
    let third = NewEntry::deflate(target.name, &with_marker(&target.bytes, "c"));
    let _ = second.save_entries(vec![third]);
    drop((first, second));
    durability.power_cut_keeping(0);
    assert_revert_reaches(&scratch, &original, "stale window after a revert");
}

fn edit_chapter(book: &Book, nth: usize, marker: &str) {
    let target = editable_chapters(&book.read()).swap_remove(nth);
    let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, marker));
    book.save_entries(vec![entry]).unwrap();
}
fn later_than_now() -> u64 {
    crate::library::now_ms() + 3_600_000
}
fn named_id(book: &Book, name: &str) -> PointId {
    book.history()
        .unwrap()
        .into_iter()
        .find(|point| {
            point.kind
                == PointKind::Named {
                    name: name.to_owned(),
                }
        })
        .unwrap()
        .id
}
fn history_log_in(data: &Path) -> PathBuf {
    data.join(format!("books/{:032x}/history/log", KEY.0))
}
fn tree_size(root: &Path) -> u64 {
    walk_files(root)
        .iter()
        .map(|path| fs::symlink_metadata(path).unwrap().len())
        .sum()
}
#[test]
fn pruning_keeps_named_snapshots_restoring_the_same_bytes_and_hides_folded_versions() {
    let scratch = Scratch::new("prune-book");
    scratch.place(&fixture(SMALL));
    let original = entries_of(&scratch.book_path());
    let book = scratch.open();
    let chapters = editable_chapters(&book.read());
    let (first, second) = (chapters[0].name.clone(), chapters[1].name.clone());
    book.take_snapshot("start").unwrap();
    edit_chapter(&book, 0, "QQQONE");
    edit_chapter(&book, 1, "QQQTWO");
    let middle = entries_of(&scratch.book_path());
    book.take_snapshot("middle").unwrap();
    edit_chapter(&book, 0, "QQQTHREE");
    edit_chapter(&book, 1, "QQQFOUR");
    assert_eq!(book.history().unwrap().len(), 6);
    let start_changes = book.history_changes(named_id(&book, "start")).unwrap();
    let middle_changes = book.history_changes(named_id(&book, "middle")).unwrap();

    let pruned = book.prune_history(later_than_now(), false).unwrap();
    assert_eq!(pruned.points, 3);
    assert!(pruned.bytes_after < pruned.bytes_before);
    let points = book.history().unwrap();
    let kinds: Vec<&PointKind> = points.iter().map(|point| &point.kind).collect();
    assert_eq!(
        kinds,
        [
            &PointKind::Save,
            &PointKind::Named {
                name: "middle".to_owned()
            },
            &PointKind::Named {
                name: "start".to_owned()
            },
        ]
    );
    assert_eq!(points[0].changed, std::slice::from_ref(&second));
    assert!(
        points[1].changed.is_empty() && points[2].changed.is_empty(),
        "versions folded into a snapshot are not listed as its changes"
    );
    let (start, middle_id) = (points[2].id, points[1].id);
    assert_eq!(book.history_changes(start).unwrap(), start_changes);
    assert_eq!(book.history_changes(middle_id).unwrap(), middle_changes);

    book.restore_point(start, RevertTarget::Chapter(first.clone()))
        .unwrap();
    assert_eq!(chapter_bytes(&book, &first), original[first.as_str()]);
    book.restore_point(middle_id, RevertTarget::WholeBook)
        .unwrap();
    assert_eq!(entries_of(&scratch.book_path()), middle);
    book.restore_point(start, RevertTarget::WholeBook).unwrap();
    assert_eq!(entries_of(&scratch.book_path()), original);
    drop(book);
    assert_eq!(scratch.open().history().unwrap().len(), 6);
}
#[test]
fn a_gap_before_kept_points_still_refuses_a_whole_book_restore_after_pruning() {
    let scratch = Scratch::new("prune-book-gap");
    scratch.place(&fixture(SMALL));
    let original = entries_of(&scratch.book_path());
    let book = scratch.open();
    let first = editable_chapters(&book.read())[0].name.clone();
    book.take_snapshot("start").unwrap();
    edit_chapter(&book, 0, "QQQONE");
    install(|step| {
        if step == Step::HistoryWriting {
            Err(io::Error::other("injected"))
        } else {
            Ok(())
        }
    });
    edit_chapter(&book, 1, "QQQTWO");
    install(|_| Ok(()));
    edit_chapter(&book, 0, "QQQTHREE");

    assert_eq!(
        book.prune_history(later_than_now(), false).unwrap().points,
        1
    );
    let start = named_id(&book, "start");
    assert_eq!(book.history().unwrap().len(), 2);
    assert!(!book.history_changes(start).unwrap().complete);
    let refused = book
        .restore_point(start, RevertTarget::WholeBook)
        .unwrap_err();
    assert!(matches!(refused, Error::NotEditable(_)), "{refused:?}");
    book.restore_point(start, RevertTarget::Chapter(first.clone()))
        .unwrap();
    assert_eq!(chapter_bytes(&book, &first), original[first.as_str()]);
}
#[test]
fn a_failure_at_any_prune_step_leaves_a_log_whose_kept_points_all_restore() {
    for step in [
        Step::PruneWriting,
        Step::PruneWritten,
        Step::PruneSynced,
        Step::PruneRenamed,
        Step::PruneDirSynced,
    ] {
        let scratch = Scratch::new("prune-book-kill");
        scratch.place(&fixture(SMALL));
        let original = entries_of(&scratch.book_path());
        let book = scratch.open();
        book.take_snapshot("start").unwrap();
        edit_chapter(&book, 0, "QQQONE");
        edit_chapter(&book, 1, "QQQTWO");
        edit_chapter(&book, 0, "QQQTHREE");
        let cutoff = later_than_now();
        let old_log = fs::read(history_log_in(&scratch.data())).unwrap();
        let reference = scratch.0.join("reference");
        fs::create_dir_all(history_log_in(&reference).parent().unwrap()).unwrap();
        fs::write(history_log_in(&reference), &old_log).unwrap();
        let done =
            crate::storage::prune_closed(&reference, KEY, &scratch.book_path(), cutoff, false)
                .unwrap();
        assert!(matches!(done, crate::storage::Pruning::Done(pruned) if pruned.points == 2));
        let new_log = fs::read(history_log_in(&reference)).unwrap();

        fail_at(step);
        let result = book.prune_history(cutoff, false);
        install(|_| Ok(()));
        let renamed = matches!(step, Step::PruneRenamed | Step::PruneDirSynced);
        assert_eq!(result.is_ok(), renamed, "{step:?}");
        let on_disk = fs::read(history_log_in(&scratch.data())).unwrap();
        assert_eq!(
            &on_disk,
            if renamed { &new_log } else { &old_log },
            "{step:?}"
        );
        assert_eq!(
            book.history().unwrap().len(),
            if renamed { 2 } else { 4 },
            "{step:?}"
        );
        drop(book);

        let reopened = scratch.open();
        let start = named_id(&reopened, "start");
        reopened
            .restore_point(start, RevertTarget::WholeBook)
            .unwrap();
        assert_eq!(entries_of(&scratch.book_path()), original, "{step:?}");
        reopened.prune_history(cutoff, false).unwrap();
        edit_first_chapter(&reopened, "again");
        assert_eq!(reopened.history().unwrap()[0].kind, PointKind::Save);
    }
}
#[test]
fn a_book_open_elsewhere_reads_the_pruned_log_and_keeps_saving() {
    let scratch = Scratch::new("prune-book-elsewhere");
    scratch.place(&fixture(SMALL));
    let original = entries_of(&scratch.book_path());
    let book = scratch.open();
    book.take_snapshot("start").unwrap();
    edit_chapter(&book, 0, "QQQONE");
    edit_chapter(&book, 1, "QQQTWO");
    edit_chapter(&book, 0, "QQQTHREE");
    assert_eq!(book.history().unwrap().len(), 4);

    let elsewhere = crate::storage::prune_closed(
        &scratch.data(),
        KEY,
        &scratch.book_path(),
        later_than_now(),
        false,
    )
    .unwrap();
    assert!(matches!(elsewhere, crate::storage::Pruning::Done(pruned) if pruned.points == 2));

    assert_eq!(book.history().unwrap().len(), 2);
    edit_chapter(&book, 1, "QQQFOUR");
    assert_eq!(book.history().unwrap().len(), 3);
    let start = named_id(&book, "start");
    book.restore_point(start, RevertTarget::WholeBook).unwrap();
    assert_eq!(entries_of(&scratch.book_path()), original);
    drop(book);
    assert_eq!(scratch.open().history().unwrap().len(), 4);
}
#[test]
fn a_closed_book_is_skipped_while_a_save_holds_it_or_its_file_is_gone() {
    let scratch = Scratch::new("prune-closed-skip");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_chapter(&book, 0, "QQQONE");
    edit_chapter(&book, 0, "QQQTWO");
    drop(book);
    let log = fs::read(history_log_in(&scratch.data())).unwrap();
    let held = fs::File::options()
        .write(true)
        .open(
            scratch
                .data()
                .join(format!("books/{:032x}/commit.lock", KEY.0)),
        )
        .unwrap();
    held.lock().unwrap();
    let prune = |path: &Path| {
        crate::storage::prune_closed(&scratch.data(), KEY, path, later_than_now(), false).unwrap()
    };
    assert_eq!(
        prune(&scratch.book_path()),
        crate::storage::Pruning::Skipped
    );
    let refused = crate::storage::delete_backups(&scratch.data(), KEY).unwrap_err();
    assert!(matches!(refused, Error::NotEditable(_)), "{refused:?}");
    assert_eq!(scratch.slot_dirs().len(), 1);
    drop(held);
    assert_eq!(
        prune(&scratch.0.join("books/missing.epub")),
        crate::storage::Pruning::Skipped
    );
    assert_eq!(fs::read(history_log_in(&scratch.data())).unwrap(), log);
    assert!(
        matches!(prune(&scratch.book_path()), crate::storage::Pruning::Done(pruned) if pruned.points == 1)
    );
    assert_eq!(
        crate::storage::prune_closed(
            &scratch.0.join("no-data"),
            KEY,
            &scratch.book_path(),
            0,
            false
        )
        .unwrap(),
        crate::storage::Pruning::Done(crate::Pruned::default())
    );
}
#[test]
fn deleting_the_backup_frees_its_bytes_and_the_next_edit_starts_a_new_one() {
    let scratch = Scratch::new("delete-backup-open");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_chapter(&book, 0, "QQQONE");
    let after_one = entries_of(&scratch.book_path());
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    let backups = scratch.data().join(format!("books/{:032x}/backups", KEY.0));
    let size = tree_size(&backups);
    assert!(size > 0);

    assert_eq!(book.delete_backups().unwrap(), size);
    assert!(!backups.exists());
    assert!(matches!(book.backup_status().unwrap(), BackupStatus::None));
    assert!(book.revert(RevertTarget::WholeBook).is_err());
    assert!(!book.data_has_any_slot());

    edit_chapter(&book, 0, "QQQTWO");
    assert!(matches!(
        book.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(
        entries_of(&scratch.book_path()),
        after_one,
        "the new backup is the file as it was before the next edit"
    );
    let size = tree_size(&backups);
    assert_eq!(book.delete_backups().unwrap(), size);
    drop(book);
    assert!(matches!(
        scratch.open().backup_status().unwrap(),
        BackupStatus::None
    ));
}
#[test]
fn deleting_a_closed_books_backup_leaves_no_backup_to_find_on_the_next_open() {
    let scratch = Scratch::new("delete-backup-closed");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_chapter(&book, 0, "QQQONE");
    drop(book);
    let backups = scratch.data().join(format!("books/{:032x}/backups", KEY.0));
    let size = tree_size(&backups);
    assert_eq!(
        crate::storage::delete_backups(&scratch.data(), KEY).unwrap(),
        size
    );
    assert_eq!(
        crate::storage::delete_backups(&scratch.data(), KEY).unwrap(),
        0
    );
    let reopened = scratch.open();
    assert!(matches!(
        reopened.backup_status().unwrap(),
        BackupStatus::None
    ));
    edit_chapter(&reopened, 0, "QQQTWO");
    assert!(matches!(
        reopened.backup_status().unwrap(),
        BackupStatus::Valid { .. }
    ));
}
#[test]
fn usage_counts_every_file_once_and_splits_each_book_into_history_backup_and_the_rest() {
    let scratch = Scratch::new("storage-usage");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    book.take_snapshot("start").unwrap();
    edit_chapter(&book, 0, "QQQONE");
    edit_chapter(&book, 1, "QQQTWO");
    drop(book);
    let data = scratch.data();
    let root = data.join(format!("books/{:032x}", KEY.0));
    fs::write(root.join("marks.json"), b"{\"marks\":[]}").unwrap();
    fs::write(data.join("prefs.json"), b"{}").unwrap();
    fs::create_dir_all(data.join("books/not-a-key")).unwrap();
    fs::write(data.join("books/not-a-key/stray"), b"stray").unwrap();
    let total = tree_size(&data);
    std::os::unix::fs::symlink(scratch.book_path(), data.join("linked.epub")).unwrap();
    std::os::unix::fs::symlink(&root, data.join("linked-dir")).unwrap();

    let usage = crate::storage::usage(&data);
    assert_eq!(usage.total, total, "links are neither followed nor counted");
    assert_eq!(
        usage.books,
        [crate::storage::BookUsage {
            key: KEY,
            history: tree_size(&root.join("history")),
            backup: tree_size(&root.join("backups")),
            other: tree_size(&root)
                - tree_size(&root.join("history"))
                - tree_size(&root.join("backups")),
        }]
    );
    assert!(usage.books[0].other >= 12, "marks.json counts as other");
    assert_eq!(
        crate::storage::history_points(&data, KEY),
        crate::storage::PointCount {
            automatic: 2,
            named: 1
        }
    );
    assert_eq!(
        crate::storage::history_points(&data, BookKey(7)),
        crate::storage::PointCount::default()
    );
    assert_eq!(
        crate::storage::usage(&scratch.0.join("nowhere")),
        Default::default()
    );
}

#[test]
fn deleting_backups_from_another_window_does_not_stop_this_window_saving() {
    let scratch = Scratch::new("backups-deleted-elsewhere");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "QQQONE");
    crate::storage::delete_backups(&scratch.data(), KEY).unwrap();
    let target = first_document(&book.read());
    let entry = NewEntry::deflate(target.name.clone(), &with_marker(&target.bytes, "QQQTWO"));
    let first = book.save_entries(vec![entry]);
    let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, "QQQTHREE"));
    let second = book.save_entries(vec![entry]);
    assert!(
        first.is_ok() && second.is_ok(),
        "saves after another window deleted the backups: {:?} / {:?}",
        first.map(|_| ()),
        second.map(|_| ())
    );
    assert!(
        matches!(book.backup_status(), Ok(BackupStatus::Valid { .. })),
        "no fresh backup was made after the old one was deleted"
    );
}

#[test]
fn backups_deleted_from_another_window_are_not_offered_for_revert() {
    let scratch = Scratch::new("backups-deleted-status");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "QQQONE");
    assert!(matches!(
        book.backup_status(),
        Ok(BackupStatus::Valid { .. })
    ));
    crate::storage::delete_backups(&scratch.data(), KEY).unwrap();
    let status = book.backup_status();
    assert!(
        matches!(status, Ok(BackupStatus::None)),
        "status after another window deleted the backups still offers a backup: {}",
        matches!(status, Ok(BackupStatus::Valid { .. }))
    );
}

#[test]
fn opening_a_book_whose_long_name_shares_its_start_keeps_this_books_save_in_progress() {
    let scratch = Scratch::new("long-shared-name");
    let stem = "\u{7b2c}".repeat(34);
    let mine = scratch.0.join(format!("books/{stem}1.epub"));
    let other = scratch.0.join(format!("books/{stem}2.epub"));
    fs::write(&mine, fs::read(fixture(SMALL)).unwrap()).unwrap();
    fs::write(&other, fs::read(fixture(SMALL)).unwrap()).unwrap();
    let book = Book::open(&mine, &scratch.data(), KEY, DEFAULT_LIMITS).unwrap();
    let other_data = scratch.0.join("other-data");
    let other_path = other.clone();
    let mut opened = false;
    install(move |step| {
        if step == Step::TempVerified && !opened {
            opened = true;
            let _ = Book::open(&other_path, &other_data, BookKey(0x7777), DEFAULT_LIMITS);
        }
        Ok(())
    });
    let target = first_document(&book.read());
    let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, "QQQMINE"));
    let result = book.save_entries(vec![entry]);
    install(|_| Ok(()));
    assert!(
        result.is_ok(),
        "opening another book in the same folder removed this save's temp: {:?}",
        result.map(|_| ())
    );
}

#[test]
fn a_same_size_outside_edit_that_keeps_its_mtime_is_not_overwritten() {
    use std::os::unix::fs::FileExt;
    for saved_first in [false, true] {
        let scratch = Scratch::new("same-size-outside-edit");
        let path = scratch.place(&fixture(SMALL));
        let book = scratch.open();
        if saved_first {
            edit_first_chapter(&book, "edited");
        }
        let target = first_document(&book.read());
        let range = {
            let reader = book.read();
            let row = reader
                .rows()
                .iter()
                .find(|row| row.href.as_ref() == Some(&target.name))
                .unwrap();
            reader.entry_record_range(row.ix)
        };
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let outside = File::options().write(true).open(&path).unwrap();
        let at = (range.start + range.end) / 2;
        let mut byte = [0u8; 1];
        FileExt::read_exact_at(&File::open(&path).unwrap(), &mut byte, at).unwrap();
        outside.write_all_at(&[byte[0] ^ 0xff], at).unwrap();
        outside
            .set_times(fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        drop(outside);
        let outside_bytes = fs::read(&path).unwrap();
        let entry = NewEntry::deflate(target.name, &with_marker(&target.bytes, "QQQMINE"));
        let result = book.save_entries(vec![entry]);
        assert!(
            matches!(result, Err(Error::ChangedOnDisk)),
            "saved first = {saved_first}: the outside edit was replaced without a warning: {:?}, file still holds outside edit = {}",
            result.map(|report| report.bytes_written),
            fs::read(&path).unwrap() == outside_bytes
        );
        assert_eq!(fs::read(&path).unwrap(), outside_bytes);
    }
}

fn lose_a_save(book: &Book) {
    fail_at(Step::StampRechecked);
    let target = editable_chapters(&book.read()).swap_remove(0);
    let lost = NewEntry::deflate(target.name, &with_marker(&target.bytes, "QQQLOST"));
    assert!(book.save_entries(vec![lost]).is_err());
}

fn clean_up_elsewhere(scratch: &Scratch, cutoff_ms: u64) -> Pruned {
    match crate::storage::prune_closed(&scratch.data(), KEY, &scratch.book_path(), cutoff_ms, false)
        .unwrap()
    {
        crate::storage::Pruning::Done(pruned) => pruned,
        crate::storage::Pruning::Skipped => panic!("the cleanup was skipped"),
    }
}

#[test]
fn a_point_listed_before_a_cleanup_elsewhere_restores_that_point() {
    let scratch = Scratch::new("stale-point-restore");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = editable_chapters(&book.read())[0].name.clone();
    edit_chapter(&book, 0, "QQQONE");
    let after_one = chapter_bytes(&book, &name);
    lose_a_save(&book);
    edit_chapter(&book, 0, "QQQTWO");
    edit_chapter(&book, 0, "QQQTHREE");
    let listed = book.history().unwrap();
    assert_eq!(listed.len(), 3);
    let two = listed[1].id;

    assert_eq!(clean_up_elsewhere(&scratch, 0).points, 0);
    book.restore_point(two, RevertTarget::Chapter(name.clone()))
        .unwrap();
    let now = String::from_utf8_lossy(&chapter_bytes(&book, &name)).into_owned();
    assert!(
        chapter_bytes(&book, &name) == after_one,
        "restoring the point listed as the QQQTWO save brought back the version before QQQTHREE instead: QQQTWO present = {}",
        now.contains("QQQTWO")
    );
}

#[test]
fn a_point_listed_before_a_cleanup_elsewhere_keeps_its_changes_and_diff() {
    let scratch = Scratch::new("stale-point-changes");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = editable_chapters(&book.read())[0].name.clone();
    edit_chapter(&book, 0, "QQQONE");
    lose_a_save(&book);
    edit_chapter(&book, 0, "QQQTWO");
    edit_chapter(&book, 1, "QQQTHREE");
    let two = book.history().unwrap()[1].id;
    let changes = book.history_changes(two).unwrap();
    let diff = book.history_diff(two, &name).unwrap();
    assert_eq!(changes.rows.len(), 2);

    assert_eq!(clean_up_elsewhere(&scratch, 0).points, 0);
    assert_eq!(book.history_changes(two).unwrap(), changes);
    assert_eq!(book.history_diff(two, &name).unwrap(), diff);
}

#[test]
fn a_point_cleaned_up_elsewhere_is_refused_not_swapped_for_another() {
    let scratch = Scratch::new("dropped-point");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    let name = editable_chapters(&book.read())[0].name.clone();
    book.take_snapshot("start").unwrap();
    edit_chapter(&book, 0, "QQQONE");
    edit_chapter(&book, 0, "QQQTWO");
    edit_chapter(&book, 0, "QQQTHREE");
    let one = book.history().unwrap()[2].id;
    let before = entries_of(&scratch.book_path());

    assert_eq!(clean_up_elsewhere(&scratch, later_than_now()).points, 2);
    for refused in [
        book.history_changes(one).map(drop),
        book.history_diff(one, &name).map(drop),
        book.restore_point(one, RevertTarget::Chapter(name.clone()))
            .map(drop),
        book.restore_point(one, RevertTarget::WholeBook).map(drop),
    ] {
        assert!(matches!(refused, Err(Error::NotEditable(_))), "{refused:?}");
    }
    assert_eq!(entries_of(&scratch.book_path()), before);
}

#[test]
fn a_symlink_at_a_backup_temp_name_does_not_redirect_the_write_outside() {
    let scratch = Scratch::new("backup-temp-link");
    scratch.place(&fixture(SMALL));
    let book = scratch.open();
    edit_first_chapter(&book, "QQQONE");
    let outside = scratch.0.join("precious");
    fs::write(&outside, b"precious").unwrap();
    let mut slots = vec![scratch.data()];
    while let Some(dir) = slots.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                slots.push(path);
            } else if path.file_name() == Some("ledger.json".as_ref()) {
                std::os::unix::fs::symlink(&outside, path.with_file_name("ledger.json.tmp"))
                    .unwrap();
            }
        }
    }
    edit_first_chapter(&book, "QQQTWO");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");
}
