mod common;

use jadeslip_core::proof::{
    block_units_of, block_xml_of, damage_of, decoded_of, tokens_of, visible_text_of,
};
use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, BookKey, ChapterIx, ChapterRev, DEFAULT_LIMITS,
    EntryName, Extracted, NewEntry, Reader, SaveOutcome,
};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn real_books() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "epub"));
    paths.sort();
    paths
}

fn real_book(id: &str) -> PathBuf {
    real_books()
        .into_iter()
        .find(|path| id_of(path) == id)
        .expect("the real book is present")
}

fn editable_generated_books() -> Vec<PathBuf> {
    common::books()
        .into_iter()
        .filter(|name| {
            !name.starts_with("bomb")
                && !matches!(
                    *name,
                    "malformed-container-ref" | "malformed-spine-ref" | "malformed-missing-chapter"
                )
        })
        .map(common::fixture)
        .collect()
}

fn id_of(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy();
    stem.rsplit('-').next().unwrap_or("").to_owned()
}

fn scratch(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/edit-real")
        .join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("data")).unwrap();
    dir
}

fn placed(source: &Path, dir: &Path) -> (Book, PathBuf) {
    let path = dir.join("book.epub");
    fs::write(&path, fs::read(source).unwrap()).unwrap();
    let book = Book::open(&path, &dir.join("data"), BookKey(7), DEFAULT_LIMITS).unwrap();
    (book, path)
}

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

fn raw_chapter(reader: &Reader, ix: ChapterIx) -> Option<Vec<u8>> {
    match reader.extract(ix) {
        Ok(Extracted::Document(bytes)) => Some(bytes),
        _ => None,
    }
}

fn is_text_at(xml: &str, at: usize) -> bool {
    let bytes = xml.as_bytes();
    let tag_open = bytes[..at].iter().rposition(|&byte| byte == b'<');
    let tag_close = bytes[..at].iter().rposition(|&byte| byte == b'>');
    if tag_open > tag_close {
        return false;
    }
    let entity = bytes[..at].iter().rposition(|&byte| byte == b'&');
    let semi = bytes[..at].iter().rposition(|&byte| byte == b';');
    entity <= semi
}

fn letter_positions(xml: &str) -> Vec<usize> {
    xml.char_indices()
        .filter(|(at, ch)| ch.is_alphanumeric() && is_text_at(xml, *at))
        .map(|(at, _)| at)
        .collect()
}

fn other_char(ch: char) -> char {
    if ch.is_ascii() {
        if ch == 'q' { 'z' } else { 'q' }
    } else if ch == '\u{4E00}' {
        '\u{4E8C}'
    } else {
        '\u{4E00}'
    }
}

fn with_char_swapped(xml: &str, at: usize) -> String {
    let ch = xml[at..].chars().next().unwrap();
    format!(
        "{}{}{}",
        &xml[..at],
        other_char(ch),
        &xml[at + ch.len_utf8()..]
    )
}

fn changed_region<'a>(old: &'a [u8], new: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    let shared = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[shared..]
        .iter()
        .rev()
        .zip(new[shared..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (
        &old[shared..old.len() - tail],
        &new[shared..new.len() - tail],
    )
}

fn is_one_character_edit(old: &[u8], new: &[u8]) -> bool {
    let (was, now) = changed_region(old, new);
    let was_one_atom = !was.is_empty()
        && (was.len() <= 4
            || (was.len() <= 12 && was.first() == Some(&b'&') && was.last() == Some(&b';')));
    was_one_atom && !now.is_empty() && now.len() <= 4
}

fn records(old: &Path, new: &Path, changed: &str, dir: &Path) -> String {
    let list = dir.join("changed.txt");
    fs::write(&list, changed).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/zip_records.py");
    let output = Command::new("python3")
        .arg("-I")
        .arg(script)
        .args([old, new, &list])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn typo_edits(xml: &[String], rev: ChapterRev, rng: &mut Rng) -> Option<BlockEdits> {
    let candidates: Vec<usize> = (0..xml.len())
        .filter(|&block| !letter_positions(&xml[block]).is_empty())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let block = candidates[rng.below(candidates.len())];
    let positions = letter_positions(&xml[block]);
    let at = positions[rng.below(positions.len())];
    Some(BlockEdits {
        base: rev,
        changes: vec![BlockChange::Replace {
            of: BlockIx(block as u32),
            xml: with_char_swapped(&xml[block], at),
        }],
    })
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real no_op -- --ignored --nocapture"]
fn no_op_rust_round_trip_on_every_chapter_of_every_real_book() {
    let mut failures = 0;
    for path in real_books() {
        let dir = scratch(&format!("noop-{}", id_of(&path)));
        let (book, copy) = placed(&path, &dir);
        let before = fs::read(&copy).unwrap();
        let reader = book.read();
        let (mut chapters, mut blocks, mut skipped, mut bad, mut wrote) = (0, 0, 0, 0, 0);
        for row in reader.rows() {
            if !matches!(reader.extract(row.ix), Ok(Extracted::Document(_))) {
                skipped += 1;
                continue;
            }
            let rendered = reader.render(row.ix).unwrap();
            let xml = block_xml_of(&rendered.xhtml).unwrap();
            blocks += xml.len() as u64;
            chapters += 1;
            let changes = xml
                .into_iter()
                .enumerate()
                .map(|(ix, xml)| BlockChange::Replace {
                    of: BlockIx(ix as u32),
                    xml,
                })
                .collect();
            let edits = BlockEdits {
                base: rendered.rev,
                changes,
            };
            match book.save_chapter(row.ix, &edits) {
                Ok(saved) if matches!(saved.outcome, SaveOutcome::Unchanged) => {}
                Ok(_) => wrote += 1,
                Err(_) => bad += 1,
            }
        }
        let same = fs::read(&copy).unwrap() == before;
        let no_backup = fs::read_dir(dir.join("data")).unwrap().next().is_none();
        println!(
            "{:9} chapters {chapters:>5} blocks {blocks:>6} skipped {skipped:>3} wrote {wrote:>3} refused {bad:>3} file_same {same} no_backup {no_backup}",
            id_of(&path)
        );
        failures += wrote + bad + u64::from(!same) + u64::from(!no_backup);
        let _ = fs::remove_dir_all(&dir);
    }
    assert_eq!(failures, 0);
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real typo_plan -- --ignored --nocapture"]
fn typo_plan_changes_one_character_on_every_chapter_of_every_real_book() {
    let mut wrong_total = 0;
    for path in real_books() {
        let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let (mut planned, mut exact, mut repaired, mut wrong, mut empty) = (0, 0, 0, 0, 0);
        for row in reader.rows() {
            let Some(old) = raw_chapter(&reader, row.ix) else {
                continue;
            };
            let rendered = reader.render(row.ix).unwrap();
            let xml = block_xml_of(&rendered.xhtml).unwrap();
            let Some(edits) = typo_edits(&xml, rendered.rev, &mut rng) else {
                empty += 1;
                continue;
            };
            planned += 1;
            match reader.plan_preview(row.ix, &edits) {
                Ok(Some((new, warnings))) if warnings.is_empty() => {
                    if is_one_character_edit(&old, &new) {
                        exact += 1;
                    } else {
                        wrong += 1;
                        let (was, now) = changed_region(&old, &new);
                        println!(
                            "  {} chapter {}: {} bytes became {}",
                            id_of(&path),
                            row.ix.0,
                            was.len(),
                            now.len()
                        );
                    }
                }
                Ok(Some((new, _))) => {
                    repaired += 1;
                    let (point, structural) = damage_of(old.clone()).unwrap();
                    let (was, now) = changed_region(&old, &new);
                    println!(
                        "  {} chapter {} first save repairs: fixes {point:?} structural {structural} length_change {} span {}",
                        id_of(&path),
                        row.ix.0,
                        new.len() as i64 - old.len() as i64,
                        was.len().max(now.len())
                    );
                }
                Ok(None) | Err(_) => wrong += 1,
            }
        }
        println!(
            "{:9} planned {planned:>5} one_char {exact:>5} first_save_repairs {repaired:>3} wrong {wrong:>3} no_text {empty:>3}",
            id_of(&path)
        );
        wrong_total += wrong;
    }
    assert_eq!(wrong_total, 0);
}

#[derive(Default)]
struct TypoTally {
    edited: u64,
    exact: u64,
    repaired: u64,
    wrong: u64,
    bad_records: u64,
    mimetype_normalised: u64,
    no_text: u64,
    repair_bytes: usize,
}

fn typo_sweep(source: &Path, label: &str, per_book: usize) -> TypoTally {
    let dir = scratch(&format!("typo-{label}"));
    let (book, copy) = placed(source, &dir);
    let total = book.read().rows().len();
    let step = (total / per_book).max(1);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let previous = dir.join("previous.epub");
    let mut tally = TypoTally::default();
    for ix in (0..total).step_by(step) {
        let ix = ChapterIx(ix as u32);
        let reader = book.read();
        let Some(old) = raw_chapter(&reader, ix) else {
            continue;
        };
        let Ok(rendered) = reader.render(ix) else {
            continue;
        };
        let xml = block_xml_of(&rendered.xhtml).unwrap();
        let Some(edits) = typo_edits(&xml, rendered.rev, &mut rng) else {
            tally.no_text += 1;
            continue;
        };
        fs::copy(&copy, &previous).unwrap();
        let saved = book.save_chapter(ix, &edits).unwrap();
        let SaveOutcome::Written(report) = &saved.outcome else {
            panic!("a typo wrote nothing");
        };
        tally.edited += 1;
        let new = raw_chapter(&book.read(), ix).unwrap();
        if !saved.warnings.is_empty() {
            tally.repaired += 1;
            let (was, now) = changed_region(&old, &new);
            tally.repair_bytes = tally.repair_bytes.max(was.len().max(now.len()));
        } else if is_one_character_edit(&old, &new) {
            tally.exact += 1;
        } else {
            tally.wrong += 1;
        }
        let name = book.read().rows()[ix.0 as usize]
            .href
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned();
        let report_text = records(&previous, &copy, &name, &dir);
        let expected = if report.mimetype_normalised {
            tally.mimetype_normalised += 1;
            "differ_expected=1 differ_unexpected=1 removed=0"
        } else {
            "differ_expected=1 differ_unexpected=0 removed=0"
        };
        if !report_text.contains(expected) || !report_text.contains("crc_failed=0") {
            tally.bad_records += 1;
        }
    }
    let _ = fs::remove_dir_all(&dir);
    tally
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real typo_commit -- --ignored --nocapture"]
fn typo_commit_changes_only_those_bytes_and_only_that_record() {
    let (mut wrong, mut bad) = (0, 0);
    for path in real_books().iter().chain(&editable_generated_books()) {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let label = if path.starts_with(common::fixtures()) {
            stem
        } else {
            id_of(path)
        };
        let per_book = if label.starts_with("large") { 5 } else { 30 };
        let tally = typo_sweep(path, &label, per_book);
        println!(
            "{label:24} edited {:>3} one_char {:>3} first_save_repairs {:>2} largest_repair_diff {:>6} wrong {:>2} bad_records {:>2} mimetype_normalised {:>2} no_text {:>2}",
            tally.edited,
            tally.exact,
            tally.repaired,
            tally.repair_bytes,
            tally.wrong,
            tally.bad_records,
            tally.mimetype_normalised,
            tally.no_text
        );
        wrong += tally.wrong;
        bad += tally.bad_records;
    }
    assert_eq!(wrong + bad, 0);
}

fn longest_text_chapter(reader: &Reader, rows: usize) -> ChapterIx {
    let mut best = (0, ChapterIx(0));
    for ix in 0..rows {
        let ix = ChapterIx(ix as u32);
        if let Some(raw) = raw_chapter(reader, ix)
            && raw.len() > best.0
        {
            best = (raw.len(), ix);
        }
    }
    best.1
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real cycles -- --ignored --nocapture"]
fn cycles_of_insert_and_delete_return_to_identical_chapter_bytes_without_growth() {
    for id in ["serial_a", "serial_b1"] {
        let dir = scratch(&format!("cycles-{id}"));
        let (book, copy) = placed(&real_book(id), &dir);
        let original_file = fs::read(&copy).unwrap();
        let ix = longest_text_chapter(&book.read(), book.read().rows().len());
        let original = raw_chapter(&book.read(), ix).unwrap();
        let shown = block_xml_of(&book.read().render(ix).unwrap().xhtml).unwrap();
        let block = shown
            .iter()
            .position(|xml| letter_positions(xml).len() > 10)
            .unwrap();
        let at = letter_positions(&shown[block])[5];
        let typed = format!("{}x{}", &shown[block][..at], &shown[block][at..]);
        let mut rev = book.read().render(ix).unwrap().rev;
        let (mut identical, mut lengths) = (0, Vec::new());
        for cycle in 0..100 {
            let changes = |xml: &String, block: usize| {
                vec![BlockChange::Replace {
                    of: BlockIx(block as u32),
                    xml: xml.clone(),
                }]
            };
            let forward = if cycle % 2 == 0 {
                changes(&typed, block)
            } else {
                vec![BlockChange::InsertAfter {
                    after: Some(BlockIx(block as u32)),
                    xml: "<p>new paragraph</p>".to_owned(),
                }]
            };
            let saved = book
                .save_chapter(
                    ix,
                    &BlockEdits {
                        base: rev,
                        changes: forward,
                    },
                )
                .unwrap();
            assert!(matches!(saved.outcome, SaveOutcome::Written(_)));
            rev = saved.rev;
            let back = if cycle % 2 == 0 {
                changes(&shown[block], block)
            } else {
                vec![BlockChange::Delete {
                    of: BlockIx(block as u32 + 1),
                }]
            };
            let saved = book
                .save_chapter(
                    ix,
                    &BlockEdits {
                        base: rev,
                        changes: back,
                    },
                )
                .unwrap();
            assert!(matches!(saved.outcome, SaveOutcome::Written(_)));
            rev = saved.rev;
            identical += u32::from(raw_chapter(&book.read(), ix).unwrap() == original);
            lengths.push(fs::metadata(&copy).unwrap().len());
        }
        let final_file = fs::read(&copy).unwrap();
        let name = book.read().rows()[ix.0 as usize]
            .href
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned();
        println!(
            "{id} records against the original: {}",
            records(&real_book(id), &copy, &name, &dir)
        );
        let same_file_lengths = lengths.windows(2).all(|pair| pair[0] == pair[1]);
        println!(
            "{id} cycles 100 identical_chapter {identical} file_length_stable {same_file_lengths} file_identical_to_original {} file_growth_bytes {}",
            final_file == original_file,
            final_file.len() as i64 - original_file.len() as i64
        );
        assert_eq!(identical, 100);
        assert!(same_file_lengths);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real reinsert -- --ignored --nocapture"]
fn reinsert_counts_what_a_delete_then_insert_cannot_return() {
    for path in real_books() {
        let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
        let mut rng = Rng(0xABCD_EF01_2345_6789);
        let (mut tried, mut identical) = (0u32, 0u32);
        let step = (reader.rows().len() / 200).max(1);
        for ix in (0..reader.rows().len()).step_by(step) {
            let ix = ChapterIx(ix as u32);
            let Some(old) = raw_chapter(&reader, ix) else {
                continue;
            };
            let rendered = reader.render(ix).unwrap();
            let xml = block_xml_of(&rendered.xhtml).unwrap();
            if xml.len() < 2 {
                continue;
            }
            let block = 1 + rng.below(xml.len() - 1);
            let edits = BlockEdits {
                base: rendered.rev,
                changes: vec![
                    BlockChange::Delete {
                        of: BlockIx(block as u32),
                    },
                    BlockChange::InsertAfter {
                        after: Some(BlockIx(block as u32)),
                        xml: xml[block].clone(),
                    },
                ],
            };
            tried += 1;
            if let Ok(Some((new, _))) = reader.plan_preview(ix, &edits) {
                identical += u32::from(new == old);
            }
        }
        println!(
            "{:9} delete_then_reinsert tried {tried:>4} byte_identical {identical:>4}",
            id_of(&path)
        );
    }
}

fn with_host(xml: &str, session: u64) -> String {
    let prefix = "jadeslip-book://";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(prefix) {
        let end = at + prefix.len();
        out.push_str(&rest[..end]);
        out.push_str(&format!("{session:016x}"));
        rest = &rest[end + 16..];
    }
    out.push_str(rest);
    out
}

fn block_diff(
    old: &[String],
    new: &[String],
    session: u64,
) -> Result<Vec<BlockChange>, jadeslip_core::Error> {
    let tokens = |xml: &str| tokens_of(xml).map(|tokens| format!("{tokens:?}"));
    let old_tokens = old
        .iter()
        .map(|xml| tokens(xml))
        .collect::<Result<Vec<_>, _>>()?;
    let new_tokens = new
        .iter()
        .map(|xml| tokens(xml))
        .collect::<Result<Vec<_>, _>>()?;
    let shared = old_tokens
        .iter()
        .zip(&new_tokens)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = old_tokens[shared..]
        .iter()
        .rev()
        .zip(new_tokens[shared..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    let mut changes = Vec::new();
    let paired = (old_end - shared).min(new_end - shared);
    for offset in 0..paired {
        let (old_ix, new_ix) = (shared + offset, shared + offset);
        if old_tokens[old_ix] != new_tokens[new_ix] {
            changes.push(BlockChange::Replace {
                of: BlockIx(old_ix as u32),
                xml: with_host(&new[new_ix], session),
            });
        }
    }
    for ix in shared + paired..old_end {
        changes.push(BlockChange::Delete {
            of: BlockIx(ix as u32),
        });
    }
    let after = (shared + paired)
        .checked_sub(1)
        .map(|at| BlockIx(at as u32));
    for xml in &new[shared + paired..new_end] {
        changes.push(BlockChange::InsertAfter {
            after,
            xml: with_host(xml, session),
        });
    }
    Ok(changes)
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real replay -- --ignored --nocapture"]
fn replay_between_the_edited_real_copies_gives_small_patches() {
    for (from, to) in [("serial_b1", "serial_b2"), ("serial_b2", "serial_b3")] {
        let dir = scratch(&format!("replay-{from}"));
        let (book, _) = placed(&real_book(from), &dir);
        let target = Reader::open(File::open(real_book(to)).unwrap(), DEFAULT_LIMITS).unwrap();
        let session = book.read().session();
        let rows = book.read().rows().len();
        assert_eq!(rows, target.rows().len());
        let (mut chapters, mut replaced, mut deleted, mut inserted) = (0u32, 0u32, 0u32, 0u32);
        let (mut patch_bytes, mut byte_equal, mut text_equal, mut failed) =
            (0u64, 0u32, 0u32, 0u32);
        let mut largest = 0usize;
        let (mut target_span, mut residual, mut token_equal) = (0u64, 0u64, 0u32);
        for ix in 0..rows {
            let ix = ChapterIx(ix as u32);
            let reader = book.read();
            let (Some(old), Some(new)) = (raw_chapter(&reader, ix), raw_chapter(&target, ix))
            else {
                continue;
            };
            if old == new {
                continue;
            }
            chapters += 1;
            let rendered = reader.render(ix).unwrap();
            let old_xml = block_xml_of(&rendered.xhtml).unwrap();
            let new_xml = block_xml_of(&target.render(ix).unwrap().xhtml).unwrap();
            let changes = block_diff(&old_xml, &new_xml, session).unwrap();
            for change in &changes {
                match change {
                    BlockChange::Replace { .. } => replaced += 1,
                    BlockChange::Delete { .. } => deleted += 1,
                    BlockChange::InsertAfter { .. } => inserted += 1,
                }
            }
            let edits = BlockEdits {
                base: rendered.rev,
                changes,
            };
            let Ok(saved) = book.save_chapter(ix, &edits) else {
                failed += 1;
                continue;
            };
            let _ = saved;
            let replayed = raw_chapter(&book.read(), ix).unwrap();
            let (was, now) = changed_region(&old, &replayed);
            let size = was.len().max(now.len());
            patch_bytes += size as u64;
            largest = largest.max(size);
            byte_equal += u32::from(replayed == new);
            let (target_was, target_now) = changed_region(&old, &new);
            target_span += target_was.len().max(target_now.len()) as u64;
            let (left, right) = changed_region(&replayed, &new);
            residual += left.len().max(right.len()) as u64;
            let same_text = book.read().visible_text(ix).unwrap().as_str()
                == target.visible_text(ix).unwrap().as_str();
            let compared = |xml: &[String]| {
                xml.iter()
                    .map(|one| tokens_of(one).unwrap())
                    .collect::<Vec<_>>()
            };
            let replayed_xml = block_xml_of(&book.read().render(ix).unwrap().xhtml).unwrap();
            token_equal += u32::from(compared(&replayed_xml) == compared(&new_xml));
            text_equal += u32::from(same_text);
        }
        println!(
            "{from} to {to}: chapters_differing {chapters} replace {replaced} delete {deleted} insert {inserted} patch_span_bytes_total {patch_bytes} largest_patch_span {largest} source_diff_span_total {target_span} residual_span_against_target {residual} byte_equal_to_target {byte_equal} text_equal_to_target {text_equal} display_tokens_equal {token_equal} refused {failed}"
        );
        assert_eq!(failed, 0);
        assert_eq!(text_equal, chapters);
        let _ = fs::remove_dir_all(&dir);
    }
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn parses(raw: &[u8]) -> bool {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(&decoded_of(raw.to_vec()), options).is_ok()
}

fn random_edit(xml: &[String], rng: &mut Rng) -> Option<(Vec<BlockChange>, Vec<Vec<u32>>)> {
    let mut units: Vec<Vec<u32>> = xml
        .iter()
        .map(|block| block_units_of(block).ok().and_then(|mut all| all.pop()))
        .collect::<Option<Vec<_>>>()?;
    let mut changes = Vec::new();
    let mut used = std::collections::HashSet::new();
    let root_block = xml.first().is_some_and(|block| block.contains("ss-body"));
    for _ in 0..1 + rng.below(3) {
        let block = rng.below(xml.len());
        if !used.insert(block) {
            continue;
        }
        let positions = letter_positions(&xml[block]);
        if positions.len() < 2 {
            continue;
        }
        let at = positions[rng.below(positions.len())];
        let ch = xml[block][at..].chars().next().unwrap();
        let (mutated, op_is_block) = match rng.below(if root_block { 5 } else { 6 }) {
            0 => (
                format!("{}x{}", &xml[block][..at], &xml[block][at..]),
                false,
            ),
            1 => (
                format!(
                    "{}\u{4E2D}\u{1F600}{}",
                    &xml[block][..at],
                    &xml[block][at..]
                ),
                false,
            ),
            2 => (
                format!("{}{}", &xml[block][..at], &xml[block][at + ch.len_utf8()..]),
                false,
            ),
            3 => (with_char_swapped(&xml[block], at), false),
            4 => (
                format!("{}<br></br>{}", &xml[block][..at], &xml[block][at..]),
                false,
            ),
            _ => (String::new(), true),
        };
        if op_is_block {
            changes.push(BlockChange::Delete {
                of: BlockIx(block as u32),
            });
            units[block].clear();
        } else {
            units[block] = block_units_of(&mutated).ok()?.pop()?;
            changes.push(BlockChange::Replace {
                of: BlockIx(block as u32),
                xml: mutated,
            });
        }
    }
    if changes.is_empty() {
        return None;
    }
    let deleted: std::collections::HashSet<u32> = changes
        .iter()
        .filter_map(|change| match change {
            BlockChange::Delete { of } => Some(of.0),
            _ => None,
        })
        .collect();
    let units = units
        .into_iter()
        .enumerate()
        .filter(|(ix, _)| !deleted.contains(&(*ix as u32)))
        .map(|(_, block)| block)
        .collect();
    Some((changes, units))
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real property -- --ignored --nocapture"]
fn property_random_edits_on_real_chapters_keep_xml_and_intended_text() {
    let mut total_bad = 0;
    for path in real_books().iter().chain(&editable_generated_books()) {
        let dir = scratch(&format!("property-{}", id_of(path)));
        let (book, copy) = placed(path, &dir);
        let reader = book.read();
        let rows = reader.rows().len();
        let mut rng = Rng(0xFEED_FACE_CAFE_BEEF ^ rows as u64);
        let (mut cases, mut xml_gated, mut bad_xml, mut bad_text, mut refused, mut bad_url) =
            (0, 0, 0, 0, 0, 0);
        for _ in 0..400 {
            let ix = ChapterIx(rng.below(rows) as u32);
            let Some(old) = raw_chapter(&reader, ix) else {
                continue;
            };
            let Ok(rendered) = reader.render(ix) else {
                continue;
            };
            let Ok(xml) = block_xml_of(&rendered.xhtml) else {
                continue;
            };
            if xml.is_empty() {
                continue;
            }
            let Some((changes, intended)) = random_edit(&xml, &mut rng) else {
                continue;
            };
            let edits = BlockEdits {
                base: rendered.rev,
                changes,
            };
            cases += 1;
            let Ok(Some((new, _))) = reader.plan_preview(ix, &edits) else {
                refused += 1;
                continue;
            };
            bad_url += u32::from(
                occurrences(&new, b"jadeslip-book:") > occurrences(&old, b"jadeslip-book:"),
            );
            if parses(&old) {
                xml_gated += 1;
                bad_xml += u32::from(!parses(&new));
            }
            let text = visible_text_of(new).unwrap();
            bad_text += u32::from(text.check_dom_blocks(&intended).is_err());
        }
        let mut committed = 0;
        let step = (rows / 12).max(1);
        for ix in (0..rows).step_by(step) {
            let ix = ChapterIx(ix as u32);
            let current = book.read();
            let Ok(rendered) = current.render(ix) else {
                continue;
            };
            let Ok(xml) = block_xml_of(&rendered.xhtml) else {
                continue;
            };
            if xml.is_empty() {
                continue;
            }
            let Some((changes, _)) = random_edit(&xml, &mut rng) else {
                continue;
            };
            let edits = BlockEdits {
                base: rendered.rev,
                changes,
            };
            if book.save_chapter(ix, &edits).is_ok() {
                committed += 1;
            }
        }
        let opened = Reader::open(File::open(&copy).unwrap(), DEFAULT_LIMITS).is_ok();
        let renamed = dir.join("book-copy.epub");
        fs::copy(&copy, &renamed).unwrap();
        let calibre = Command::new("ebook-meta")
            .arg(&renamed)
            .output()
            .map(|output| output.status.success());
        println!(
            "{:18} cases {cases:>3} xml_checked {xml_gated:>3} bad_xml {bad_xml} bad_text {bad_text} bad_url {bad_url} refused {refused} committed {committed:>2} reader_opens {opened} ebook_meta {calibre:?}",
            path.file_stem()
                .unwrap()
                .to_string_lossy()
                .chars()
                .rev()
                .take(18)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
        total_bad += bad_xml + bad_text + bad_url + refused + u32::from(!opened);
        let _ = fs::remove_dir_all(&dir);
    }
    assert_eq!(total_bad, 0);
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "real books: cargo test --release -p jadeslip-core --test edit_real timing -- --ignored --nocapture"]
fn timing_plan_and_commit_on_the_largest_real_book_and_a_long_block() {
    let source = real_books()
        .into_iter()
        .max_by_key(|path| {
            Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS)
                .map_or(0, |reader| reader.rows().len())
        })
        .unwrap();
    let dir = scratch("timing");
    let (book, _) = placed(&source, &dir);
    let mut rng = Rng(0x0DDB_A11C_0FFE_E123);
    let rows = book.read().rows().len();
    let (mut plan_ms, mut save_ms, mut commit_ms) = (Vec::new(), Vec::new(), Vec::new());
    let mut first_save = 0.0;
    for run in 0..31 {
        let ix = ChapterIx(rng.below(rows) as u32);
        let reader = book.read();
        let rendered = reader.render(ix).unwrap();
        let xml = block_xml_of(&rendered.xhtml).unwrap();
        let Some(edits) = typo_edits(&xml, rendered.rev, &mut rng) else {
            continue;
        };
        let started = Instant::now();
        let _ = reader.plan_preview(ix, &edits).unwrap();
        let plan = started.elapsed().as_secs_f64() * 1e3;
        let started = Instant::now();
        let saved = book.save_chapter(ix, &edits).unwrap();
        let total = started.elapsed().as_secs_f64() * 1e3;
        let SaveOutcome::Written(report) = saved.outcome else {
            panic!("a typo wrote nothing");
        };
        if run == 0 {
            first_save = total;
        } else {
            plan_ms.push(plan);
            save_ms.push(total);
            commit_ms.push(report.phases.total.as_secs_f64() * 1e3);
        }
    }
    println!(
        "{} one chapter: first_save_with_backup_ms {first_save:.1} plan_median_ms {:.2} commit_median_ms {:.1} save_median_ms {:.1} runs {}",
        id_of(&source),
        median(plan_ms),
        median(commit_ms),
        median(save_ms.clone()),
        save_ms.len()
    );
    let _ = fs::remove_dir_all(&dir);

    for (label, body) in [
        ("long_words_8mb", "word ".repeat(8 << 18)),
        ("long_entities_8mb", "&#233;".repeat(8 << 20 >> 3)),
        ("long_breaks_2mb", "line<br/>".repeat(2 << 20 >> 3)),
    ] {
        let dir = scratch(&format!("timing-{label}"));
        let (book, _) = placed(&common::fixture("toc-none"), &dir);
        let name = book.read().rows()[0].href.clone().unwrap();
        let chapter = format!(
            "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p>{body}</p></body></html>"
        );
        let name = EntryName::parse(name.as_str()).unwrap();
        book.save_entries(vec![NewEntry::deflate(name, chapter.as_bytes())])
            .unwrap();
        let reader = book.read();
        let rendered = reader.render(ChapterIx(0)).unwrap();
        let xml = block_xml_of(&rendered.xhtml).unwrap();
        let edited = with_char_swapped(&xml[0], text_at(&xml[0], xml[0].len() / 2));
        let edits = BlockEdits {
            base: rendered.rev,
            changes: vec![BlockChange::Replace {
                of: BlockIx(0),
                xml: edited,
            }],
        };
        let old = raw_chapter(&reader, ChapterIx(0)).unwrap();
        let started = Instant::now();
        let (new, _) = reader
            .plan_preview(ChapterIx(0), &edits)
            .unwrap()
            .expect("a typo writes");
        let plan = started.elapsed().as_secs_f64() * 1e3;
        let (was, now) = changed_region(&old, &new);
        let started = Instant::now();
        book.save_chapter(ChapterIx(0), &edits).unwrap();
        println!(
            "{label}: chapter_bytes {} plan_ms {plan:.0} plan_plus_commit_ms {:.0} changed_old {} changed_new {}",
            old.len(),
            started.elapsed().as_secs_f64() * 1e3,
            was.len(),
            now.len()
        );
        let current = book.read();
        let current_rendered = current.render(ChapterIx(0)).unwrap();
        let shown = block_xml_of(&current_rendered.xhtml).unwrap();
        let far_apart = with_char_swapped(&shown[0], text_at(&shown[0], shown[0].len() / 4));
        let far_apart = with_char_swapped(&far_apart, text_at(&far_apart, far_apart.len() / 4 * 3));
        let far_edits = BlockEdits {
            base: current_rendered.rev,
            changes: vec![BlockChange::Replace {
                of: BlockIx(0),
                xml: far_apart,
            }],
        };
        let started = Instant::now();
        let far = current
            .plan_preview(ChapterIx(0), &far_edits)
            .unwrap()
            .expect("two typos write")
            .0;
        let plan = started.elapsed().as_secs_f64() * 1e3;
        let current_raw = raw_chapter(&current, ChapterIx(0)).unwrap();
        let differing = current_raw.iter().zip(&far).filter(|(a, b)| a != b).count();
        println!(
            "{label} two_far_typos: plan_ms {plan:.0} of {} bytes, length_change {} differing_bytes_if_same_length {differing}",
            current_raw.len(),
            far.len() as i64 - current_raw.len() as i64
        );
        let start = shown[0].find('>').unwrap() + 1;
        let end = shown[0].rfind('<').unwrap();
        let rewritten = format!(
            "{}{}{}",
            &shown[0][..start],
            shown[0][start..end]
                .replace('e', "E")
                .replace('o', "x")
                .replace('\u{E9}', "\u{E8}"),
            &shown[0][end..]
        );
        let rewrite_edits = BlockEdits {
            base: current_rendered.rev,
            changes: vec![BlockChange::Replace {
                of: BlockIx(0),
                xml: rewritten,
            }],
        };
        let started = Instant::now();
        let outcome = current
            .plan_preview(ChapterIx(0), &rewrite_edits)
            .map(|plan| plan.is_some());
        println!(
            "{label} many_edits_past_the_cap: plan_ms {:.0} wrote {:?}",
            started.elapsed().as_secs_f64() * 1e3,
            outcome.ok()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

fn text_at(xml: &str, mut at: usize) -> usize {
    while !xml.is_char_boundary(at)
        || !xml[at..].starts_with(char::is_alphanumeric)
        || !is_text_at(xml, at)
    {
        at += 1;
    }
    at
}
