use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/history-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn log(&self) -> PathBuf {
        self.0.join("history").join(LOG_FILE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn packed(text: &str) -> Packed {
    Packed {
        method: 0,
        crc32: crc32fast::hash(text.as_bytes()),
        inflated: text.len() as u64,
        body: text.as_bytes().to_vec(),
    }
}

fn file(tag: u8) -> FileHash {
    FileHash([tag; 32])
}

fn record(entry: &str, before: Option<&Packed>, from: u8, to: u8) -> Record {
    Record {
        kind: RecordKind::Save,
        at: u64::from(from),
        rows: 3,
        file_before: file(from),
        file_after: file(to),
        changes: vec![Change {
            entry: EntryName::parse(entry).unwrap(),
            before: before.map(BlobKey::of),
        }],
        carried: Vec::new(),
    }
}

fn reopened(scratch: &Scratch) -> History {
    History::new(&scratch.0)
}

#[test]
fn a_version_kept_twice_is_written_once_and_reads_back_byte_exact() {
    let scratch = Scratch::new("dedupe");
    let old = packed(&"chapter text ".repeat(500));
    let mut history = reopened(&scratch);
    history
        .append(vec![old.clone()], record("a.xhtml", Some(&old), 1, 2))
        .unwrap();
    let once = fs::metadata(scratch.log()).unwrap().len();
    history
        .append(
            vec![old.clone(), old.clone()],
            record("b.xhtml", Some(&old), 2, 3),
        )
        .unwrap();
    let twice = fs::metadata(scratch.log()).unwrap().len();
    assert!(
        twice - once < old.body.len() as u64,
        "the second record stored the version again: grew {}",
        twice - once
    );
    let mut fresh = reopened(&scratch);
    let records = fresh.records().unwrap().to_vec();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1], record("b.xhtml", Some(&old), 2, 3));
    assert_eq!(fresh.blob(&BlobKey::of(&old)).unwrap(), old);
}

#[test]
fn a_torn_tail_is_ignored_and_the_next_append_replaces_it() {
    let scratch = Scratch::new("torn");
    let one = packed("first version");
    let two = packed(&"second version ".repeat(400));
    let mut history = reopened(&scratch);
    history
        .append(vec![one.clone()], record("a.xhtml", Some(&one), 1, 2))
        .unwrap();
    let kept = fs::metadata(scratch.log()).unwrap().len();
    history
        .append(vec![two.clone()], record("a.xhtml", Some(&two), 2, 3))
        .unwrap();
    let full = fs::metadata(scratch.log()).unwrap().len();
    for cut in [kept + 1, kept + 13, (kept + full) / 2, full - 1] {
        let bytes = fs::read(scratch.log()).unwrap();
        fs::write(scratch.log(), &bytes[..cut as usize]).unwrap();
        let mut torn = reopened(&scratch);
        assert_eq!(torn.records().unwrap().len(), 1, "cut at {cut}");
        torn.append(Vec::new(), record("c.xhtml", None, 2, 4))
            .unwrap();
        let mut fresh = reopened(&scratch);
        let records = fresh.records().unwrap().to_vec();
        assert_eq!(records.len(), 2, "cut at {cut}");
        assert_eq!(records[1].changes[0].entry.as_str(), "c.xhtml");
        assert_eq!(
            fresh.refresh().unwrap().end,
            fs::metadata(scratch.log()).unwrap().len(),
            "cut at {cut}: torn bytes were left in the log"
        );
        fs::write(scratch.log(), &bytes).unwrap();
    }
}

#[test]
fn a_changed_record_or_version_is_not_trusted() {
    let scratch = Scratch::new("damaged");
    let one = packed("first version body");
    let mut history = reopened(&scratch);
    history
        .append(vec![one.clone()], record("a.xhtml", Some(&one), 1, 2))
        .unwrap();
    history
        .append(Vec::new(), record("b.xhtml", None, 2, 3))
        .unwrap();
    let bytes = fs::read(scratch.log()).unwrap();
    let mut damaged_record = bytes.clone();
    let rows = memchr::memmem::rfind(&damaged_record, b"\"rows\":3").unwrap();
    damaged_record[rows + 7] = b'2';
    fs::write(scratch.log(), &damaged_record).unwrap();
    assert_eq!(reopened(&scratch).records().unwrap().len(), 1);

    let mut damaged_blob = bytes;
    let body_at = memchr::memmem::find(&damaged_blob, b"first version").unwrap();
    damaged_blob[body_at] ^= 1;
    fs::write(scratch.log(), &damaged_blob).unwrap();
    let mut fresh = reopened(&scratch);
    assert_eq!(fresh.records().unwrap().len(), 2);
    assert!(fresh.blob(&BlobKey::of(&one)).is_err());
}

#[test]
fn landed_points_drop_saves_that_never_reached_the_file_and_stop_at_gaps() {
    let one = packed("one");
    let name = EntryName::parse("a.xhtml").unwrap();
    let records = vec![
        record("a.xhtml", Some(&one), 1, 2),
        record("a.xhtml", None, 2, 9),
        record("b.xhtml", None, 2, 3),
    ];
    let view = landed(&records, file(3));
    let ids: Vec<u32> = view.points.iter().map(|(id, _)| id.0).collect();
    assert_eq!(ids, [0, 2], "the save that never landed is dropped");
    let at = view.state_at(RecordIx(0)).unwrap();
    assert!(at.complete);
    assert_eq!(at.version(&name), Version::Then(Some(BlobKey::of(&one))));
    assert!(view.state_at(RecordIx(1)).is_none());

    let trailing = landed(&records[..1], file(1));
    assert!(
        trailing.points.is_empty(),
        "a last save that never landed is dropped"
    );

    let gapped = vec![
        record("a.xhtml", Some(&one), 1, 2),
        record("b.xhtml", None, 5, 6),
    ];
    let view = landed(&gapped, file(6));
    let at = view.state_at(RecordIx(0)).unwrap();
    assert!(!at.complete, "a gap after the point makes it incomplete");
    assert_eq!(at.version(&name), Version::Then(Some(BlobKey::of(&one))));
    assert_eq!(
        at.version(&EntryName::parse("b.xhtml").unwrap()),
        Version::Unknown
    );
    assert!(view.state_at(RecordIx(1)).unwrap().complete);
    assert!(
        !landed(&gapped, file(7))
            .state_at(RecordIx(1))
            .unwrap()
            .complete
    );
}

#[test]
fn carried_versions_count_after_the_points_own_changes_and_survive_a_reopen() {
    let scratch = Scratch::new("carried");
    let mine = packed("the point's own version");
    let folded = packed("a version carried from a later save");
    let shadowed = packed("a later version of the point's own chapter");
    let mut carrying = record("a.xhtml", Some(&mine), 1, 2);
    carrying.carried = vec![
        Change {
            entry: EntryName::parse("a.xhtml").unwrap(),
            before: Some(BlobKey::of(&shadowed)),
        },
        Change {
            entry: EntryName::parse("b.xhtml").unwrap(),
            before: Some(BlobKey::of(&folded)),
        },
    ];
    let mut history = reopened(&scratch);
    history
        .append(
            vec![mine.clone(), folded.clone(), shadowed],
            carrying.clone(),
        )
        .unwrap();
    history
        .append(Vec::new(), record("c.xhtml", None, 2, 3))
        .unwrap();
    let mut fresh = reopened(&scratch);
    let records = fresh.records().unwrap().to_vec();
    assert_eq!(records[0], carrying);
    let at = landed(&records, file(3)).state_at(RecordIx(0)).unwrap();
    assert!(at.complete);
    assert_eq!(
        at.version(&EntryName::parse("a.xhtml").unwrap()),
        Version::Then(Some(BlobKey::of(&mine)))
    );
    assert_eq!(
        at.version(&EntryName::parse("b.xhtml").unwrap()),
        Version::Then(Some(BlobKey::of(&folded)))
    );
    assert_eq!(
        at.version(&EntryName::parse("c.xhtml").unwrap()),
        Version::Then(None)
    );
    assert_eq!(fresh.blob(&BlobKey::of(&folded)).unwrap(), folded);
}

#[test]
fn a_record_without_carried_versions_is_written_as_before() {
    let one = packed("one");
    let bytes = record("a.xhtml", Some(&one), 1, 2).to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("carried"));
    assert_eq!(
        Record::parse(&bytes).unwrap(),
        record("a.xhtml", Some(&one), 1, 2)
    );
}

#[test]
fn a_log_replaced_under_an_open_history_is_read_afresh() {
    let scratch = Scratch::new("replaced");
    let small = packed("small");
    let mut open = reopened(&scratch);
    open.append(vec![small.clone()], record("a.xhtml", Some(&small), 1, 2))
        .unwrap();
    assert_eq!(open.records().unwrap().len(), 1);

    let other = Scratch::new("replacement");
    let big = packed(&"a much longer replacement version ".repeat(200));
    let mut writer = reopened(&other);
    writer
        .append(vec![big.clone()], record("b.xhtml", Some(&big), 5, 6))
        .unwrap();
    writer
        .append(Vec::new(), record("c.xhtml", None, 6, 7))
        .unwrap();
    let replacement = scratch.log().with_extension("new");
    fs::copy(other.log(), &replacement).unwrap();
    fs::rename(&replacement, scratch.log()).unwrap();

    assert_eq!(open.blob(&BlobKey::of(&big)).unwrap(), big);
    assert!(open.blob(&BlobKey::of(&small)).is_err());
    let records = open.records().unwrap().to_vec();
    assert_eq!(
        records,
        [
            record("b.xhtml", Some(&big), 5, 6),
            record("c.xhtml", None, 6, 7)
        ]
    );
    open.append(Vec::new(), record("d.xhtml", None, 7, 8))
        .unwrap();
    let mut fresh = reopened(&scratch);
    assert_eq!(fresh.records().unwrap().len(), 3);
    assert_eq!(fresh.blob(&BlobKey::of(&big)).unwrap(), big);
}

fn entry(name: &str) -> EntryName {
    EntryName::parse(name).unwrap()
}

fn point(
    kind: RecordKind,
    at: u64,
    from: u8,
    to: u8,
    changes: &[(&str, Option<&Packed>)],
) -> Record {
    Record {
        kind,
        at,
        rows: 3,
        file_before: file(from),
        file_after: file(to),
        changes: changes
            .iter()
            .map(|(name, before)| Change {
                entry: entry(name),
                before: before.map(BlobKey::of),
            })
            .collect(),
        carried: Vec::new(),
    }
}

fn named(name: &str, at: u64, on: u8) -> Record {
    point(
        RecordKind::Named {
            name: name.to_owned(),
        },
        at,
        on,
        on,
        &[],
    )
}

fn written(scratch: &Scratch, records: &[(Vec<Packed>, Record)]) -> History {
    let mut history = reopened(scratch);
    for (blobs, record) in records {
        history.append(blobs.clone(), record.clone()).unwrap();
    }
    reopened(scratch)
}

fn temp_of(scratch: &Scratch) -> PathBuf {
    scratch.log().with_file_name("log.new")
}

#[test]
fn a_cutoff_drops_only_older_automatic_points_and_folds_them_into_the_kept_one_before() {
    let scratch = Scratch::new("prune-age");
    let (v0, v1, v2) = (packed("a zero"), packed("b one"), packed("a two"));
    let records = vec![
        point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&v0))]),
        point(RecordKind::Save, 20, 2, 3, &[("b.xhtml", Some(&v1))]),
        named("start", 30, 3),
        point(RecordKind::Save, 40, 3, 4, &[("a.xhtml", Some(&v2))]),
        point(RecordKind::Save, 50, 4, 5, &[("c.xhtml", None)]),
    ];
    let mut history = written(
        &scratch,
        &[
            (vec![v0.clone()], records[0].clone()),
            (vec![v1.clone()], records[1].clone()),
            (Vec::new(), records[2].clone()),
            (vec![v2.clone()], records[3].clone()),
            (Vec::new(), records[4].clone()),
        ],
    );
    let before = landed(&records, file(5)).state_at(RecordIx(2)).unwrap();
    let size = fs::metadata(scratch.log()).unwrap().len();

    let pruned = history.prune(file(5), 45, false).unwrap();
    assert_eq!(pruned.points, 3);
    assert_eq!(pruned.bytes_before, size);
    assert_eq!(
        pruned.bytes_after,
        fs::metadata(scratch.log()).unwrap().len()
    );
    assert!(pruned.bytes_after < size);
    assert!(!temp_of(&scratch).exists());

    let mut fresh = reopened(&scratch);
    let kept = fresh.records().unwrap().to_vec();
    let kinds: Vec<(&RecordKind, u64)> = kept
        .iter()
        .map(|record| (&record.kind, record.at))
        .collect();
    assert_eq!(
        kinds,
        [(&records[2].kind, 30), (&RecordKind::Save, 50)],
        "only the snapshot and the newer save are left"
    );
    assert_eq!(
        landed(&kept, file(5)).state_at(RecordIx(0)).unwrap(),
        before
    );
    assert_eq!(fresh.blob(&BlobKey::of(&v2)).unwrap(), v2);
    assert!(fresh.blob(&BlobKey::of(&v0)).is_err());
    assert!(fresh.blob(&BlobKey::of(&v1)).is_err());
}

#[test]
fn nothing_older_than_the_cutoff_leaves_the_log_file_alone() {
    let scratch = Scratch::new("prune-none");
    let one = packed("one");
    let mut history = written(
        &scratch,
        &[
            (
                vec![one.clone()],
                point(RecordKind::Save, 100, 1, 2, &[("a.xhtml", Some(&one))]),
            ),
            (
                Vec::new(),
                point(RecordKind::Save, 200, 2, 3, &[("b.xhtml", None)]),
            ),
        ],
    );
    let bytes = fs::read(scratch.log()).unwrap();
    let modified = fs::metadata(scratch.log()).unwrap().modified().unwrap();
    let pruned = history.prune(file(3), 100, false).unwrap();
    assert_eq!(
        pruned,
        Pruned {
            points: 0,
            bytes_before: bytes.len() as u64,
            bytes_after: bytes.len() as u64
        }
    );
    assert_eq!(fs::read(scratch.log()).unwrap(), bytes);
    assert_eq!(
        fs::metadata(scratch.log()).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(
        reopened(&Scratch::new("prune-empty"))
            .prune(file(1), 0, false)
            .unwrap(),
        Pruned::default()
    );
}

#[test]
fn the_newest_point_is_kept_even_when_it_is_older_than_the_cutoff() {
    let scratch = Scratch::new("prune-newest");
    let (one, two) = (packed("one"), packed("two"));
    let mut history = written(
        &scratch,
        &[
            (
                vec![one.clone()],
                point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
            ),
            (
                vec![two.clone()],
                point(RecordKind::Save, 20, 2, 3, &[("a.xhtml", Some(&two))]),
            ),
        ],
    );
    let pruned = history.prune(file(3), u64::MAX, false).unwrap();
    assert_eq!(pruned.points, 1);
    let kept = reopened(&scratch).records().unwrap().to_vec();
    assert_eq!(
        kept,
        [point(
            RecordKind::Save,
            20,
            2,
            3,
            &[("a.xhtml", Some(&two))]
        )]
    );
}

#[test]
fn a_gap_after_a_kept_point_survives_and_only_saves_before_it_are_folded() {
    let scratch = Scratch::new("prune-gap");
    let (v1, v2, v3) = (packed("a one"), packed("b two"), packed("c three"));
    let records = vec![
        named("start", 10, 1),
        point(RecordKind::Save, 20, 1, 2, &[("a.xhtml", Some(&v1))]),
        point(RecordKind::Save, 30, 7, 8, &[("b.xhtml", Some(&v2))]),
        point(RecordKind::Save, 40, 8, 9, &[("c.xhtml", Some(&v3))]),
    ];
    let mut history = written(
        &scratch,
        &[
            (Vec::new(), records[0].clone()),
            (vec![v1.clone()], records[1].clone()),
            (vec![v2.clone()], records[2].clone()),
            (vec![v3.clone()], records[3].clone()),
        ],
    );
    let before = landed(&records, file(9)).state_at(RecordIx(0)).unwrap();
    assert!(!before.complete);
    history.prune(file(9), 35, false).unwrap();
    let kept = reopened(&scratch).records().unwrap().to_vec();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].file_after, file(2), "the fold stops at the gap");
    let after = landed(&kept, file(9)).state_at(RecordIx(0)).unwrap();
    assert_eq!(after, before);
    assert!(!after.complete);
    assert_eq!(
        after.version(&entry("a.xhtml")),
        Version::Then(Some(BlobKey::of(&v1)))
    );
    assert_eq!(after.version(&entry("b.xhtml")), Version::Unknown);
}

#[test]
fn restores_point_at_their_kept_target_and_a_dropped_target_is_no_longer_listed() {
    let scratch = Scratch::new("prune-restore");
    let one = packed("one");
    let records = [
        point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
        named("start", 20, 2),
        point(RecordKind::Save, 30, 2, 3, &[("a.xhtml", None)]),
        point(
            RecordKind::Restore { to: RecordIx(1) },
            40,
            3,
            4,
            &[("b.xhtml", None)],
        ),
        point(
            RecordKind::Restore { to: RecordIx(0) },
            50,
            4,
            5,
            &[("c.xhtml", None)],
        ),
        point(
            RecordKind::Restore { to: RecordIx(4) },
            60,
            5,
            6,
            &[("d.xhtml", None)],
        ),
    ];
    let mut history = written(
        &scratch,
        &records
            .iter()
            .map(|record| (vec![one.clone()], record.clone()))
            .collect::<Vec<_>>(),
    );
    history.prune(file(6), 35, false).unwrap();
    let kinds: Vec<RecordKind> = reopened(&scratch)
        .records()
        .unwrap()
        .iter()
        .map(|record| record.kind.clone())
        .collect();
    assert_eq!(
        kinds,
        [
            records[1].kind.clone(),
            RecordKind::Restore { to: RecordIx(0) },
            RecordKind::Restore {
                to: RecordIx(u32::MAX)
            },
            RecordKind::Restore { to: RecordIx(2) },
        ]
    );
}

#[test]
fn saves_that_never_landed_are_dropped_and_not_counted_as_points() {
    let scratch = Scratch::new("prune-unlanded");
    let one = packed("one");
    let mut history = written(
        &scratch,
        &[
            (
                vec![one.clone()],
                point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
            ),
            (
                Vec::new(),
                point(RecordKind::Save, 20, 2, 9, &[("b.xhtml", None)]),
            ),
            (
                Vec::new(),
                point(RecordKind::Save, 30, 2, 3, &[("c.xhtml", None)]),
            ),
            (
                Vec::new(),
                point(RecordKind::Save, 40, 3, 8, &[("d.xhtml", None)]),
            ),
        ],
    );
    let pruned = history.prune(file(3), 0, false).unwrap();
    assert_eq!(pruned.points, 0);
    let kept = reopened(&scratch).records().unwrap().to_vec();
    let ats: Vec<u64> = kept.iter().map(|record| record.at).collect();
    assert_eq!(ats, [10, 30]);
}

#[test]
fn a_dry_run_reports_the_same_numbers_and_writes_nothing() {
    let scratch = Scratch::new("prune-dry");
    let (one, two) = (packed(&"one ".repeat(300)), packed("two"));
    let records = [
        (
            vec![one.clone()],
            point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
        ),
        (
            vec![two.clone()],
            point(RecordKind::Save, 20, 2, 3, &[("b.xhtml", Some(&two))]),
        ),
        (Vec::new(), named("now", 30, 3)),
        (
            Vec::new(),
            point(RecordKind::Save, 40, 3, 4, &[("c.xhtml", None)]),
        ),
    ];
    let mut history = written(&scratch, &records);
    let bytes = fs::read(scratch.log()).unwrap();
    let dry = history.prune(file(4), 25, true).unwrap();
    assert_eq!(fs::read(scratch.log()).unwrap(), bytes);
    assert!(!temp_of(&scratch).exists());
    let real = history.prune(file(4), 25, false).unwrap();
    assert_eq!(dry, real);
    assert_eq!(real.points, 2);
}

#[test]
fn a_temp_left_by_a_crash_is_never_read_and_the_next_prune_replaces_it() {
    let scratch = Scratch::new("prune-leftover");
    let one = packed("one");
    let records = [
        (
            vec![one.clone()],
            point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
        ),
        (
            Vec::new(),
            point(RecordKind::Save, 20, 2, 3, &[("b.xhtml", None)]),
        ),
    ];
    let mut history = written(&scratch, &records);
    let bytes = fs::read(scratch.log()).unwrap();
    fs::write(temp_of(&scratch), &bytes[..bytes.len() / 2]).unwrap();
    assert_eq!(reopened(&scratch).records().unwrap().len(), 2);
    assert_eq!(history.prune(file(3), 0, false).unwrap().points, 0);
    assert!(
        !temp_of(&scratch).exists(),
        "a prune with nothing to drop still clears it"
    );

    fs::write(temp_of(&scratch), b"JSH1 garbage").unwrap();
    history.prune(file(3), 15, false).unwrap();
    assert!(!temp_of(&scratch).exists());
    let kept = reopened(&scratch).records().unwrap().to_vec();
    assert_eq!(kept, [records[1].1.clone()]);
}

#[test]
fn a_failure_at_any_prune_step_leaves_the_old_or_the_new_log_and_the_next_prune_or_save_works() {
    use crate::commit::seam::install;
    let (one, two) = (packed(&"one ".repeat(200)), packed("two"));
    let records = [
        (Vec::new(), named("start", 5, 1)),
        (
            vec![one.clone()],
            point(RecordKind::Save, 10, 1, 2, &[("a.xhtml", Some(&one))]),
        ),
        (
            vec![two.clone()],
            point(RecordKind::Save, 20, 2, 3, &[("b.xhtml", Some(&two))]),
        ),
        (
            Vec::new(),
            point(RecordKind::Save, 40, 3, 4, &[("c.xhtml", None)]),
        ),
    ];
    let reference = Scratch::new("prune-kill-reference");
    written(&reference, &records)
        .prune(file(4), 30, false)
        .unwrap();
    let new_log = fs::read(reference.log()).unwrap();
    let new_state = landed(reopened(&reference).records().unwrap(), file(4))
        .state_at(RecordIx(0))
        .unwrap();

    for step in [
        Step::PruneWriting,
        Step::PruneWritten,
        Step::PruneSynced,
        Step::PruneRenamed,
        Step::PruneDirSynced,
    ] {
        let scratch = Scratch::new("prune-kill");
        let mut history = written(&scratch, &records);
        let old_log = fs::read(scratch.log()).unwrap();
        let mut fired = false;
        install(move |seen| {
            if seen == step && !fired {
                fired = true;
                return Err(io::Error::other("injected"));
            }
            Ok(())
        });
        let result = history.prune(file(4), 30, false);
        install(|_| Ok(()));
        let on_disk = fs::read(scratch.log()).unwrap();
        let renamed = matches!(step, Step::PruneRenamed | Step::PruneDirSynced);
        assert_eq!(result.is_ok(), renamed, "{step:?}");
        assert_eq!(
            &on_disk,
            if renamed { &new_log } else { &old_log },
            "{step:?}"
        );
        assert!(!temp_of(&scratch).exists(), "{step:?}");

        let mut fresh = reopened(&scratch);
        let kept = fresh.records().unwrap().to_vec();
        assert_eq!(
            fresh.refresh().unwrap().end,
            on_disk.len() as u64,
            "{step:?}: every frame scans"
        );
        let state = landed(&kept, file(4)).state_at(RecordIx(0)).unwrap();
        if renamed {
            assert_eq!(state, new_state, "{step:?}");
        }
        for key in state.known.values().flatten() {
            assert!(fresh.blob(key).is_ok(), "{step:?}");
        }

        let again = history.prune(file(4), 30, false).unwrap();
        assert_eq!(again.points, if renamed { 0 } else { 2 }, "{step:?}");
        assert_eq!(fs::read(scratch.log()).unwrap(), new_log, "{step:?}");
        history
            .append(
                Vec::new(),
                point(RecordKind::Save, 50, 4, 5, &[("d.xhtml", None)]),
            )
            .unwrap();
        assert_eq!(reopened(&scratch).records().unwrap().len(), 3, "{step:?}");
    }
}

fn prose(seed: u64, len: usize) -> String {
    const WORDS: [&str; 32] = [
        "the", "river", "sword", "quietly", "moon", "said", "Lin", "mountain", "sect", "elder",
        "qi", "and", "of", "a", "heaven", "disciple", "turned", "toward", "blade", "spirit",
        "stone", "night", "cold", "wind", "her", "his", "smiled", "before", "under", "golden",
        "core", "realm",
    ];
    let mut state = seed;
    let mut out = String::with_capacity(len + 16);
    while out.len() < len {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push_str(WORDS[(state >> 59) as usize]);
        out.push(if (state >> 32).is_multiple_of(11) {
            '\n'
        } else {
            ' '
        });
    }
    out.truncate(len);
    out
}

fn edited(count: usize, len: usize) -> Vec<String> {
    let text = prose(7, len);
    (0..count)
        .map(|ix| {
            let at = len / 3 + ix * 211 % (len / 3);
            format!("{}<p>edit {ix}</p>{}", &text[..at], &text[at..])
        })
        .collect()
}

fn saved_versions(scratch: &Scratch, texts: &[String]) -> (History, Vec<Packed>) {
    let versions: Vec<Packed> = texts
        .iter()
        .map(|text| Packed::of(text.as_bytes()))
        .collect();
    let records: Vec<(Vec<Packed>, Record)> = versions
        .iter()
        .enumerate()
        .map(|(ix, version)| {
            let at = 10 * (ix as u64 + 1);
            let (from, to) = (ix as u8, ix as u8 + 1);
            let record = point(
                RecordKind::Save,
                at,
                from,
                to,
                &[("a.xhtml", Some(version))],
            );
            (vec![version.clone()], record)
        })
        .collect();
    (written(scratch, &records), versions)
}

fn readable_versions(history: &mut History, texts: &[String], versions: &[Packed]) -> Vec<bool> {
    texts
        .iter()
        .zip(versions)
        .map(
            |(text, version)| match history.text(&BlobKey::of(version)) {
                Ok(back) => {
                    assert_eq!(back, text.as_bytes(), "a version read back as other text");
                    true
                }
                Err(error) => {
                    assert_eq!(error.user_message(), damaged().user_message());
                    false
                }
            },
        )
        .collect()
}

fn reframed(bytes: &mut [u8], at: u64) {
    let at = at as usize;
    let head = at - HEAD_LEN as usize;
    let len = le32(bytes, head + 5) as usize;
    let crc = crc32fast::hash(&bytes[at..at + len]);
    bytes[head + 9..head + 13].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn squeezed_text_expands_back_byte_exact_with_and_without_a_base() {
    let long = prose(3, 100_000);
    let mut changed = long.clone().into_bytes();
    for at in [5_000, 40_000, 70_123, 99_990] {
        changed[at] = b'#';
    }
    let cases: [(&[u8], Option<&[u8]>); 8] = [
        (b"", None),
        (b"", Some(b"")),
        (b"", Some(long.as_bytes())),
        (long.as_bytes(), Some(b"")),
        (long.as_bytes(), None),
        (&changed, Some(long.as_bytes())),
        (&changed[..9_000], Some(long.as_bytes())),
        (long.as_bytes(), Some(&changed[..3_000])),
    ];
    for (text, base) in cases {
        let label = (text.len(), base.map(<[u8]>::len));
        let body = text::squeeze(text, base).unwrap();
        let len = text.len() as u64;
        assert_eq!(
            text::expand(&body, base, len).as_deref(),
            Some(text),
            "{label:?}"
        );
        let mut junk = body.clone();
        junk.push(0);
        assert_eq!(text::expand(&junk, base, len), None, "{label:?}");
        assert_eq!(text::expand(&body, base, len + 1), None, "{label:?}");
        if !text.is_empty() {
            assert_eq!(text::expand(&body, base, len - 1), None, "{label:?}");
        }
    }
}

#[test]
fn a_long_chapter_with_a_small_edit_squeezes_to_a_small_difference() {
    let base = prose(11, 100_000);
    let text = format!(
        "{}a new line{}another{}",
        &base[..20_000],
        &base[20_000..61_000],
        &base[61_500..]
    );
    let alone = text::squeeze(text.as_bytes(), None).unwrap();
    let against = text::squeeze(text.as_bytes(), Some(base.as_bytes())).unwrap();
    assert!(
        against.len() * 10 < alone.len(),
        "{} bytes against the base, {} alone",
        against.len(),
        alone.len()
    );
}

#[test]
fn forty_five_versions_read_back_byte_exact_after_compaction_in_far_less_room() {
    let scratch = Scratch::new("compact-round-trip");
    let texts = edited(45, 20_000);
    let (mut history, versions) = saved_versions(&scratch, &texts);
    let records = history.records().unwrap().to_vec();
    let pruned = history.prune(file(45), 0, false).unwrap();
    assert_eq!(pruned.points, 0);
    assert_eq!(
        pruned.bytes_after,
        fs::metadata(scratch.log()).unwrap().len()
    );
    assert!(pruned.bytes_after * 5 < pruned.bytes_before, "{pruned:?}");

    let mut fresh = reopened(&scratch);
    assert_eq!(fresh.records().unwrap(), records);
    assert_eq!(readable_versions(&mut fresh, &texts, &versions), [true; 45]);
    for (text, version) in texts.iter().zip(&versions) {
        let packed = fresh.blob(&BlobKey::of(version)).unwrap();
        assert_eq!(
            (packed.crc32, packed.inflated),
            (version.crc32, version.inflated)
        );
        assert_eq!(packed.unpack("a.xhtml").unwrap(), text.as_bytes());
    }
}

#[test]
fn a_second_cleanup_leaves_a_compacted_log_byte_identical_even_after_a_new_save() {
    let scratch = Scratch::new("compact-twice");
    let texts = edited(46, 20_000);
    let (mut history, versions) = saved_versions(&scratch, &texts[..45]);
    history.prune(file(45), 0, false).unwrap();
    let once = fs::read(scratch.log()).unwrap();
    let again = history.prune(file(45), 0, false).unwrap();
    assert_eq!(
        again,
        Pruned {
            points: 0,
            bytes_before: once.len() as u64,
            bytes_after: once.len() as u64
        }
    );
    assert_eq!(fs::read(scratch.log()).unwrap(), once);

    let newest = Packed::of(texts[45].as_bytes());
    history
        .append(
            vec![newest.clone()],
            point(RecordKind::Save, 460, 45, 46, &[("a.xhtml", Some(&newest))]),
        )
        .unwrap();
    let grown = fs::metadata(scratch.log()).unwrap().len();
    let compacted = history.prune(file(46), 0, false).unwrap();
    assert!(compacted.bytes_after < grown, "{compacted:?}");
    let settled = fs::read(scratch.log()).unwrap();
    assert_eq!(
        history.prune(file(46), 0, false).unwrap().bytes_after,
        settled.len() as u64
    );
    assert_eq!(fs::read(scratch.log()).unwrap(), settled);
    let mut all = versions;
    all.push(newest);
    assert_eq!(
        readable_versions(&mut reopened(&scratch), &texts, &all),
        [true; 46]
    );
}

#[test]
fn saving_an_old_text_again_after_compaction_stores_no_new_copy() {
    let scratch = Scratch::new("compact-dedupe");
    let texts = edited(30, 20_000);
    let (mut history, versions) = saved_versions(&scratch, &texts);
    history.prune(file(30), 0, false).unwrap();
    let before = fs::metadata(scratch.log()).unwrap().len();
    let again = point(
        RecordKind::Save,
        400,
        30,
        31,
        &[("a.xhtml", Some(&versions[10]))],
    );
    history
        .append(vec![versions[10].clone()], again.clone())
        .unwrap();
    assert_eq!(
        fs::metadata(scratch.log()).unwrap().len() - before,
        HEAD_LEN + again.to_bytes().len() as u64
    );
    let mut fresh = reopened(&scratch);
    assert_eq!(
        fresh.text(&BlobKey::of(&versions[10])).unwrap(),
        texts[10].as_bytes()
    );
    let meta = fresh.blob_meta(&BlobKey::of(&versions[10])).unwrap();
    assert_eq!(
        (meta.crc32, meta.inflated),
        (versions[10].crc32, versions[10].inflated)
    );
}

#[test]
fn a_version_whose_base_was_cleaned_away_is_rebuilt_against_the_next_kept_one() {
    let scratch = Scratch::new("compact-dropped-base");
    let texts = edited(45, 20_000);
    let versions: Vec<Packed> = texts
        .iter()
        .map(|text| Packed::of(text.as_bytes()))
        .collect();
    let mut records = vec![(Vec::new(), named("start", 5, 0))];
    records.extend(versions.iter().enumerate().map(|(ix, version)| {
        let at = 10 * (ix as u64 + 1);
        let (from, to) = (ix as u8, ix as u8 + 1);
        let record = point(
            RecordKind::Save,
            at,
            from,
            to,
            &[("a.xhtml", Some(version))],
        );
        (vec![version.clone()], record)
    }));
    let mut history = written(&scratch, &records);
    history.prune(file(45), 0, false).unwrap();
    let pruned = history.prune(file(45), 300, false).unwrap();
    assert_eq!(pruned.points, 29);

    let mut fresh = reopened(&scratch);
    let kept = fresh.records().unwrap().to_vec();
    let start = landed(&kept, file(45)).state_at(RecordIx(0)).unwrap();
    assert_eq!(
        start.version(&entry("a.xhtml")),
        Version::Then(Some(BlobKey::of(&versions[0])))
    );
    let expected: Vec<bool> = (0..45).map(|ix| ix == 0 || ix >= 29).collect();
    assert_eq!(readable_versions(&mut fresh, &texts, &versions), expected);
}

#[test]
fn a_damaged_base_makes_the_versions_built_on_it_damaged_never_wrong() {
    let texts = edited(45, 20_000);
    for fix_frame in [false, true] {
        let scratch = Scratch::new("compact-damaged-base");
        let (mut history, versions) = saved_versions(&scratch, &texts);
        history.prune(file(45), 0, false).unwrap();
        let meta = reopened(&scratch).refresh().unwrap().blobs[&BlobKey::of(&versions[43])];
        assert_eq!(meta.form, Form::Text(Some(BlobKey::of(&versions[44]))));
        let mut bytes = fs::read(scratch.log()).unwrap();
        bytes[meta.at as usize + TEXT_META_LEN + 9] ^= 0x10;
        if fix_frame {
            reframed(&mut bytes, meta.at);
        }
        fs::write(scratch.log(), &bytes).unwrap();
        let readable = readable_versions(&mut reopened(&scratch), &texts, &versions);
        let expected: Vec<bool> = (0..45).map(|ix| ix < 25 || ix == 44).collect();
        assert_eq!(readable, expected, "fix_frame {fix_frame}");

        let pruned = reopened(&scratch).prune(file(45), 15, false).unwrap();
        assert_eq!(pruned.points, 1);
        let readable = readable_versions(&mut reopened(&scratch), &texts, &versions);
        let expected: Vec<bool> = (0..45)
            .map(|ix| (1..25).contains(&ix) || ix == 44)
            .collect();
        assert_eq!(readable, expected, "fix_frame {fix_frame}");
    }
}

#[test]
fn damage_to_the_newest_version_reaches_at_most_twenty_versions_back() {
    let scratch = Scratch::new("compact-chain");
    let texts = edited(45, 20_000);
    let (mut history, versions) = saved_versions(&scratch, &texts);
    history.prune(file(45), 0, false).unwrap();
    let at = reopened(&scratch).refresh().unwrap().blobs[&BlobKey::of(&versions[44])].at;
    let mut bytes = fs::read(scratch.log()).unwrap();
    bytes[at as usize + 60] ^= 0x10;
    fs::write(scratch.log(), &bytes).unwrap();
    let readable = readable_versions(&mut reopened(&scratch), &texts, &versions);
    let expected: Vec<bool> = (0..45).map(|ix| ix < 25).collect();
    assert_eq!(readable, expected);
}

#[test]
fn a_chapter_that_flips_between_two_texts_compacts_without_a_loop() {
    let scratch = Scratch::new("compact-flip");
    let texts = edited(2, 20_000);
    let flips: Vec<String> = (0..12).map(|ix| texts[ix % 2].clone()).collect();
    let (mut history, versions) = saved_versions(&scratch, &flips);
    let before = fs::metadata(scratch.log()).unwrap().len();
    let pruned = history.prune(file(12), 0, false).unwrap();
    assert!(pruned.bytes_after < before, "{pruned:?}");
    let once = fs::read(scratch.log()).unwrap();
    history.prune(file(12), 0, false).unwrap();
    assert_eq!(fs::read(scratch.log()).unwrap(), once);
    assert_eq!(
        readable_versions(&mut reopened(&scratch), &flips, &versions),
        [true; 12]
    );
}

#[test]
fn versions_that_name_each_other_as_base_read_as_damaged_and_cleanup_still_works() {
    let scratch = Scratch::new("compact-cycle");
    let texts = edited(2, 20_000);
    let (one, two) = (
        Packed::of(texts[0].as_bytes()),
        Packed::of(texts[1].as_bytes()),
    );
    let (k1, k2) = (BlobKey::of(&one), BlobKey::of(&two));
    let looped = |key, base, text: &str, other: &str| {
        let body = text::squeeze(text.as_bytes(), Some(other.as_bytes())).unwrap();
        frame(TEXT, &text_payload(key, Some(base), text.as_bytes(), &body))
    };
    let records = [
        point(RecordKind::Save, 10, 0, 1, &[("a.xhtml", Some(&one))]),
        point(RecordKind::Save, 20, 1, 2, &[("a.xhtml", Some(&two))]),
    ];
    let mut bytes = looped(k1, k2, &texts[0], &texts[1]);
    bytes.extend(looped(k2, k1, &texts[1], &texts[0]));
    for record in &records {
        bytes.extend(frame(RECORD, &record.to_bytes()));
    }
    fs::create_dir_all(scratch.log().parent().unwrap()).unwrap();
    fs::write(scratch.log(), &bytes).unwrap();

    let mut history = reopened(&scratch);
    let versions = [one, two];
    assert_eq!(
        readable_versions(&mut history, &texts, &versions),
        [false, false]
    );
    assert_eq!(history.prune(file(2), 15, false).unwrap().points, 1);
    let mut fresh = reopened(&scratch);
    assert_eq!(fresh.records().unwrap(), &records[1..]);
    assert_eq!(
        readable_versions(&mut fresh, &texts, &versions),
        [false, false]
    );
}

#[test]
fn a_rewrite_that_would_not_give_back_every_version_is_refused_and_the_old_log_stays() {
    use crate::commit::seam::install;
    let scratch = Scratch::new("compact-refused");
    let texts = edited(30, 20_000);
    let (mut history, versions) = saved_versions(&scratch, &texts);
    let old_log = fs::read(scratch.log()).unwrap();
    let temp = temp_of(&scratch);
    let key = BlobKey::of(&versions[10]);
    install(move |step| {
        if step == Step::PruneSynced {
            let at = History::at(temp.clone()).refresh()?.blobs[&key].at;
            let mut bytes = fs::read(&temp)?;
            bytes[at as usize + TEXT_META_LEN + 9] ^= 0x10;
            reframed(&mut bytes, at);
            fs::write(&temp, &bytes)?;
        }
        Ok(())
    });
    let result = history.prune(file(30), 0, false);
    install(|_| Ok(()));
    assert!(
        result
            .unwrap_err()
            .user_message()
            .contains("could not be cleaned up safely")
    );
    assert_eq!(fs::read(scratch.log()).unwrap(), old_log);
    assert!(!temp_of(&scratch).exists());
    assert_eq!(
        readable_versions(&mut reopened(&scratch), &texts, &versions),
        [true; 30]
    );
}

#[test]
fn a_text_that_only_matches_the_checksum_and_length_reads_as_damaged() {
    let mut state = 1u64;
    let mut seen: HashMap<u32, Vec<u8>> = HashMap::new();
    let (real, forged) = loop {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        let text = [mixed.to_le_bytes(), (mixed ^ (mixed >> 31)).to_le_bytes()].concat();
        if let Some(other) = seen.insert(crc32fast::hash(&text), text.clone())
            && other != text
        {
            break (other, text);
        }
    };
    assert_eq!(crc32fast::hash(&real), crc32fast::hash(&forged));
    assert_eq!(real.len(), forged.len());

    let version = Packed::of(&real);
    let key = BlobKey::of(&version);
    let record = point(RecordKind::Save, 10, 0, 1, &[("a.xhtml", Some(&version))]);
    for (body_of, readable) in [(&real, true), (&forged, false)] {
        let scratch = Scratch::new("compact-forged");
        let body = text::squeeze(body_of, None).unwrap();
        let mut bytes = frame(TEXT, &text_payload(key, None, &real, &body));
        bytes.extend(frame(RECORD, &record.to_bytes()));
        fs::create_dir_all(scratch.log().parent().unwrap()).unwrap();
        fs::write(scratch.log(), &bytes).unwrap();
        let mut history = reopened(&scratch);
        match history.text(&key) {
            Ok(text) => assert!(readable && text == real),
            Err(error) => {
                assert!(!readable);
                assert_eq!(error.user_message(), damaged().user_message());
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Save(Vec<u8>),
    Named,
    Restore(u8, Vec<u8>),
    FailedHistoryWrite(Vec<u8>),
    NeverLanded(Vec<u8>),
    BackToEarlierFile(u8, Vec<u8>),
}

fn touched() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
    proptest::collection::vec(0u8..4, 0..3)
}

fn op() -> impl proptest::strategy::Strategy<Value = Op> {
    use proptest::prelude::*;
    prop_oneof![
        6 => touched().prop_map(Op::Save),
        2 => Just(Op::Named),
        2 => (any::<u8>(), touched()).prop_map(|(to, entries)| Op::Restore(to, entries)),
        1 => touched().prop_map(Op::FailedHistoryWrite),
        1 => touched().prop_map(Op::NeverLanded),
        1 => (any::<u8>(), touched()).prop_map(|(to, entries)| Op::BackToEarlierFile(to, entries)),
    ]
}

fn numbered(n: u32) -> FileHash {
    let mut hash = [0xa5; 32];
    hash[..4].copy_from_slice(&n.to_le_bytes());
    FileHash(hash)
}

struct Simulated {
    log: Vec<(Vec<Packed>, Record)>,
    current: FileHash,
    revisits: bool,
}

fn simulated_version(next: u32, slot: u8, long: bool) -> Packed {
    let marker = format!("version {next} of c{slot}");
    if !long {
        return packed(&marker);
    }
    let base = prose(u64::from(slot), 20_000);
    let at = (next as usize * 997) % base.len();
    Packed::of(format!("{}{marker}{}", &base[..at], &base[at..]).as_bytes())
}

fn simulate(ops: &[Op], long: bool) -> Simulated {
    let mut versions: HashMap<u8, Packed> = HashMap::new();
    let mut seen = vec![numbered(0)];
    let mut current = numbered(0);
    let mut log = Vec::new();
    let mut revisits = false;
    let mut next = 0u32;
    for (ix, op) in ops.iter().enumerate() {
        let at = 10 * (ix as u64 + 1);
        let mut save = |entries: &[u8], kind: RecordKind, to: Option<FileHash>, lands: bool| {
            let mut blobs = Vec::new();
            let mut changes = Vec::new();
            for slot in entries {
                let name = entry(&format!("c{slot}.xhtml"));
                if changes.iter().any(|change: &Change| change.entry == name) {
                    continue;
                }
                let before = versions.get(slot).cloned();
                changes.push(Change {
                    entry: name,
                    before: before.as_ref().map(BlobKey::of),
                });
                blobs.extend(before);
                next += 1;
                if lands {
                    versions.insert(*slot, simulated_version(next, *slot, long));
                }
            }
            next += 1;
            let after = to.unwrap_or_else(|| numbered(next));
            (
                blobs,
                Record {
                    kind,
                    at,
                    rows: 3,
                    file_before: current,
                    file_after: after,
                    changes,
                    carried: Vec::new(),
                },
            )
        };
        match op {
            Op::Save(entries) => {
                let made = save(entries, RecordKind::Save, None, true);
                current = made.1.file_after;
                log.push(made);
            }
            Op::Named => log.push((
                Vec::new(),
                Record {
                    kind: RecordKind::Named {
                        name: format!("n{ix}"),
                    },
                    at,
                    rows: 3,
                    file_before: current,
                    file_after: current,
                    changes: Vec::new(),
                    carried: Vec::new(),
                },
            )),
            Op::Restore(to, entries) => {
                let kind = match log.len() {
                    0 => RecordKind::Save,
                    count => RecordKind::Restore {
                        to: RecordIx(u32::from(*to) % count as u32),
                    },
                };
                let made = save(entries, kind, None, true);
                current = made.1.file_after;
                log.push(made);
            }
            Op::FailedHistoryWrite(entries) => {
                save(entries, RecordKind::Save, None, true);
                current = numbered(next);
            }
            Op::NeverLanded(entries) => log.push(save(entries, RecordKind::Save, None, false)),
            Op::BackToEarlierFile(to, entries) => {
                let earlier = seen[usize::from(*to) % seen.len()];
                revisits = true;
                let made = save(entries, RecordKind::Save, Some(earlier), true);
                current = earlier;
                log.push(made);
            }
        }
        seen.push(current);
    }
    Simulated {
        log,
        current,
        revisits,
    }
}

fn check_pruned(ops: &[Op], cutoff: u64, long: bool) {
    let simulated = simulate(ops, long);
    let scratch = Scratch::new("prune-property");
    let mut history = written(&scratch, &simulated.log);
    if simulated.log.is_empty() {
        let pruned = history.prune(simulated.current, cutoff, false).unwrap();
        assert_eq!(pruned, Pruned::default());
        return;
    }
    let old_bytes = fs::read(scratch.log()).unwrap();
    let old_copy = scratch.0.join("old-log");
    fs::write(&old_copy, &old_bytes).unwrap();
    let mut old = History::at(old_copy);
    let old_records = old.records().unwrap().to_vec();
    let old_view = landed(&old_records, simulated.current);
    let newest = old_view.points.last().map(|(id, _)| *id);
    let expected: Vec<RecordIx> = old_view
        .points
        .iter()
        .filter(|(id, record)| {
            matches!(record.kind, RecordKind::Named { .. })
                || record.at >= cutoff
                || Some(*id) == newest
        })
        .map(|(id, _)| *id)
        .collect();
    let dropped = old_view
        .points
        .iter()
        .filter(|(id, record)| !expected.contains(id) && record.listed())
        .count() as u32;

    let dry = history.prune(simulated.current, cutoff, true);
    let result = history.prune(simulated.current, cutoff, false);
    let pruned = match result {
        Ok(pruned) => pruned,
        Err(_) => {
            assert!(
                simulated.revisits,
                "only a file that comes back to an earlier state may refuse: {ops:?}"
            );
            assert_eq!(fs::read(scratch.log()).unwrap(), old_bytes);
            assert!(!temp_of(&scratch).exists());
            return;
        }
    };
    let dry = dry.unwrap();
    if long {
        assert_eq!(dry.points, pruned.points, "{ops:?}");
        assert!(pruned.bytes_after <= dry.bytes_after, "{ops:?}");
    } else {
        assert_eq!(dry, pruned, "{ops:?}");
    }
    assert_eq!(pruned.points, dropped, "{ops:?}");
    assert_eq!(pruned.bytes_before, old_bytes.len() as u64);
    assert_eq!(
        pruned.bytes_after,
        fs::metadata(scratch.log()).unwrap().len()
    );

    let mut new = reopened(&scratch);
    let new_records = new.records().unwrap().to_vec();
    let new_view = landed(&new_records, simulated.current);
    assert_eq!(
        new_view.points.len(),
        new_records.len(),
        "every kept point lands: {ops:?}"
    );
    let kept_ids: Vec<PointId> = expected
        .iter()
        .map(|ix| old_records[ix.0 as usize].id())
        .collect();
    let new_ids: Vec<PointId> = new_records.iter().map(Record::id).collect();
    assert_eq!(new_ids, kept_ids, "every kept point keeps its id: {ops:?}");
    for (new_ix, old_id) in expected.iter().enumerate() {
        let then = old_view.state_at(*old_id).unwrap();
        let now = new_view.state_at(RecordIx(new_ix as u32)).unwrap();
        assert_eq!(now, then, "point {old_id:?}: {ops:?}");
        for key in now.known.values().flatten() {
            if long {
                assert_eq!(new.text(key).ok(), old.text(key).ok(), "{ops:?}");
            } else {
                assert_eq!(new.blob(key).ok(), old.blob(key).ok(), "{ops:?}");
            }
        }
        let was = &old_records[old_id.0 as usize].kind;
        let is = &new_records[new_ix].kind;
        match (was, is) {
            (RecordKind::Restore { to: target }, RecordKind::Restore { to }) => {
                let moved = expected.iter().position(|id| id == target);
                assert_eq!(to.0, moved.map_or(u32::MAX, |ix| ix as u32), "{ops:?}");
            }
            _ => assert_eq!(was, is, "{ops:?}"),
        }
    }
    let once = fs::read(scratch.log()).unwrap();
    let again = reopened(&scratch)
        .prune(simulated.current, cutoff, false)
        .unwrap();
    assert_eq!(again.points, 0, "a second prune finds nothing: {ops:?}");
    assert_eq!(again.bytes_after, pruned.bytes_after);
    assert_eq!(fs::read(scratch.log()).unwrap(), once, "{ops:?}");
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(256),
        ..proptest::prelude::ProptestConfig::default()
    })]

    #[test]
    fn every_kept_point_restores_the_same_versions_after_pruning(
        ops in proptest::collection::vec(op(), 1..24),
        cut in 0u64..260,
    ) {
        check_pruned(&ops, cut, false);
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(64),
        ..proptest::prelude::ProptestConfig::default()
    })]

    #[test]
    fn every_kept_point_restores_the_same_text_after_compaction(
        ops in proptest::collection::vec(op(), 1..24),
        cut in 0u64..260,
    ) {
        check_pruned(&ops, cut, true);
    }
}
