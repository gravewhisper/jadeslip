use super::*;
use crate::backup::hash_of;
use crate::common::fixture;
use crate::zip::ZipIndex;
use crate::{
    BackupStatus, CancelToken, DEFAULT_LIMITS, Query, RevertTarget, SearchEnd, SearchEvent,
    SearchOptions, SearchScope,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/manage-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("data")).unwrap();
        Scratch(dir)
    }

    fn book(&self) -> PathBuf {
        self.0.join("book.epub")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn place(&self, source: &Path) -> Book {
        fs::copy(source, self.book()).unwrap();
        let mut perms = fs::metadata(self.book()).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(self.book(), perms).unwrap();
        self.open()
    }

    fn open(&self) -> Book {
        Book::open_by_fingerprint(&self.book(), &self.data(), DEFAULT_LIMITS).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validator() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/epub_structure.py")
}

fn structure(path: &Path) -> Value {
    let output = Command::new("python3")
        .arg("-I")
        .arg(validator())
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

struct PackageFiles {
    opf: EntryName,
    nav: Option<EntryName>,
    ncx: Option<EntryName>,
}

fn package_files(reader: &Reader) -> PackageFiles {
    PackageFiles {
        opf: reader.opf_entry().clone(),
        nav: reader.nav_name().cloned(),
        ncx: reader.ncx_name().cloned(),
    }
}

struct Mem<'b>(&'b [u8]);

impl ReadAt for Mem<'_> {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        let at = offset as usize;
        buf.copy_from_slice(&self.0[at..at + buf.len()]);
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.0.len() as u64)
    }
}

fn records_of(bytes: &[u8]) -> BTreeMap<EntryName, Vec<u8>> {
    let index = ZipIndex::parse(&Mem(bytes), &DEFAULT_LIMITS).unwrap();
    index
        .entries()
        .iter()
        .map(|entry| {
            let range = entry.record.start as usize..entry.record.end as usize;
            (entry.name.clone(), bytes[range].to_vec())
        })
        .collect()
}

fn entry_bytes(bytes: &[u8], name: &EntryName) -> Vec<u8> {
    let index = ZipIndex::parse(&Mem(bytes), &DEFAULT_LIMITS).unwrap();
    index
        .read_entry(&Mem(bytes), index.entry(name).unwrap(), &DEFAULT_LIMITS)
        .unwrap()
}

fn assert_parses(bytes: &[u8], what: &str, label: &str) {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    assert!(
        roxmltree::Document::parse_with_options(text, options).is_ok(),
        "{label}: {what} does not parse"
    );
}

fn order(reader: &Reader) -> Vec<Option<String>> {
    reader
        .rows()
        .iter()
        .map(|row| row.href.as_ref().map(|name| name.as_str().to_owned()))
        .collect()
}

#[derive(Default)]
struct Tally {
    applied: BTreeMap<&'static str, u32>,
    refused: BTreeMap<&'static str, u32>,
    moved_groups: u32,
    reasons: BTreeMap<String, u32>,
}

fn kind_of(op: &ChapterOp) -> &'static str {
    match op {
        ChapterOp::Add { .. } => "add",
        ChapterOp::Rename { .. } => "rename",
        ChapterOp::Move { .. } => "move",
        ChapterOp::Delete { .. } => "delete",
        ChapterOp::UndoDelete { .. } => "undo",
    }
}

struct TocShape {
    rows: Vec<Option<u32>>,
    depths: Vec<u16>,
}

fn toc_shape(reader: &Reader) -> Option<TocShape> {
    let toc = reader.toc().unwrap();
    if toc.source == crate::TocSource::HeadingsPending {
        return None;
    }
    Some(TocShape {
        rows: toc
            .entries
            .iter()
            .map(|entry| entry.target.map(|ix| ix.0))
            .collect(),
        depths: toc.entries.iter().map(|entry| entry.depth).collect(),
    })
}

fn expected_block(reader: &Reader, row: RowRef) -> BTreeSet<u32> {
    let shape = toc_shape(reader);
    let subtree = |shape: &TocShape, at: usize| -> Vec<u32> {
        let mut end = at + 1;
        while end < shape.depths.len() && shape.depths[end] > shape.depths[at] {
            end += 1;
        }
        shape.rows[at..end].iter().flatten().copied().collect()
    };
    let mut moved = BTreeSet::new();
    let mut queue = Vec::new();
    match row {
        RowRef::Chapter(ix) => queue.push(ix.0),
        RowRef::Entry(at) => {
            let shape = shape.as_ref().unwrap();
            queue.extend(subtree(shape, at as usize));
        }
    }
    moved.extend(queue.iter().copied());
    if let Some(shape) = &shape {
        while let Some(found) = queue.pop() {
            for (at, target) in shape.rows.iter().enumerate() {
                if *target == Some(found) {
                    for inner in subtree(shape, at) {
                        if moved.insert(inner) {
                            queue.push(inner);
                        }
                    }
                }
            }
        }
        let listed: BTreeSet<u32> = shape.rows.iter().flatten().copied().collect();
        for row in moved.clone() {
            if !listed.contains(&row) {
                continue;
            }
            let mut next = row + 1;
            while (next as usize) < reader.rows().len()
                && !listed.contains(&next)
                && !reader.rows()[next as usize].is_nav
            {
                moved.insert(next);
                next += 1;
            }
        }
    }
    moved
}

fn pick_row(reader: &Reader, rng: &mut Rng) -> RowRef {
    let shape = toc_shape(reader);
    if rng.below(2) == 0
        && let Some(shape) = shape
        && !shape.rows.is_empty()
    {
        return RowRef::Entry(rng.below(shape.rows.len()) as u32);
    }
    RowRef::Chapter(ChapterIx(rng.below(reader.rows().len()) as u32))
}

fn random_op(book: &Book, rng: &mut Rng, undo: &mut Vec<DeletedId>, serial: &mut u32) -> ChapterOp {
    let reader = book.read();
    let rows = reader.rows().len();
    *serial += 1;
    match rng.below(10) {
        0 | 1 => ChapterOp::Add {
            after: (rng.below(5) != 0).then(|| ChapterIx(rng.below(rows) as u32)),
            title: format!("Added <{serial}> & \"quoted\""),
        },
        2 | 3 => ChapterOp::Rename {
            row: pick_row(&reader, rng),
            title: format!("Renamed <{serial}> & done"),
        },
        4..=7 => ChapterOp::Move {
            row: pick_row(&reader, rng),
            to: Spot {
                row: pick_row(&reader, rng),
                after: rng.below(2) == 0,
            },
        },
        8 => ChapterOp::Delete {
            chapter: ChapterIx(rng.below(rows) as u32),
        },
        _ => match undo.pop() {
            Some(deleted) => ChapterOp::UndoDelete { deleted },
            None => ChapterOp::Delete {
                chapter: ChapterIx(rng.below(rows) as u32),
            },
        },
    }
}

fn run_sequence(source: &Path, label: &str, steps: u32, seed: u64) -> Tally {
    let scratch = Scratch::new("seq");
    let book = scratch.place(source);
    let original = fs::read(scratch.book()).unwrap();
    let original_hash = hash_of(&File::open(scratch.book()).unwrap()).unwrap();
    let identity = book.identity();
    let baseline = structure(&scratch.book());
    let baseline_problems = strings(&baseline["problems"]);
    let nav_was_ordered = baseline["nav_ordered"].as_bool();
    let ncx_was_ordered = baseline["ncx_ordered"].as_bool();
    let mut rng = Rng(seed);
    let mut undo = Vec::new();
    let mut serial = 0;
    let mut tally = Tally::default();
    for step in 0..steps {
        let before = book.read();
        let prior = fs::read(scratch.book()).unwrap();
        let op = random_op(&book, &mut rng, &mut undo, &mut serial);
        let prior_order = order(&before);
        let expected_moved = match &op {
            ChapterOp::Move { row, .. } => Some(expected_block(&before, *row)),
            _ => None,
        };
        let kind = kind_of(&op);
        let context = format!("{label} step {step} {kind}");
        let outcome = book.manage(op.clone(), None);
        let managed = match outcome {
            Ok(managed) => managed,
            Err(error) => {
                assert!(
                    !error.user_message().is_empty(),
                    "{context}: refusal has no message"
                );
                assert_eq!(
                    fs::read(scratch.book()).unwrap(),
                    prior,
                    "{context}: a refused operation changed the file"
                );
                *tally.refused.entry(kind).or_default() += 1;
                *tally.reasons.entry(error.user_message()).or_default() += 1;
                continue;
            }
        };
        *tally.applied.entry(kind).or_default() += 1;
        if let Some(deleted) = managed.deleted {
            undo.push(deleted);
        }
        let after = book.read();
        let current = fs::read(scratch.book()).unwrap();
        assert_eq!(book.identity(), identity, "{context}: identity moved");
        let reopened = Reader::open(File::open(scratch.book()).unwrap(), DEFAULT_LIMITS).unwrap();
        assert_eq!(order(&reopened), order(&after), "{context}: reopen differs");
        after.check_no_new_problems(&before, &context);
        let files = package_files(&after);
        for name in [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
            .into_iter()
            .flatten()
        {
            assert_parses(&entry_bytes(&current, name), name.as_str(), &context);
        }
        let toc = after.toc().unwrap();
        if toc.source != crate::TocSource::HeadingsPending {
            assert!(
                toc.entries.iter().any(|entry| entry.target.is_some()),
                "{context}: the contents lost every target"
            );
        }
        let shape = structure(&scratch.book());
        let fresh: Vec<_> = strings(&shape["problems"])
            .difference(&baseline_problems)
            .cloned()
            .collect();
        assert!(fresh.is_empty(), "{context}: validator found {fresh:?}");
        if nav_was_ordered == Some(true) {
            assert_eq!(
                shape["nav_ordered"],
                Value::Bool(true),
                "{context}: nav order"
            );
        }
        if ncx_was_ordered == Some(true) {
            assert_eq!(
                shape["ncx_ordered"],
                Value::Bool(true),
                "{context}: ncx order"
            );
        }

        let mut allowed: BTreeSet<EntryName> =
            [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
        allowed.insert(EntryName::parse("mimetype").unwrap());
        if let ChapterOp::Rename { .. } = op
            && let Some(landing) = managed.landing
            && let Some(name) = &after.rows()[landing.0 as usize].href
        {
            allowed.insert(name.clone());
        }
        let (old_records, new_records) = (records_of(&prior), records_of(&current));
        let mut added = 0;
        let mut removed = 0;
        for (name, record) in &new_records {
            match old_records.get(name) {
                None => added += 1,
                Some(old) => {
                    if !allowed.contains(name) && old != record {
                        assert!(
                            matches!(op, ChapterOp::Delete { .. } | ChapterOp::UndoDelete { .. }),
                            "{context}: an unrelated record changed"
                        );
                        let was = entry_bytes(&prior, name);
                        let now = entry_bytes(&current, name);
                        let holds_link = |bytes: &[u8]| {
                            String::from_utf8_lossy(bytes)
                                .to_lowercase()
                                .contains("href")
                        };
                        assert!(
                            holds_link(&was) || holds_link(&now),
                            "{context}: a chapter with no link changed"
                        );
                        let text = |bytes: Vec<u8>| {
                            crate::chapter::decode_and_model(bytes, &DEFAULT_LIMITS)
                                .unwrap()
                                .text
                                .as_str()
                                .to_owned()
                        };
                        assert_eq!(
                            text(was),
                            text(now),
                            "{context}: a link fix changed the text"
                        );
                    }
                }
            }
        }
        for name in old_records.keys() {
            if !new_records.contains_key(name) {
                removed += 1;
            }
        }
        match &op {
            ChapterOp::Add { after: wanted, .. } => {
                assert_eq!((added, removed), (1, 0), "{context}: records");
                let at = managed.landing.unwrap().0 as usize;
                let mut expected = prior_order.clone();
                expected.insert(at, order(&after)[at].clone());
                assert_eq!(order(&after), expected, "{context}: read order");
                if let Some(wanted) = wanted {
                    assert!(
                        at > wanted.0 as usize,
                        "{context}: placed before its anchor"
                    );
                }
                let rendered = after.render(ChapterIx(at as u32)).unwrap();
                assert!(
                    rendered.title.unwrap().contains("Added"),
                    "{context}: title"
                );
            }
            ChapterOp::Rename { .. } => {
                assert_eq!((added, removed), (0, 0), "{context}: records");
                assert_eq!(order(&after), prior_order, "{context}: read order");
            }
            ChapterOp::Move { .. } => {
                assert_eq!((added, removed), (0, 0), "{context}: records");
                let block = expected_moved.unwrap();
                if block.len() > 1 {
                    tally.moved_groups += 1;
                }
                let now = order(&after);
                let mut left: Vec<_> = prior_order
                    .iter()
                    .enumerate()
                    .filter(|(at, _)| !block.contains(&(*at as u32)))
                    .map(|(_, row)| row.clone())
                    .collect();
                let moved: Vec<_> = block
                    .iter()
                    .map(|&at| prior_order[at as usize].clone())
                    .collect();
                let spot = now
                    .windows(moved.len().max(1))
                    .position(|window| window == moved.as_slice());
                let spot =
                    spot.unwrap_or_else(|| panic!("{context}: the block did not stay together"));
                left.splice(spot..spot, moved);
                assert_eq!(now, left, "{context}: read order");
            }
            ChapterOp::Delete { chapter } => {
                let mut expected = prior_order.clone();
                expected.remove(chapter.0 as usize);
                assert_eq!(order(&after), expected, "{context}: read order");
                assert!(removed <= 1 && added == 0, "{context}: records");
            }
            ChapterOp::UndoDelete { .. } => {
                assert_eq!((added, removed), (1, 0), "{context}: records");
                assert_eq!(
                    order(&after).len(),
                    prior_order.len() + 1,
                    "{context}: rows"
                );
            }
        }
    }
    let rows_before_reopen = order(&book.read());
    drop(book);
    let again = scratch.open();
    assert_eq!(
        again.identity(),
        identity,
        "{label}: a new launch lost the identity"
    );
    assert_eq!(order(&again.read()), rows_before_reopen, "{label}: reopen");
    if tally.applied.values().sum::<u32>() > 0 {
        assert!(
            matches!(again.backup_status().unwrap(), BackupStatus::Valid { .. }),
            "{label}: a later launch lost the backup"
        );
        again.revert(RevertTarget::WholeBook).unwrap();
        assert_eq!(
            hash_of(&File::open(scratch.book()).unwrap()).unwrap(),
            original_hash,
            "{label}: revert did not restore the hash"
        );
        assert_eq!(
            fs::read(scratch.book()).unwrap(),
            original,
            "{label}: revert bytes"
        );
        assert_eq!(again.identity(), identity, "{label}: identity after revert");
    }
    tally
}

fn report(label: &str, tally: &Tally) {
    println!(
        "{label}: applied {:?} refused {:?} groups {} reasons {:?}",
        tally.applied, tally.refused, tally.moved_groups, tally.reasons
    );
}

const SHAPES: &[&str] = &[
    "toc-nav-only",
    "toc-ncx-only",
    "toc-none",
    "toc-nested-nav",
    "toc-nested-ncx",
    "images-mixed",
    "reader-css",
    "gbk-xml-decl",
    "gbk-meta-charset",
    "malformed-spine-ref",
    "malformed-missing-chapter",
    "mimetype-not-first",
    "toc-hostile",
    "headings-hostile",
];

#[test]
fn random_sequences_keep_every_generated_shape_valid() {
    for &name in SHAPES {
        for seed in [11, 0x5eed, 0xdead_beef] {
            let tally = run_sequence(&fixture(name), name, 40, seed);
            report(name, &tally);
            assert!(
                tally.applied.values().sum::<u32>() > 10,
                "{name}: too few ops applied"
            );
        }
    }
}

fn meta(title: &str) -> NewBookMeta {
    NewBookMeta {
        title: title.to_owned(),
        author: "An Author".to_owned(),
        language: "en".to_owned(),
        first_chapter: "One".to_owned(),
    }
}

#[test]
fn random_sequences_keep_a_new_book_valid_with_both_contents() {
    let scratch = Scratch::new("new-both");
    let path = scratch.0.join("made.epub");
    let book = new_book(&path, &scratch.data(), &meta("Made"), DEFAULT_LIMITS).unwrap();
    drop(book);
    let shape = structure(&path);
    assert_eq!(strings(&shape["problems"]), BTreeSet::new());
    assert_eq!(shape["nav_links"], 1);
    assert_eq!(shape["ncx_links"], 1);
    for seed in [3, 77, 901] {
        let tally = run_sequence(&path, "new-both", 60, seed);
        report("new-both", &tally);
    }
}

fn real_books() -> Vec<PathBuf> {
    let mut books: Vec<PathBuf> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "epub"))
            .collect();
    books.sort();
    books
}

fn label_of(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy();
    stem.rsplit('-').next().unwrap().to_owned()
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core manage::tests::real -- --ignored --nocapture"]
fn real_book_random_sequences_stay_valid() {
    for path in real_books() {
        let label = label_of(&path);
        let size = fs::metadata(&path).unwrap().len();
        let steps = if size > 15 << 20 { 18 } else { 40 };
        let tally = run_sequence(&path, &label, steps, 0x1234_5678 ^ size);
        report(&label, &tally);
        assert!(
            tally.applied.values().sum::<u32>() > steps / 3,
            "{label}: too few applied"
        );
    }
}

#[test]
fn delete_then_undo_returns_every_package_file_and_the_chapter_byte_equal() {
    for &name in SHAPES {
        let scratch = Scratch::new("undo");
        let book = scratch.place(&fixture(name));
        let reader = book.read();
        if reader.rows().len() < 2 {
            continue;
        }
        let row = (reader.rows().len() / 2..reader.rows().len())
            .chain(0..reader.rows().len() / 2)
            .find(|&at| !reader.rows()[at].is_nav && reader.rows()[at].rev.is_some())
            .unwrap();
        let first_save = fs::read(scratch.book()).unwrap();
        let name_of = reader.rows()[row].href.clone().unwrap();
        let before_bytes = entry_bytes(&first_save, &name_of);
        let before_order = order(&reader);
        let files = package_files(&reader);
        drop(reader);
        let managed = book
            .manage(
                ChapterOp::Delete {
                    chapter: ChapterIx(row as u32),
                },
                None,
            )
            .unwrap();
        let id = managed.deleted.unwrap();
        let listed = match book.backup_status().unwrap() {
            BackupStatus::Valid { deleted, .. } => deleted,
            _ => panic!("{name}: no valid backup after a delete"),
        };
        assert_eq!(
            listed.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![id]
        );
        assert_ne!(
            order(&book.read()),
            before_order,
            "{name}: delete changed nothing"
        );
        let undone = book
            .manage(ChapterOp::UndoDelete { deleted: id }, None)
            .unwrap();
        assert_eq!(
            undone.landing,
            Some(ChapterIx(row as u32)),
            "{name}: landing"
        );
        let now = fs::read(scratch.book()).unwrap();
        assert_eq!(
            entry_bytes(&now, &name_of),
            before_bytes,
            "{name}: chapter bytes"
        );
        assert_eq!(
            order(&book.read()),
            before_order,
            "{name}: chapter not in place"
        );
        for doc in [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
            .into_iter()
            .flatten()
        {
            assert_eq!(
                entry_bytes(&now, doc),
                entry_bytes(&first_save, doc),
                "{name}: {} not restored",
                doc.as_str()
            );
        }
        match book.backup_status().unwrap() {
            BackupStatus::Valid { deleted, .. } => {
                assert!(deleted.is_empty(), "{name}: record stays")
            }
            _ => panic!("{name}: backup lost"),
        }
    }
}

#[test]
fn undo_after_later_operations_still_restores_the_chapter_near_its_place() {
    for &name in &["toc-nav-only", "toc-ncx-only", "toc-nested-nav", "toc-none"] {
        let scratch = Scratch::new("undo-late");
        let book = scratch.place(&fixture(name));
        let target = book.read().rows()[2].href.clone().unwrap();
        let before_bytes = entry_bytes(&fs::read(scratch.book()).unwrap(), &target);
        let id = book
            .manage(
                ChapterOp::Delete {
                    chapter: ChapterIx(2),
                },
                None,
            )
            .unwrap()
            .deleted
            .unwrap();
        book.manage(
            ChapterOp::Add {
                after: Some(ChapterIx(3)),
                title: "Later".to_owned(),
            },
            None,
        )
        .unwrap();
        let undone = book
            .manage(ChapterOp::UndoDelete { deleted: id }, None)
            .unwrap();
        let reader = book.read();
        assert_eq!(undone.landing, Some(ChapterIx(2)), "{name}: place");
        assert_eq!(
            entry_bytes(&fs::read(scratch.book()).unwrap(), &target),
            before_bytes,
            "{name}: bytes"
        );
        reader.check_whole_package_for_test();
        let shape = structure(&scratch.book());
        assert!(
            strings(&shape["problems"])
                .iter()
                .all(|problem| problem.contains("EPUB3") || problem.contains("no target")),
            "{name}: {shape}"
        );
        if name != "toc-none" {
            let toc = reader.toc().unwrap();
            assert!(
                toc.entries
                    .iter()
                    .any(|entry| entry.target == Some(ChapterIx(2))),
                "{name}: the contents do not list it again"
            );
        }
    }
}

#[test]
fn a_new_id_and_file_never_collide_with_what_is_there_or_a_deleted_chapter() {
    let scratch = Scratch::new("collide");
    let path = scratch.0.join("made.epub");
    let book = new_book(&path, &scratch.data(), &meta("Made"), DEFAULT_LIMITS).unwrap();
    let add = |title: &str, book: &Book| {
        book.manage(
            ChapterOp::Add {
                after: None,
                title: title.to_owned(),
            },
            None,
        )
        .unwrap()
    };
    add("Two", &book);
    add("Three", &book);
    let deleted = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(2),
            },
            None,
        )
        .unwrap()
        .deleted
        .unwrap();
    let fresh = add("Replaces", &book);
    let reader = book.read();
    let names: Vec<String> = reader
        .rows()
        .iter()
        .map(|row| row.href.as_ref().unwrap().as_str().to_owned())
        .collect();
    assert_eq!(names.len(), 3);
    assert!(fresh.landing.is_some());
    book.manage(ChapterOp::UndoDelete { deleted }, None)
        .unwrap();
    let reader = book.read();
    assert_eq!(reader.rows().len(), 4);
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
}

#[test]
fn the_last_chapter_and_the_contents_page_are_refused_readably() {
    let scratch = Scratch::new("last");
    let path = scratch.0.join("made.epub");
    let book = new_book(&path, &scratch.data(), &meta("Made"), DEFAULT_LIMITS).unwrap();
    let before = fs::read(&path).unwrap();
    let error = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap_err();
    assert!(error.user_message().contains("last one"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);

    let source = scratch.0.join("nav-in-spine.epub");
    custom_book(
        &source,
        &opf_of(
            "",
            "<item id=\"a\" href=\"a.xhtml\" media-type=\"application/xhtml+xml\"/>",
            "<itemref idref=\"n\"/><itemref idref=\"a\"/>",
            "",
        ),
        &[
            (
                "nav.xhtml",
                &nav_of("<li><a href=\"a.xhtml\">A</a></li>", ""),
            ),
            ("a.xhtml", CHAPTER),
        ],
    );
    let other = Scratch::new("nav-row");
    let book = other.place(&source);
    let row = book
        .read()
        .rows()
        .iter()
        .position(|row| row.is_nav)
        .unwrap();
    let error = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(row as u32),
            },
            None,
        )
        .unwrap_err();
    assert!(
        error.user_message().contains("table of contents"),
        "{error}"
    );
}

#[test]
fn a_new_book_is_valid_opens_and_refuses_to_overwrite() {
    let scratch = Scratch::new("new");
    let path = scratch.0.join("made.epub");
    let book = new_book(
        &path,
        &scratch.data(),
        &meta("<script>x</script> & Book"),
        DEFAULT_LIMITS,
    )
    .unwrap();
    let reader = book.read();
    assert_eq!(reader.summary().title, "<script>x</script> & Book");
    assert_eq!(reader.rows().len(), 1);
    assert_eq!(reader.toc().unwrap().entries[0].label, "One");
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
    let files = package_files(&reader);
    let bytes = fs::read(&path).unwrap();
    for name in [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
        .into_iter()
        .flatten()
    {
        assert_parses(&entry_bytes(&bytes, name), name.as_str(), "new book");
    }
    assert!(!String::from_utf8_lossy(&entry_bytes(&bytes, &files.opf)).contains("<script>"));
    let error = new_book(&path, &scratch.data(), &meta("Again"), DEFAULT_LIMITS)
        .err()
        .unwrap();
    assert!(error.user_message().contains("already exists"));
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "an existing file was overwritten"
    );
    let bad = NewBookMeta {
        language: "not a language!".to_owned(),
        ..meta("x")
    };
    assert!(
        new_book(
            &scratch.0.join("b.epub"),
            &scratch.data(),
            &bad,
            DEFAULT_LIMITS
        )
        .is_err()
    );
    let empty = meta("   ");
    assert!(
        new_book(
            &scratch.0.join("c.epub"),
            &scratch.data(),
            &empty,
            DEFAULT_LIMITS
        )
        .is_err()
    );
    assert!(!scratch.0.join("b.epub").exists() && !scratch.0.join("c.epub").exists());
}

#[test]
fn a_later_launch_finds_the_backup_and_the_same_identity_after_an_op() {
    let scratch = Scratch::new("identity");
    let book = scratch.place(&fixture("toc-nav-only"));
    let identity = book.identity();
    let fingerprint = book.read().summary().fingerprint;
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "New".to_owned(),
        },
        None,
    )
    .unwrap();
    let created = match book.backup_status().unwrap() {
        BackupStatus::Valid { created_at, .. } => created_at,
        _ => panic!("no backup"),
    };
    let moved = book.read().summary().fingerprint;
    assert_ne!(moved, fingerprint, "an OPF edit changes the fingerprint");
    drop(book);
    let again = scratch.open();
    assert_eq!(again.identity(), identity);
    assert_eq!(crate::resolve_identity(&scratch.data(), &moved), identity);
    match again.backup_status().unwrap() {
        BackupStatus::Valid { created_at, .. } => assert_eq!(created_at, created),
        _ => panic!("a later launch did not find the backup"),
    }
    let entries: Vec<_> = fs::read_dir(scratch.data().join("books"))
        .unwrap()
        .collect();
    assert_eq!(entries.len(), 1, "one book keeps one data folder");
}

fn literal(text: &str) -> Query {
    Query::Literal {
        text: text.to_owned(),
        case: crate::Case::Insensitive,
        whole_word: false,
    }
}

fn search_options() -> SearchOptions {
    SearchOptions::default()
}

fn searched(book: &Book, text: &str) -> Vec<u32> {
    let mut hits = Vec::new();
    let end = book.search(
        &literal(text),
        SearchScope::Whole,
        &search_options(),
        &CancelToken::default(),
        &mut |event| {
            if let SearchEvent::Hits { chapter, .. } = event {
                hits.push(chapter.0);
            }
        },
    );
    assert!(matches!(end, SearchEnd::Done));
    hits
}

#[test]
fn search_sees_a_new_chapter_and_follows_the_new_order() {
    let scratch = Scratch::new("search");
    let book = scratch.place(&fixture("toc-nav-only"));
    let added = book
        .manage(
            ChapterOp::Add {
                after: Some(ChapterIx(0)),
                title: "Zanzibar marker".to_owned(),
            },
            None,
        )
        .unwrap();
    assert_eq!(added.landing, Some(ChapterIx(1)));
    assert_eq!(searched(&book, "Zanzibar marker").first(), Some(&1));
    book.manage(
        ChapterOp::Move {
            row: RowRef::Chapter(ChapterIx(1)),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(4)),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    assert_eq!(searched(&book, "Zanzibar marker").first(), Some(&4));
    assert!(
        book.read()
            .render(ChapterIx(4))
            .unwrap()
            .xhtml
            .contains("Zanzibar marker")
    );
}

#[test]
fn a_replace_preview_goes_stale_after_an_op() {
    let scratch = Scratch::new("stale");
    let book = scratch.place(&fixture("toc-nav-only"));
    let request = crate::ReplaceRequest::new(literal("the"), "THE".to_owned());
    let preview = book
        .preview_replace(
            &request,
            SearchScope::Whole,
            10,
            &search_options(),
            &CancelToken::default(),
            &mut |_| {},
        )
        .unwrap();
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "New".to_owned(),
        },
        None,
    )
    .unwrap();
    let error = book
        .apply_replace(
            &preview,
            &[],
            &search_options(),
            &CancelToken::default(),
            &mut |_, _| {},
        )
        .err()
        .unwrap();
    assert!(matches!(error, Error::PlanStale), "{error}");
}

#[test]
fn hostile_titles_stay_text_in_every_package_file() {
    const HOSTILE: &[&str] = &[
        "</a></li><li><a href=\"evil.xhtml\">x</a></li>",
        "]]><script>alert(1)</script>",
        "&amp; &#x3C; <!-- comment -->",
        "<?pi x?>\u{0}\u{1}\u{FFFE}",
        "\" onload=\"x",
        "</text></navLabel><content src=\"evil.xhtml\"/>",
    ];
    for &name in &["toc-nested-nav", "toc-nested-ncx", "toc-none"] {
        let scratch = Scratch::new("hostile");
        let book = scratch.place(&fixture(name));
        let manifest_before = book.read().manifest_count();
        for title in HOSTILE {
            let added = book
                .manage(
                    ChapterOp::Add {
                        after: Some(ChapterIx(0)),
                        title: (*title).to_owned(),
                    },
                    None,
                )
                .unwrap();
            book.manage(
                ChapterOp::Rename {
                    row: RowRef::Chapter(added.landing.unwrap()),
                    title: format!("{title}!"),
                },
                None,
            )
            .unwrap();
            let reader = book.read();
            let bytes = fs::read(scratch.book()).unwrap();
            let files = package_files(&reader);
            for doc in [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
                .into_iter()
                .flatten()
            {
                let text = entry_bytes(&bytes, doc);
                assert_parses(&text, doc.as_str(), name);
                let text = String::from_utf8_lossy(&text);
                assert!(
                    !text.contains("evil.xhtml\">"),
                    "{name}: a link was injected"
                );
                assert!(!text.contains("<script"), "{name}: a script was injected");
                assert!(
                    !text.contains("src=\"evil.xhtml\""),
                    "{name}: a target was injected"
                );
            }
            let chapter = entry_bytes(
                &bytes,
                reader.rows()[added.landing.unwrap().0 as usize]
                    .href
                    .as_ref()
                    .unwrap(),
            );
            assert_parses(&chapter, "chapter", name);
            assert!(!String::from_utf8_lossy(&chapter).contains("<script"));
            if reader.toc().unwrap().source != crate::TocSource::HeadingsPending {
                let toc = reader.toc().unwrap();
                let at = added.landing.unwrap();
                let label = toc
                    .entries
                    .iter()
                    .find(|entry| entry.target == Some(at))
                    .unwrap();
                assert_eq!(
                    label.label,
                    clean_title(&format!("{title}!")),
                    "{name}: label round trip"
                );
            }
        }
        assert_eq!(
            book.read().manifest_count(),
            manifest_before + HOSTILE.len()
        );
        let shape = structure(&scratch.book());
        assert!(
            strings(&shape["problems"])
                .iter()
                .all(|problem| problem.contains("EPUB3") || problem.contains("no target")),
            "{name}: {shape}"
        );
    }
}

#[test]
fn a_hostile_book_keeps_its_nav_entries_as_they_were_through_ops() {
    let scratch = Scratch::new("hostile-book");
    let book = scratch.place(&fixture("toc-hostile"));
    let before = {
        let reader = book.read();
        let files = package_files(&reader);
        String::from_utf8(entry_bytes(
            &fs::read(scratch.book()).unwrap(),
            files.nav.as_ref().unwrap(),
        ))
        .unwrap()
    };
    let added = book
        .manage(
            ChapterOp::Add {
                after: None,
                title: "Plain".to_owned(),
            },
            None,
        )
        .unwrap();
    let reader = book.read();
    let files = package_files(&reader);
    let after = String::from_utf8(entry_bytes(
        &fs::read(scratch.book()).unwrap(),
        files.nav.as_ref().unwrap(),
    ))
    .unwrap();
    assert!(after.len() > before.len());
    let label = after.find(">Plain<").expect("the new row is listed");
    let start = after[..label].rfind("<li").unwrap();
    let end = label + after[label..].find("</li>").unwrap() + "</li>".len();
    let removed = format!("{}{}", &after[..start], &after[end..]);
    let squeeze = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(squeeze(&removed), squeeze(&before), "other entries changed");
    assert!(added.landing.is_some());
}

#[test]
fn move_keeps_linear_and_properties_and_unknown_metadata_bytes() {
    let scratch = Scratch::new("attrs");
    let path = scratch.0.join("attrs.epub");
    {
        use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
        let opf = "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">T</dc:title><x:odd xmlns:x=\"urn:x\" y=\"z\">keep</x:odd><meta name=\"cover\" content=\"im\"/></metadata>\n <manifest>\n  <item id=\"im\" href=\"i.png\" media-type=\"image/png\" properties=\"cover-image\"/>\n  <item id=\"n\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n  <item id=\"a\" href=\"a.xhtml\" media-type=\"application/xhtml+xml\" properties=\"svg\"/>\n  <item id=\"b\" href=\"b.xhtml\" media-type=\"application/xhtml+xml\"/>\n </manifest>\n <spine page-progression-direction=\"ltr\">\n  <itemref idref=\"a\" linear=\"no\" properties=\"page-spread-left\"/>\n  <itemref idref=\"b\"/>\n </spine>\n</package>";
        let nav = "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body><nav epub:type=\"toc\"><ol><li><a href=\"a.xhtml\">A</a></li><li><a href=\"b.xhtml\">B</a></li></ol></nav></body></html>";
        let chapter = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>a</p></body></html>";
        let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.add(b"mimetype", b"application/epub+zip", STORED)
            .unwrap();
        zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)
            .unwrap();
        zip.add(b"content.opf", opf.as_bytes(), DEFLATE).unwrap();
        zip.add(b"nav.xhtml", nav.as_bytes(), DEFLATE).unwrap();
        zip.add(b"a.xhtml", chapter.as_bytes(), DEFLATE).unwrap();
        zip.add(b"b.xhtml", chapter.as_bytes(), DEFLATE).unwrap();
        zip.add(b"i.png", b"png", STORED).unwrap();
        zip.finish().unwrap();
    }
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let opf_name = book.read().opf_entry().clone();
    let before = String::from_utf8(entry_bytes(&fs::read(&path).unwrap(), &opf_name)).unwrap();
    book.manage(
        ChapterOp::Move {
            row: RowRef::Chapter(ChapterIx(0)),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(1)),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    let after = String::from_utf8(entry_bytes(&fs::read(&path).unwrap(), &opf_name)).unwrap();
    let expected = before.replace(
        "\n  <itemref idref=\"a\" linear=\"no\" properties=\"page-spread-left\"/>\n  <itemref idref=\"b\"/>",
        "\n  <itemref idref=\"b\"/>\n  <itemref idref=\"a\" linear=\"no\" properties=\"page-spread-left\"/>",
    );
    assert_eq!(after, expected, "only the two spine rows may swap");
    let added = book
        .manage(
            ChapterOp::Add {
                after: None,
                title: "C".to_owned(),
            },
            None,
        )
        .unwrap();
    let with_add = String::from_utf8(entry_bytes(&fs::read(&path).unwrap(), &opf_name)).unwrap();
    assert!(with_add.contains("<x:odd xmlns:x=\"urn:x\" y=\"z\">keep</x:odd>"));
    assert!(with_add.contains("properties=\"cover-image\""));
    assert!(with_add.contains("properties=\"nav\""));
    assert_eq!(added.landing, Some(ChapterIx(2)));
}

fn kill_op(name: &str) -> ChapterOp {
    match name {
        "delete" => ChapterOp::Delete {
            chapter: ChapterIx(2),
        },
        "add" => ChapterOp::Add {
            after: Some(ChapterIx(1)),
            title: "Added".to_owned(),
        },
        "rename" => ChapterOp::Rename {
            row: RowRef::Chapter(ChapterIx(1)),
            title: "Renamed".to_owned(),
        },
        "undo" => ChapterOp::UndoDelete {
            deleted: DeletedId(1),
        },
        _ => ChapterOp::Move {
            row: RowRef::Chapter(ChapterIx(0)),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(3)),
                after: true,
            },
        },
    }
}

#[test]
#[ignore = "kill child, run by the kill test"]
fn manage_kill_child() {
    let root = PathBuf::from(std::env::var_os("SS_KILL_ROOT").unwrap());
    let wanted = std::env::var("SS_KILL_STEP").unwrap();
    let op = kill_op(&std::env::var("SS_KILL_OP").unwrap());
    let book =
        Book::open_by_fingerprint(&root.join("book.epub"), &root.join("data"), DEFAULT_LIMITS)
            .unwrap();
    let sentinel = root.join("sentinel");
    crate::commit::seam::install(move |step| {
        if format!("{step:?}") == wanted {
            fs::write(&sentinel, b"at").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Ok(())
    });
    book.manage(op, None).unwrap();
    fs::write(root.join("finished"), b"done").unwrap();
}

#[test]
fn a_real_kill_at_every_step_of_an_op_leaves_old_or_new_and_recovery_converges() {
    use std::sync::{Arc, Mutex};
    let source = fixture("toc-nav-only");
    let original = fs::read(&source).unwrap();
    for op_name in ["delete", "add", "move", "rename", "undo"] {
        let prepare = |scratch: &Scratch| -> Vec<u8> {
            fs::write(scratch.book(), &original).unwrap();
            if op_name == "undo" {
                scratch.open().manage(kill_op("delete"), None).unwrap();
            }
            fs::read(scratch.book()).unwrap()
        };
        let reference = Scratch::new("kill-ref");
        prepare(&reference);
        let book = reference.open();
        let identity = book.identity();
        let steps = Arc::new(Mutex::new(Vec::new()));
        let seen = steps.clone();
        crate::commit::seam::install(move |step| {
            seen.lock().unwrap().push(format!("{step:?}"));
            Ok(())
        });
        book.manage(kill_op(op_name), None).unwrap();
        crate::commit::seam::install(|_| Ok(()));
        let new = fs::read(reference.book()).unwrap();
        let steps = steps.lock().unwrap().clone();
        let alias_at = steps.iter().position(|step| step == "AliasWritten");
        assert_eq!(
            alias_at.is_some(),
            matches!(op_name, "delete" | "add" | "move"),
            "an alias is written exactly when the package file becomes a new one: {steps:?}"
        );
        let rename_at = steps.iter().position(|step| step == "BookRenamed").unwrap();
        assert!(
            alias_at.is_none_or(|alias_at| alias_at < rename_at),
            "the alias must be durable before the rename"
        );
        if op_name == "delete" {
            let deleted_at = steps
                .iter()
                .position(|step| step == "DeletedSynced")
                .unwrap();
            assert!(
                deleted_at < rename_at,
                "the record must be durable before the rename"
            );
        }
        let mut distinct: Vec<String> = Vec::new();
        for step in &steps {
            if !distinct.contains(step) {
                distinct.push(step.clone());
            }
        }
        let (mut old_seen, mut new_seen) = (0, 0);
        for name in &distinct {
            let scratch = Scratch::new("kill");
            let old = prepare(&scratch);
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "manage::tests::manage_kill_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("SS_KILL_ROOT", &scratch.0)
                .env("SS_KILL_STEP", name)
                .env("SS_KILL_OP", op_name)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let sentinel = scratch.0.join("sentinel");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while !sentinel.exists() {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "{op_name} {name}: child ended early"
                );
                assert!(
                    std::time::Instant::now() < deadline,
                    "{op_name} {name}: never reached"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            let after = fs::read(scratch.book()).unwrap();
            let is_new = after == new;
            assert!(
                is_new || after == old,
                "{op_name} {name}: neither old nor new"
            );
            if is_new {
                new_seen += 1;
            } else {
                old_seen += 1;
            }
            let reopened = scratch.open();
            assert_eq!(
                reopened.identity(),
                identity,
                "{op_name} {name}: identity after a kill"
            );
            reopened.recover().unwrap();
            let rows = reopened.read().rows().len();
            let expected_listed = match op_name {
                "delete" => Some(usize::from(is_new)),
                "undo" => Some(usize::from(!is_new)),
                _ => None,
            };
            if let Some(expected) = expected_listed {
                let listed = match reopened.backup_status().unwrap() {
                    BackupStatus::Valid { deleted, .. } => deleted.len(),
                    BackupStatus::None | BackupStatus::OtherFileOnly => 0,
                };
                assert_eq!(listed, expected, "{op_name} {name}: restore list");
            }
            let again = reopened.manage(kill_op("add"), None).unwrap();
            assert!(again.landing.is_some(), "{op_name} {name}: the next op");
            assert_eq!(reopened.read().rows().len(), rows + 1);
            reopened.revert(RevertTarget::WholeBook).unwrap();
            assert_eq!(
                fs::read(scratch.book()).unwrap(),
                original,
                "{op_name} {name}: revert"
            );
            let stray = fs::read_dir(&scratch.0)
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
            assert_eq!(stray, 0, "{op_name} {name}: a temp stayed");
        }
        assert!(
            old_seen > 0 && new_seen > 0,
            "{op_name}: {old_seen} old, {new_seen} new"
        );
        println!(
            "{op_name}: {} steps, {old_seen} old, {new_seen} new",
            distinct.len()
        );
    }
}

fn spread(mut values: Vec<f64>) -> (f64, f64, f64) {
    values.sort_by(f64::total_cmp);
    (
        values[0],
        values[values.len() / 2],
        values[values.len() - 1],
    )
}

#[test]
#[ignore = "timing: cargo test --release -p jadeslip-core manage::tests::timed -- --ignored --nocapture"]
fn timed_ops_on_the_largest_books() {
    let mut books = real_books();
    books.retain(|path| {
        fs::metadata(path).unwrap().len() > 15 << 20 && label_of(path).starts_with('f')
    });
    books.push(fixture("large-h1-p"));
    books.push(fixture("large-h1-div-br"));
    for source in books {
        let label = label_of(&source);
        let scratch = Scratch::new("timed");
        let book = scratch.place(&source);
        let rows = book.read().rows().len() as u32;
        let middle = ChapterIx(rows / 2);
        let started = std::time::Instant::now();
        let first = book
            .manage(
                ChapterOp::Add {
                    after: Some(middle),
                    title: "First".to_owned(),
                },
                None,
            )
            .unwrap();
        println!(
            "{label}: {rows} rows, first op {:.0} ms, of which backup {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3,
            first.report.phases.backup.as_secs_f64() * 1e3
        );
        let mut times: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
        let mut last_phases: BTreeMap<&'static str, crate::CommitPhases> = BTreeMap::new();
        let mut plans: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
        for round in 0..7u32 {
            let ops: Vec<ChapterOp> = vec![
                ChapterOp::Add {
                    after: Some(middle),
                    title: format!("Added {round}"),
                },
                ChapterOp::Rename {
                    row: RowRef::Chapter(ChapterIx(rows / 3)),
                    title: format!("Renamed {round}"),
                },
                ChapterOp::Move {
                    row: RowRef::Chapter(ChapterIx(rows / 4)),
                    to: Spot {
                        row: RowRef::Chapter(ChapterIx(rows * 3 / 4)),
                        after: true,
                    },
                },
                ChapterOp::Delete {
                    chapter: ChapterIx(rows / 5),
                },
            ];
            let mut deleted = None;
            for op in ops {
                let kind = kind_of(&op);
                let at = std::time::Instant::now();
                let managed = book.manage(op, None).unwrap();
                let total = at.elapsed().as_secs_f64() * 1e3;
                times.entry(kind).or_default().push(total);
                plans
                    .entry(kind)
                    .or_default()
                    .push(total - managed.report.phases.total.as_secs_f64() * 1e3);
                last_phases.insert(kind, managed.report.phases);
                deleted = managed.deleted.or(deleted);
            }
            let at = std::time::Instant::now();
            let managed = book
                .manage(
                    ChapterOp::UndoDelete {
                        deleted: deleted.unwrap(),
                    },
                    None,
                )
                .unwrap();
            let total = at.elapsed().as_secs_f64() * 1e3;
            times.entry("undo").or_default().push(total);
            plans
                .entry("undo")
                .or_default()
                .push(total - managed.report.phases.total.as_secs_f64() * 1e3);
            last_phases.insert("undo", managed.report.phases);
        }
        assert_eq!(
            book.read().rows().len() as u32,
            rows + 1 + 7,
            "{label}: the work happened"
        );
        for (kind, values) in times {
            let (low, median, high) = spread(values);
            let (_, plan, _) = spread(plans.remove(kind).unwrap());
            let phases = last_phases[kind];
            println!(
                "{label} {kind}: median {median:.0} ms, range {low:.0} to {high:.0}, 7 runs. plan and checks {plan:.0} ms. last run: write {:.0} read {:.0} hash {:.0} sync {:.0} verify {:.0} ledger {:.0} rename {:.2} sync_dir {:.0}",
                phases.write.as_secs_f64() * 1e3,
                phases.read.as_secs_f64() * 1e3,
                phases.hash.as_secs_f64() * 1e3,
                phases.sync_temp.as_secs_f64() * 1e3,
                phases.verify.as_secs_f64() * 1e3,
                (phases.ledger_pending + phases.ledger_head).as_secs_f64() * 1e3,
                phases.rename.as_secs_f64() * 1e3,
                phases.sync_dir.as_secs_f64() * 1e3,
            );
        }
        let reader = book.read();
        let shape = structure(&scratch.book());
        assert!(strings(&shape["problems"]).is_empty(), "{label}: {shape}");
        drop(reader);
    }
}

fn custom_book(path: &Path, opf: &str, files: &[(&str, &str)]) {
    use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"content.opf", opf.as_bytes(), DEFLATE).unwrap();
    for (name, text) in files {
        zip.add(name.as_bytes(), text.as_bytes(), DEFLATE).unwrap();
    }
    zip.finish().unwrap();
}

const CHAPTER: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p>a</p></body></html>";

fn opf_of(extra_meta: &str, items: &str, spine: &str, guide: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">T</dc:title>{extra_meta}</metadata><manifest><item id=\"n\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>{items}</manifest><spine>{spine}</spine>{guide}</package>"
    )
}

const THREE_ITEMS: &str = "<item id=\"a\" href=\"a.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"b\" href=\"b.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"c\" href=\"c.xhtml\" media-type=\"application/xhtml+xml\"/>";
const THREE_ROWS: &str = "<itemref idref=\"a\"/><itemref idref=\"b\"/><itemref idref=\"c\"/>";

fn nav_of(toc: &str, extra: &str) -> String {
    format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body><nav epub:type=\"toc\"><ol>{toc}</ol></nav>{extra}</body></html>"
    )
}

const THREE_TOC: &str = "<li><a href=\"a.xhtml\">A</a></li><li><a href=\"b.xhtml\">B</a></li><li><a href=\"c.xhtml\">C</a></li>";

#[test]
fn a_failing_plan_writes_nothing_and_says_why() {
    let scratch = Scratch::new("refuse");
    let path = scratch.0.join("refuse.epub");
    let opf = opf_of(
        "<meta refines=\"#b\" property=\"x:y\">z</meta>",
        THREE_ITEMS,
        THREE_ROWS,
        "",
    );
    custom_book(
        &path,
        &opf,
        &[
            ("nav.xhtml", &nav_of(THREE_TOC, "")),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let before = fs::read(&path).unwrap();
    let error = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(1),
            },
            None,
        )
        .unwrap_err();
    assert!(matches!(error, Error::WouldBreak(_)), "{error}");
    assert!(error.user_message().contains("depends on"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        !matches!(book.backup_status().unwrap(), BackupStatus::Valid { .. }),
        "a refused op made a backup"
    );
    let only = Scratch::new("refuse-toc");
    let path = only.0.join("one-entry.epub");
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            (
                "nav.xhtml",
                &nav_of("<li><a href=\"a.xhtml\">A</a></li>", ""),
            ),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &only.data(), DEFAULT_LIMITS).unwrap();
    let before = fs::read(&path).unwrap();
    let error = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap_err();
    assert!(error.user_message().contains("no entries left"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn delete_cuts_guide_landmarks_and_empty_lists_then_undo_restores_them_exactly() {
    let scratch = Scratch::new("guide");
    let path = scratch.0.join("guide.epub");
    let guide = "<guide><reference type=\"text\" href=\"b.xhtml#x\" title=\"T\"/></guide>";
    let landmarks = "<nav epub:type=\"landmarks\"><ol><li><a epub:type=\"bodymatter\" href=\"b.xhtml\">Start</a></li></ol></nav><nav epub:type=\"page-list\"><ol><li><a href=\"a.xhtml#p1\">1</a></li><li><a href=\"b.xhtml#p2\">2</a></li></ol></nav>";
    let toc = "<li><span>Part</span><ol><li><a href=\"b.xhtml\">B</a></li></ol></li><li><a href=\"a.xhtml\">A</a><ol><li><a href=\"b.xhtml#s\">B2</a></li></ol></li><li><a href=\"c.xhtml\">C</a></li>";
    let ncx = "<?xml version=\"1.0\"?><ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><navMap><navPoint id=\"p1\" playOrder=\"1\"><navLabel><text>A</text></navLabel><content src=\"a.xhtml\"/><navPoint id=\"p2\" playOrder=\"2\"><navLabel><text>B</text></navLabel><content src=\"b.xhtml\"/></navPoint></navPoint><navPoint id=\"p3\" playOrder=\"3\"><navLabel><text>C</text></navLabel><content src=\"c.xhtml\"/></navPoint></navMap><pageList><pageTarget id=\"t1\" type=\"normal\" value=\"1\"><navLabel><text>1</text></navLabel><content src=\"b.xhtml#p\"/></pageTarget></pageList></ncx>";
    let items = format!(
        "{THREE_ITEMS}<item id=\"x\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>"
    );
    let rows = "<itemref idref=\"b\"/><itemref idref=\"a\"/><itemref idref=\"c\"/>";
    let opf = opf_of("", &items, rows, guide).replace("<spine>", "<spine toc=\"x\">");
    custom_book(
        &path,
        &opf,
        &[
            ("nav.xhtml", &nav_of(toc, landmarks)),
            ("toc.ncx", ncx),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let names: Vec<_> = ["content.opf", "nav.xhtml", "toc.ncx"]
        .iter()
        .map(|name| EntryName::parse(name).unwrap())
        .collect();
    let original = fs::read(&path).unwrap();
    let managed = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap();
    let now = fs::read(&path).unwrap();
    let text = |at: usize| String::from_utf8(entry_bytes(&now, &names[at])).unwrap();
    assert!(!text(0).contains("<guide"), "an emptied guide stays");
    assert!(
        !text(1).contains("b.xhtml"),
        "the nav still links to the chapter"
    );
    assert!(
        !text(1).contains("landmarks"),
        "an emptied landmarks nav stays"
    );
    assert!(
        text(1).contains("a.xhtml#p1"),
        "the other page link was cut"
    );
    assert!(
        !text(1).contains("Part"),
        "a span row with no children stays"
    );
    assert!(
        !text(2).contains("b.xhtml"),
        "the older contents still link to the chapter"
    );
    assert!(!text(2).contains("pageList"), "an emptied page list stays");
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
    book.manage(
        ChapterOp::UndoDelete {
            deleted: managed.deleted.unwrap(),
        },
        None,
    )
    .unwrap();
    let back = fs::read(&path).unwrap();
    for name in &names {
        assert_eq!(
            entry_bytes(&back, name),
            entry_bytes(&original, name),
            "{}",
            name.as_str()
        );
    }
}

#[test]
fn deleting_a_parent_keeps_its_children_at_its_level() {
    let scratch = Scratch::new("hoist");
    let path = scratch.0.join("hoist.epub");
    let toc = "<li><a href=\"a.xhtml\">A</a><ol><li><a href=\"b.xhtml\">B</a></li><li><a href=\"c.xhtml\">C</a></li></ol></li>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            ("nav.xhtml", &nav_of(toc, "")),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    book.manage(
        ChapterOp::Delete {
            chapter: ChapterIx(0),
        },
        None,
    )
    .unwrap();
    let reader = book.read();
    let shape: Vec<(String, u16)> = reader
        .toc()
        .unwrap()
        .entries
        .iter()
        .map(|entry| (entry.label.clone(), entry.depth))
        .collect();
    assert_eq!(shape, vec![("B".to_owned(), 0), ("C".to_owned(), 0)]);
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
}

#[test]
fn a_new_chapter_borrows_the_stylesheets_of_the_chapter_before_it() {
    let scratch = Scratch::new("css");
    let path = scratch.0.join("css.epub");
    let styled = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title><link rel=\"stylesheet\" href=\"s.css\"/><link rel=\"stylesheet\" href=\"http://evil.example/x.css\"/></head><body><p>a</p></body></html>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            ("nav.xhtml", &nav_of(THREE_TOC, "")),
            ("a.xhtml", styled),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let added = book
        .manage(
            ChapterOp::Add {
                after: Some(ChapterIx(0)),
                title: "New".to_owned(),
            },
            None,
        )
        .unwrap();
    let reader = book.read();
    let name = reader.rows()[added.landing.unwrap().0 as usize]
        .href
        .clone()
        .unwrap();
    let text = String::from_utf8(entry_bytes(&fs::read(&path).unwrap(), &name)).unwrap();
    assert!(
        text.contains("href=\"s.css\""),
        "the book's stylesheet is missing"
    );
    assert!(
        !text.contains("evil.example"),
        "a remote stylesheet was copied"
    );
    assert!(text.starts_with("<?xml"), "an XML declaration is missing");
}

#[test]
fn rename_changes_the_heading_only_when_it_mirrors_the_label() {
    let scratch = Scratch::new("rename");
    let path = scratch.0.join("rename.epub");
    let mirrored = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>A</title></head><body><h1>A</h1><p>a</p></body></html>";
    let different = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Other</title></head><body><h2>Other</h2><p>a</p></body></html>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            ("nav.xhtml", &nav_of(THREE_TOC, "")),
            ("a.xhtml", mirrored),
            ("b.xhtml", different),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    book.manage(
        ChapterOp::Rename {
            row: RowRef::Chapter(ChapterIx(0)),
            title: "New A".to_owned(),
        },
        None,
    )
    .unwrap();
    book.manage(
        ChapterOp::Rename {
            row: RowRef::Chapter(ChapterIx(1)),
            title: "New B".to_owned(),
        },
        None,
    )
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    let a = String::from_utf8(entry_bytes(&bytes, &EntryName::parse("a.xhtml").unwrap())).unwrap();
    let b = String::from_utf8(entry_bytes(&bytes, &EntryName::parse("b.xhtml").unwrap())).unwrap();
    assert!(
        a.contains("<title>New A</title>") && a.contains("<h1>New A</h1>"),
        "a mirrored heading stayed"
    );
    assert_eq!(
        b, different,
        "a heading that differs from the label changed"
    );
    let labels: Vec<String> = book
        .read()
        .toc()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.label.clone())
        .collect();
    assert_eq!(labels, ["New A", "New B", "C"]);
    assert!(
        bytes.windows(7).any(|window| window == b"a.xhtml"),
        "the file name changed"
    );
}

fn calibre(tool: &str, args: &[&std::ffi::OsStr], home: &Path) -> bool {
    Command::new(tool)
        .args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("CALIBRE_CONFIG_DIRECTORY", home)
        .env("CALIBRE_TEMP_DIR", home)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
#[ignore = "Calibre: cargo test --release -p jadeslip-core manage::tests::calibre -- --ignored --nocapture"]
fn calibre_opens_every_result_and_converts_a_sample() {
    let mut sources: Vec<PathBuf> = real_books();
    sources.extend(
        [
            "toc-nav-only",
            "toc-ncx-only",
            "toc-none",
            "toc-nested-nav",
            "toc-nested-ncx",
            "large-h1-p",
        ]
        .iter()
        .map(|name| fixture(name)),
    );
    let scratch_new = Scratch::new("calibre-new");
    let made = scratch_new.0.join("made.epub");
    drop(new_book(&made, &scratch_new.data(), &meta("Made"), DEFAULT_LIMITS).unwrap());
    sources.push(made);
    for source in sources {
        let label = label_of(&source);
        let scratch = Scratch::new("calibre");
        let home = scratch.0.join("calibre-home");
        fs::create_dir_all(&home).unwrap();
        let book = scratch.place(&source);
        let mut rng = Rng(0xabcd_1234);
        let mut undo = Vec::new();
        let mut serial = 0;
        let mut applied = 0;
        let mut opened = 0;
        for step in 0..8 {
            let op = random_op(&book, &mut rng, &mut undo, &mut serial);
            match book.manage(op, None) {
                Ok(managed) => {
                    applied += 1;
                    if let Some(deleted) = managed.deleted {
                        undo.push(deleted);
                    }
                }
                Err(_) => continue,
            }
            assert!(
                calibre("ebook-meta", &[scratch.book().as_os_str()], &home),
                "{label} step {step}: ebook-meta refused the result"
            );
            opened += 1;
        }
        let out = scratch.0.join("out.txt");
        let started = std::time::Instant::now();
        let converted = calibre(
            "ebook-convert",
            &[scratch.book().as_os_str(), out.as_os_str()],
            &home,
        );
        assert!(converted, "{label}: ebook-convert failed");
        let size = fs::metadata(&out).unwrap().len();
        assert!(size > 0, "{label}: ebook-convert wrote nothing");
        println!(
            "{label}: {applied} ops, ebook-meta opened {opened}, ebook-convert {:.1} s, {size} bytes of text",
            started.elapsed().as_secs_f64()
        );
    }
}

#[test]
fn a_whole_book_revert_clears_the_restore_list() {
    let scratch = Scratch::new("clear");
    let book = scratch.place(&fixture("toc-nav-only"));
    let original = fs::read(scratch.book()).unwrap();
    book.manage(
        ChapterOp::Delete {
            chapter: ChapterIx(1),
        },
        None,
    )
    .unwrap();
    book.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(fs::read(scratch.book()).unwrap(), original);
    book.manage(
        ChapterOp::Add {
            after: None,
            title: "Next".to_owned(),
        },
        None,
    )
    .unwrap();
    book.manage(
        ChapterOp::Delete {
            chapter: ChapterIx(0),
        },
        None,
    )
    .unwrap();
    let listed = match book.backup_status().unwrap() {
        BackupStatus::Valid { deleted, .. } => deleted.len(),
        _ => panic!("no backup"),
    };
    assert_eq!(listed, 1, "an old record outlived a revert");
    let slot_files = fs::read_dir(scratch.data().join("books"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("backups");
    let deleted_dir = fs::read_dir(slot_files)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("deleted");
    assert_eq!(
        fs::read_dir(deleted_dir).unwrap().count(),
        1,
        "a stale record file stayed"
    );
}

#[test]
fn moving_a_group_takes_its_chapters_and_keeps_them_nested() {
    let scratch = Scratch::new("group");
    let path = scratch.0.join("group.epub");
    let toc = "<li><span>Part</span><ol><li><a href=\"a.xhtml\">A</a></li><li><a href=\"b.xhtml\">B</a></li></ol></li><li><a href=\"c.xhtml\">C</a></li>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            ("nav.xhtml", &nav_of(toc, "")),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    book.manage(
        ChapterOp::Move {
            row: RowRef::Entry(0),
            to: Spot {
                row: RowRef::Entry(3),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    let reader = book.read();
    let names: Vec<_> = order(&reader).into_iter().flatten().collect();
    assert_eq!(names, ["c.xhtml", "a.xhtml", "b.xhtml"]);
    let shape: Vec<(String, u16)> = reader
        .toc()
        .unwrap()
        .entries
        .iter()
        .map(|entry| (entry.label.clone(), entry.depth))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("C".to_owned(), 0),
            ("Part".to_owned(), 0),
            ("A".to_owned(), 1),
            ("B".to_owned(), 1)
        ]
    );
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
}

#[test]
fn play_order_follows_the_new_order_after_add_and_move() {
    let scratch = Scratch::new("play");
    let book = scratch.place(&fixture("toc-ncx-only"));
    book.manage(
        ChapterOp::Add {
            after: Some(ChapterIx(1)),
            title: "New".to_owned(),
        },
        None,
    )
    .unwrap();
    book.manage(
        ChapterOp::Move {
            row: RowRef::Chapter(ChapterIx(0)),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(4)),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    let reader = book.read();
    let bytes = fs::read(scratch.book()).unwrap();
    let ncx = String::from_utf8(entry_bytes(&bytes, reader.ncx_name().unwrap())).unwrap();
    let orders: Vec<u32> = ncx
        .split("playOrder=\"")
        .skip(1)
        .map(|tail| tail.split('"').next().unwrap().parse().unwrap())
        .collect();
    assert_eq!(orders, (1..=7).collect::<Vec<u32>>());
}

#[test]
fn a_guide_keeps_its_other_references_after_a_delete() {
    let scratch = Scratch::new("guide-two");
    let path = scratch.0.join("guide.epub");
    let guide = "<guide><reference type=\"text\" href=\"a.xhtml\" title=\"A\"/><reference type=\"toc\" href=\"b.xhtml\" title=\"B\"/></guide>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, guide),
        &[
            ("nav.xhtml", &nav_of(THREE_TOC, "")),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    book.manage(
        ChapterOp::Delete {
            chapter: ChapterIx(1),
        },
        None,
    )
    .unwrap();
    let opf_name = book.read().opf_entry().clone();
    let opf = String::from_utf8(entry_bytes(&fs::read(&path).unwrap(), &opf_name)).unwrap();
    assert!(opf.contains("href=\"a.xhtml\" title=\"A\""));
    assert!(
        !opf.contains("title=\"B\""),
        "a reference to the deleted file stayed"
    );
}

#[test]
fn a_section_row_is_refused_a_move_readably() {
    let scratch = Scratch::new("section");
    let path = scratch.0.join("section.epub");
    let toc = "<li><a href=\"a.xhtml\">A</a><ol><li><a href=\"a.xhtml#s\">Section</a></li></ol></li><li><a href=\"b.xhtml\">B</a></li><li><a href=\"c.xhtml\">C</a></li>";
    custom_book(
        &path,
        &opf_of("", THREE_ITEMS, THREE_ROWS, ""),
        &[
            ("nav.xhtml", &nav_of(toc, "")),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let before = fs::read(&path).unwrap();
    let error = book
        .manage(
            ChapterOp::Move {
                row: RowRef::Entry(1),
                to: Spot {
                    row: RowRef::Entry(3),
                    after: true,
                },
            },
            None,
        )
        .unwrap_err();
    assert!(error.user_message().contains("section"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core manage::tests::real -- --ignored --nocapture"]
fn real_book_delete_then_undo_restores_every_package_file_byte_equal() {
    for path in real_books() {
        let label = label_of(&path);
        let scratch = Scratch::new("real-undo");
        let book = scratch.place(&path);
        let reader = book.read();
        let row = (reader.rows().len() / 2..reader.rows().len())
            .find(|&at| !reader.rows()[at].is_nav && reader.rows()[at].rev.is_some())
            .unwrap();
        let name = reader.rows()[row].href.clone().unwrap();
        let files = package_files(&reader);
        let before_order = order(&reader);
        drop(reader);
        let original = fs::read(scratch.book()).unwrap();
        let id = book
            .manage(
                ChapterOp::Delete {
                    chapter: ChapterIx(row as u32),
                },
                None,
            )
            .unwrap()
            .deleted
            .unwrap();
        let undone = book
            .manage(ChapterOp::UndoDelete { deleted: id }, None)
            .unwrap();
        assert_eq!(
            undone.landing,
            Some(ChapterIx(row as u32)),
            "{label}: place"
        );
        assert_eq!(order(&book.read()), before_order, "{label}: read order");
        let now = fs::read(scratch.book()).unwrap();
        assert!(
            entry_bytes(&now, &name) == entry_bytes(&original, &name),
            "{label}: chapter bytes"
        );
        for doc in [Some(&files.opf), files.nav.as_ref(), files.ncx.as_ref()]
            .into_iter()
            .flatten()
        {
            assert!(
                entry_bytes(&now, doc) == entry_bytes(&original, doc),
                "{label}: a package file changed"
            );
        }
        println!("{label}: delete then undo byte-equal");
    }
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core manage::tests::real -- --ignored --nocapture"]
fn real_book_groups_move_as_one_and_stay_valid() {
    for path in real_books() {
        let label = label_of(&path);
        let scratch = Scratch::new("real-group");
        let book = scratch.place(&path);
        let baseline = strings(&structure(&scratch.book())["problems"]);
        let groups: Vec<u32> = {
            let reader = book.read();
            let toc = reader.toc().unwrap();
            let rows_of = |at: usize| {
                let depth = toc.entries[at].depth;
                let mut rows = BTreeSet::new();
                rows.extend(toc.entries[at].target);
                for entry in toc.entries[at + 1..]
                    .iter()
                    .take_while(|entry| entry.depth > depth)
                {
                    rows.extend(entry.target);
                }
                rows.len()
            };
            (0..toc.entries.len())
                .filter(|&at| toc.entries[at].depth == 0 && rows_of(at) > 1)
                .map(|at| at as u32)
                .take(6)
                .collect()
        };
        let mut moved = 0;
        let mut refused = 0;
        for &group in &groups {
            let reader = book.read();
            let last = reader.toc().unwrap().entries.len() as u32 - 1;
            let block = expected_block(&reader, RowRef::Entry(group));
            let names = order(&reader);
            let moving: Vec<_> = block.iter().map(|&at| names[at as usize].clone()).collect();
            let prior = fs::read(scratch.book()).unwrap();
            drop(reader);
            let outcome = book.manage(
                ChapterOp::Move {
                    row: RowRef::Entry(group),
                    to: Spot {
                        row: RowRef::Entry(last),
                        after: true,
                    },
                },
                None,
            );
            if outcome.is_err() {
                assert_eq!(
                    fs::read(scratch.book()).unwrap(),
                    prior,
                    "{label}: a refusal wrote"
                );
                refused += 1;
                continue;
            }
            moved += 1;
            let now = order(&book.read());
            assert!(
                now.windows(moving.len())
                    .any(|window| window == moving.as_slice()),
                "{label}: a group did not stay together"
            );
            let fresh: Vec<_> = strings(&structure(&scratch.book())["problems"])
                .difference(&baseline)
                .cloned()
                .collect();
            assert!(fresh.is_empty(), "{label}: {fresh:?}");
            book.revert(RevertTarget::WholeBook).unwrap();
        }
        println!(
            "{label}: {} groups, {moved} moved, {refused} refused",
            groups.len()
        );
    }
}

#[test]
fn a_delete_that_failed_at_the_rename_leaves_no_second_restore_row() {
    let scratch = Scratch::new("failed-delete");
    let book = scratch.place(&fixture("toc-nav-only"));
    let before = fs::read(scratch.book()).unwrap();
    crate::commit::seam::install(|step| {
        if format!("{step:?}") == "StampRechecked" {
            return Err(std::io::Error::other("injected"));
        }
        Ok(())
    });
    let op = ChapterOp::Delete {
        chapter: ChapterIx(1),
    };
    assert!(book.manage(op.clone(), None).is_err());
    crate::commit::seam::install(|_| Ok(()));
    assert_eq!(fs::read(scratch.book()).unwrap(), before);
    book.manage(op, None).unwrap();
    match book.backup_status().unwrap() {
        BackupStatus::Valid { deleted, .. } => assert_eq!(deleted.len(), 1),
        _ => panic!("no backup"),
    }
}

const LINK_HEAD: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><head><title>T</title></head><body>";
const LINK_TAIL: &str = "</body></html>\n";

struct LinkBook {
    scratch: Scratch,
    book: Book,
    names: Vec<EntryName>,
}

fn link_book(label: &str, chapters: usize) -> LinkBook {
    let scratch = Scratch::new(label);
    let book = new_book(
        &scratch.book(),
        &scratch.data(),
        &meta("Links"),
        DEFAULT_LIMITS,
    )
    .unwrap();
    for at in 1..chapters {
        book.manage(
            ChapterOp::Add {
                after: Some(ChapterIx(at as u32 - 1)),
                title: format!("Chapter {at}"),
            },
            None,
        )
        .unwrap();
    }
    let names = book
        .read()
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .map(|row| row.href.clone().unwrap())
        .collect();
    LinkBook {
        scratch,
        book,
        names,
    }
}

impl LinkBook {
    fn base(&self, at: usize) -> String {
        self.names[at]
            .as_str()
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned()
    }

    fn set_body(&self, at: usize, body: &str) {
        self.set_raw(at, format!("{LINK_HEAD}{body}{LINK_TAIL}").as_bytes());
    }

    fn set_raw(&self, at: usize, bytes: &[u8]) {
        self.book
            .save_entries(vec![NewEntry::deflate(self.names[at].clone(), bytes)])
            .unwrap();
    }

    fn body(&self, at: usize) -> String {
        let bytes = entry_bytes(&fs::read(self.scratch.book()).unwrap(), &self.names[at]);
        let text = String::from_utf8(bytes).unwrap();
        let start = text.find("<body>").unwrap() + "<body>".len();
        text[start..text.rfind("</body>").unwrap()].to_owned()
    }

    fn row_of(&self, at: usize) -> ChapterIx {
        self.book
            .read()
            .chapter_of_entry(self.names[at].as_str())
            .unwrap()
    }

    fn delete(&self, at: usize) -> Managed {
        self.book
            .manage(
                ChapterOp::Delete {
                    chapter: self.row_of(at),
                },
                None,
            )
            .unwrap()
    }

    fn dangling(&self) -> u64 {
        structure(&self.scratch.book())["dangling_links"]
            .as_u64()
            .unwrap()
    }
}

#[test]
fn delete_unwraps_every_inbound_link_shape_and_leaves_the_rest_alone() {
    let made = link_book("unwrap-shapes", 4);
    let gone = made.base(2);
    let kept = made.base(1);
    let cases = [
        (
            format!("<p><a epub:type=\"noteref\" id=\"typed\" href=\"{gone}\">note</a></p>"),
            "<p><span id=\"typed\">note</span></p>".to_owned(),
        ),
        (
            format!("<p><a id='q\"uote' href=\"{gone}\">quoted</a></p>"),
            "<p><span id=\"q&quot;uote\">quoted</span></p>".to_owned(),
        ),
        (
            format!("<p>see <a href=\"{gone}\">there</a> now</p>"),
            "<p>see there now</p>".to_owned(),
        ),
        (
            format!("<p><a href=\"{gone}#note\" class=\"c\"><i>x</i> y</a></p>"),
            "<p><i>x</i> y</p>".to_owned(),
        ),
        (
            format!("<p><a href=\"./{gone}\">dot</a></p>"),
            "<p>dot</p>".to_owned(),
        ),
        (
            format!("<p><A HREF=\"{gone}\">upper</A></p>"),
            "<p>upper</p>".to_owned(),
        ),
        (
            format!("<p><a id=\"keep\" href=\"{gone}\">with id</a></p>"),
            "<p><span id=\"keep\">with id</span></p>".to_owned(),
        ),
        (
            format!("<p><a id=\"lone\" href=\"{gone}\"/>tail</p>"),
            "<p><span id=\"lone\"></span>tail</p>".to_owned(),
        ),
        (
            format!("<p><a href=\"{gone}\"/>bare</p>"),
            "<p>bare</p>".to_owned(),
        ),
        (
            format!("<p><a xlink:href=\"{gone}\">svg</a></p>"),
            "<p>svg</p>".to_owned(),
        ),
        (
            format!("<map name=\"m\"><area href=\"{gone}\" alt=\"a\"/></map><p>m</p>"),
            "<map name=\"m\"></map><p>m</p>".to_owned(),
        ),
        (
            format!(
                "<p><a href=\"{}\">encoded</a></p>",
                gone.replace('c', "%63")
            ),
            "<p>encoded</p>".to_owned(),
        ),
        (
            format!(
                "<p><a href=\"{}\">entity</a></p>",
                gone.replace('c', "&#99;")
            ),
            "<p>entity</p>".to_owned(),
        ),
        (
            format!("<p><a href=\"{gone}\">outer</a> and <a href=\"{kept}\">other</a></p>"),
            format!("<p>outer and <a href=\"{kept}\">other</a></p>"),
        ),
        (
            format!("<p><a href=\"http://example.com/{gone}\">web</a></p>"),
            format!("<p><a href=\"http://example.com/{gone}\">web</a></p>"),
        ),
        (
            format!("<p><a href=\"sub/{gone}\">other folder</a></p>"),
            format!("<p><a href=\"sub/{gone}\">other folder</a></p>"),
        ),
        (
            format!("<!-- <a href=\"{gone}\">c</a> --><p>note</p>"),
            format!("<!-- <a href=\"{gone}\">c</a> --><p>note</p>"),
        ),
        (
            "<p><a href=\"#here\">self</a><a id=\"here\">target</a></p>".to_owned(),
            "<p><a href=\"#here\">self</a><a id=\"here\">target</a></p>".to_owned(),
        ),
    ];
    for (body, expected) in &cases {
        made.set_body(0, body);
        let managed = made.delete(2);
        assert_eq!(&made.body(0), expected, "{body}");
        made.book
            .manage(
                ChapterOp::UndoDelete {
                    deleted: managed.deleted.unwrap(),
                },
                None,
            )
            .unwrap();
        assert_eq!(&made.body(0), body, "undo of {body}");
    }
}

#[test]
fn links_from_other_folders_and_chapters_outside_the_spine_are_found() {
    let made = link_book("unwrap-spine", 3);
    let gone = made.base(1);
    made.set_body(
        0,
        &format!("<p><a href=\"../OEBPS/{gone}#x\">up and back</a></p>"),
    );
    made.set_body(2, &format!("<p><a href=\"{gone}\">last</a></p>"));
    made.delete(1);
    assert_eq!(made.body(0), "<p>up and back</p>");
    assert_eq!(made.body(2), "<p>last</p>");
    assert_eq!(made.dangling(), 0);
}

#[test]
fn unwrap_then_undo_is_byte_equal_for_the_whole_book_and_changes_only_linking_chapters() {
    let made = link_book("unwrap-undo", 5);
    let gone = made.base(3);
    made.set_body(
        0,
        &format!("<p>a <a href=\"{gone}\">one</a> b <a href=\"{gone}#n\">two</a></p>"),
    );
    made.set_body(1, "<p>no links at all here</p>");
    let before = fs::read(made.scratch.book()).unwrap();
    let managed = made.delete(3);
    let during = fs::read(made.scratch.book()).unwrap();
    assert_eq!(made.dangling(), 0);
    assert_eq!(
        strings(&structure(&made.scratch.book())["problems"]),
        BTreeSet::new()
    );
    assert_eq!(made.body(0), "<p>a one b two</p>");
    for unchanged in [1, 2, 4] {
        assert_eq!(
            entry_bytes(&during, &made.names[unchanged]),
            entry_bytes(&before, &made.names[unchanged]),
            "a chapter with no link keeps its bytes"
        );
    }
    made.book
        .manage(
            ChapterOp::UndoDelete {
                deleted: managed.deleted.unwrap(),
            },
            None,
        )
        .unwrap();
    let after = fs::read(made.scratch.book()).unwrap();
    let names = |bytes: &[u8]| -> Vec<String> {
        let file = made.scratch.0.join("probe.epub");
        fs::write(&file, bytes).unwrap();
        let source = File::open(&file).unwrap();
        let zip = ZipIndex::parse(&source, &DEFAULT_LIMITS).unwrap();
        zip.entries()
            .iter()
            .map(|entry| entry.name.as_str().to_owned())
            .collect()
    };
    let mut listed = names(&before);
    let mut listed_after = names(&after);
    listed.sort();
    listed_after.sort();
    assert_eq!(listed, listed_after);
    for name in listed {
        let name = EntryName::parse(&name).unwrap();
        assert_eq!(
            entry_bytes(&before, &name),
            entry_bytes(&after, &name),
            "{} is byte-equal after the undo",
            name.as_str()
        );
    }
}

#[test]
fn a_gbk_chapter_keeps_its_encoding_through_unwrap_and_undo() {
    let made = link_book("unwrap-gbk", 3);
    let gone = made.base(1);
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"GBK\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body><p>你好<a href=\"{gone}\">世界</a>再见</p></body></html>\n"
    );
    let (encoded, _, _) = encoding_rs::GBK.encode(&text);
    let original = encoded.into_owned();
    made.set_raw(0, &original);
    let managed = made.delete(1);
    let now = entry_bytes(&fs::read(made.scratch.book()).unwrap(), &made.names[0]);
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(&now);
    assert!(!had_errors);
    assert!(decoded.contains("<p>你好世界再见</p>"));
    assert!(decoded.contains("encoding=\"GBK\""));
    made.book
        .manage(
            ChapterOp::UndoDelete {
                deleted: managed.deleted.unwrap(),
            },
            None,
        )
        .unwrap();
    assert_eq!(
        entry_bytes(&fs::read(made.scratch.book()).unwrap(), &made.names[0]),
        original
    );
}

#[test]
fn an_edit_after_the_delete_keeps_the_edit_and_warns_at_undo() {
    let made = link_book("unwrap-edited", 3);
    let gone = made.base(1);
    made.set_body(0, &format!("<p><a href=\"{gone}\">x</a></p>"));
    let managed = made.delete(1);
    made.set_body(0, "<p>x and new words</p>");
    let undone = made
        .book
        .manage(
            ChapterOp::UndoDelete {
                deleted: managed.deleted.unwrap(),
            },
            None,
        )
        .unwrap();
    assert_eq!(made.body(0), "<p>x and new words</p>");
    assert_eq!(undone.report.warnings.len(), 1);
    assert_eq!(made.dangling(), 0);
}

#[test]
fn undoing_out_of_order_never_brings_back_a_dangling_link() {
    let made = link_book("unwrap-order", 4);
    let (one, two) = (made.base(1), made.base(2));
    made.set_body(1, &format!("<p><a href=\"{two}\">to two</a></p>"));
    made.set_body(
        0,
        &format!("<p><a href=\"{one}\">to one</a> <a href=\"{two}\">to two</a></p>"),
    );
    let first = made.delete(1).deleted.unwrap();
    let second = made.delete(2).deleted.unwrap();
    assert_eq!(made.dangling(), 0);
    made.book
        .manage(ChapterOp::UndoDelete { deleted: first }, None)
        .unwrap();
    assert_eq!(
        made.dangling(),
        0,
        "chapter one came back without its link to two"
    );
    assert!(!made.body(1).contains("href"));
    made.book
        .manage(ChapterOp::UndoDelete { deleted: second }, None)
        .unwrap();
    assert_eq!(made.dangling(), 0);
    assert!(strings(&structure(&made.scratch.book())["problems"]).is_empty());
}

#[test]
fn undoing_in_reverse_order_returns_every_link() {
    let made = link_book("unwrap-lifo", 4);
    let (one, two) = (made.base(1), made.base(2));
    made.set_body(1, &format!("<p><a href=\"{two}\">to two</a></p>"));
    let body = format!("<p><a href=\"{one}\">to one</a> <a href=\"{two}\">to two</a></p>");
    made.set_body(0, &body);
    let before = fs::read(made.scratch.book()).unwrap();
    let first = made.delete(1).deleted.unwrap();
    let second = made.delete(2).deleted.unwrap();
    made.book
        .manage(ChapterOp::UndoDelete { deleted: second }, None)
        .unwrap();
    made.book
        .manage(ChapterOp::UndoDelete { deleted: first }, None)
        .unwrap();
    let after = fs::read(made.scratch.book()).unwrap();
    for name in &made.names {
        assert_eq!(entry_bytes(&after, name), entry_bytes(&before, name));
    }
}

#[test]
fn reverting_a_linking_chapter_cannot_bring_back_a_dangling_link() {
    let made = link_book("unwrap-revert", 3);
    let gone = made.base(1);
    let kept = made.base(2);
    made.set_body(
        0,
        &format!("<p><a href=\"{gone}\">x</a> <a href=\"{kept}\">y</a></p>"),
    );
    let fresh = Book::open_by_fingerprint(
        &made.scratch.book(),
        &made.scratch.0.join("fresh-data"),
        DEFAULT_LIMITS,
    )
    .unwrap();
    let row = fresh
        .read()
        .chapter_of_entry(made.names[1].as_str())
        .unwrap();
    fresh
        .manage(ChapterOp::Delete { chapter: row }, None)
        .unwrap();
    assert_eq!(made.body(0), format!("<p>x <a href=\"{kept}\">y</a></p>"));
    let plain = fresh
        .revert(RevertTarget::Chapter(made.names[0].clone()))
        .unwrap();
    assert_eq!(
        plain.bytes_written, 0,
        "unwrapped bytes already match, so nothing is written"
    );
    let text = format!("{LINK_HEAD}<p>edited away</p>{LINK_TAIL}");
    fresh
        .save_entries(vec![NewEntry::deflate(
            made.names[0].clone(),
            text.as_bytes(),
        )])
        .unwrap();
    fresh
        .revert(RevertTarget::Chapter(made.names[0].clone()))
        .unwrap();
    assert_eq!(made.dangling(), 0);
    assert_eq!(made.body(0), format!("<p>x <a href=\"{kept}\">y</a></p>"));
    fresh.revert(RevertTarget::WholeBook).unwrap();
    assert_eq!(
        made.body(0),
        format!("<p><a href=\"{gone}\">x</a> <a href=\"{kept}\">y</a></p>"),
        "the whole-book revert brings the link and its target back together"
    );
    assert_eq!(made.dangling(), 0);
}

#[test]
fn the_contents_page_cannot_be_reverted_alone() {
    let made = link_book("revert-nav", 3);
    let nav = made.book.read().nav_name().cloned().unwrap();
    let error = made.book.revert(RevertTarget::Chapter(nav)).unwrap_err();
    assert!(error.user_message().contains("whole book"));
    let opf = made.book.read().opf_entry().clone();
    assert!(made.book.revert(RevertTarget::Chapter(opf)).is_err());
}

#[test]
fn a_chapter_that_cannot_be_scanned_warns_and_the_delete_still_happens() {
    let made = link_book("unwrap-limit", 3);
    let gone = made.base(1);
    let deep = format!(
        "{}<a href=\"{gone}\">x</a>{}",
        "<i>".repeat(200),
        "</i>".repeat(200)
    );
    made.set_body(0, &deep);
    let managed = made.delete(1);
    assert!(managed.deleted.is_some());
    let shown = managed.report.warnings.join(" ");
    assert!(shown.contains("could not be checked"), "{shown}");
}

#[test]
fn a_book_with_nothing_linking_rewrites_no_chapter() {
    let made = link_book("unwrap-none", 4);
    let before = fs::read(made.scratch.book()).unwrap();
    made.delete(2);
    let after = fs::read(made.scratch.book()).unwrap();
    for at in [0, 1, 3] {
        assert_eq!(
            entry_bytes(&before, &made.names[at]),
            entry_bytes(&after, &made.names[at])
        );
    }
}

#[test]
#[ignore = "copies real books, run by hand"]
fn real_book_delete_with_links_counts_and_times() {
    for path in real_books() {
        let scratch = Scratch::new("real-links");
        let book = scratch.place(&path);
        let reader = book.read();
        let row = reader
            .rows()
            .iter()
            .rev()
            .find(|row| !row.is_nav && row.linear_ordinal.is_some())
            .unwrap()
            .ix;
        drop(reader);
        let started = std::time::Instant::now();
        let managed = book
            .manage(ChapterOp::Delete { chapter: row }, None)
            .unwrap();
        let took = started.elapsed();
        let found = structure(&scratch.book());
        println!(
            "delete {:?} rewritten {} dangling {} problems {}",
            took,
            managed.report.entries_rewritten,
            found["dangling_links"],
            found["problems"].as_array().unwrap().len()
        );
    }
}

fn ncx_book(path: &Path, depth: &str) {
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">T</dc:title></metadata><manifest><item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>{THREE_ITEMS}</manifest><spine toc=\"ncx\">{THREE_ROWS}</spine></package>"
    );
    let point = |id: &str, file: &str, label: &str, kids: &str| {
        format!(
            "<navPoint id=\"{id}\"><navLabel><text>{label}</text></navLabel><content src=\"{file}\"/>{kids}</navPoint>"
        )
    };
    let nested = point("pb", "b.xhtml", "B", "");
    let ncx = format!(
        "<?xml version=\"1.0\"?><ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:depth\" content=\"{depth}\"/></head><docTitle><text>T</text></docTitle><navMap>{}{}</navMap></ncx>",
        point("pa", "a.xhtml", "A", &nested),
        point("pc", "c.xhtml", "C", "")
    );
    custom_book(
        path,
        &opf,
        &[
            ("toc.ncx", &ncx),
            ("a.xhtml", CHAPTER),
            ("b.xhtml", CHAPTER),
            ("c.xhtml", CHAPTER),
        ],
    );
}

fn ncx_depth(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let ncx =
        String::from_utf8(entry_bytes(&bytes, &EntryName::parse("toc.ncx").unwrap())).unwrap();
    let tail = ncx.split("name=\"dtb:depth\" content=\"").nth(1).unwrap();
    tail.split('"').next().unwrap().to_owned()
}

#[test]
fn the_ncx_depth_follows_the_nesting_through_delete_undo_and_move() {
    let scratch = Scratch::new("ncx-depth");
    let path = scratch.0.join("depth.epub");
    ncx_book(&path, "2");
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let deleted = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap()
        .deleted
        .unwrap();
    assert_eq!(ncx_depth(&path), "1", "after deleting the parent");
    book.manage(ChapterOp::UndoDelete { deleted }, None)
        .unwrap();
    assert_eq!(ncx_depth(&path), "2", "after undo");
    let deleted = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap()
        .deleted
        .unwrap();
    book.manage(
        ChapterOp::Add {
            after: Some(ChapterIx(1)),
            title: "Later".to_owned(),
        },
        None,
    )
    .unwrap();
    assert_eq!(ncx_depth(&path), "1", "after an add at the top level");
    book.manage(ChapterOp::UndoDelete { deleted }, None)
        .unwrap();
    let deepest = book
        .read()
        .toc()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.depth + 1)
        .max()
        .unwrap();
    assert_eq!(
        ncx_depth(&path),
        deepest.to_string(),
        "after undo past a later add"
    );
    book.manage(
        ChapterOp::Move {
            row: RowRef::Chapter(ChapterIx(1)),
            to: Spot {
                row: RowRef::Chapter(ChapterIx(2)),
                after: true,
            },
        },
        None,
    )
    .unwrap();
    assert_eq!(ncx_depth(&path), "1", "after moving the child to the top");
    assert_eq!(strings(&structure(&path)["problems"]), BTreeSet::new());
}

#[test]
fn a_wrong_ncx_depth_is_left_as_the_book_had_it() {
    let scratch = Scratch::new("ncx-depth-wrong");
    let path = scratch.0.join("depth.epub");
    ncx_book(&path, "0");
    let book = Book::open_by_fingerprint(&path, &scratch.data(), DEFAULT_LIMITS).unwrap();
    let deleted = book
        .manage(
            ChapterOp::Delete {
                chapter: ChapterIx(0),
            },
            None,
        )
        .unwrap()
        .deleted
        .unwrap();
    assert_eq!(ncx_depth(&path), "0");
    book.manage(ChapterOp::UndoDelete { deleted }, None)
        .unwrap();
    assert_eq!(ncx_depth(&path), "0");
}
