use super::{Book, CommitReport, Payload, WriterState, no_chapter_file};
use crate::backup::RevertTarget;
use crate::book::Reader;
use crate::chapter::decode_and_model;
use crate::history::{
    Change, DiffPart, History, PointId, PointKind, Pruned, Record, RecordIx, RecordKind, Staged,
    StateAt, Version, damaged, diff_text, key_of, landed,
};
use crate::library::now_ms;
use crate::manage::unwrap_links;
use crate::storage::remove_backups;
use crate::zip::{
    Changes, EntryFate, EntryName, NewEntry, Packed, RevertKind, RevertRow, Rewritten, ZipEntry,
    directory_diff,
};
use crate::{Error, FileHash, Limits};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io;
use std::thread::JoinHandle;

const MAX_NAME_CHARS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cause {
    Save,
    Restore(RecordIx),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HistoryPoint {
    pub id: PointId,
    pub at: u64,
    pub kind: PointKind,
    pub changed: Vec<EntryName>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PointChanges {
    pub rows: Vec<RevertRow>,
    pub complete: bool,
}

struct Restoring {
    targets: HashMap<EntryName, Option<NewEntry>>,
    spine_delta: i64,
}

impl Changes for Restoring {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error> {
        Ok(match self.targets.remove(&entry.name) {
            Some(Some(new)) => EntryFate::Write(new),
            Some(None) => EntryFate::Drop,
            None => EntryFate::RawCopy,
        })
    }

    fn added(&mut self) -> Vec<NewEntry> {
        let mut added: Vec<NewEntry> = self.targets.drain().filter_map(|(_, new)| new).collect();
        added.sort_by(|a, b| a.name().cmp(b.name()));
        added
    }

    fn spine_delta(&self) -> i64 {
        self.spine_delta
    }
}

fn same_as(entry: Option<&ZipEntry>, packed_crc: u32, packed_inflated: u64) -> bool {
    entry.is_some_and(|entry| entry.crc32 == packed_crc && entry.inflated == packed_inflated)
}

pub(super) fn diffed_names(head: &Reader<File>, new: &Reader<File>) -> Vec<EntryName> {
    directory_diff(new.zip(), head.zip())
        .into_iter()
        .map(|row| row.entry)
        .collect()
}

pub(super) fn rewritten_names(head: &Reader<File>, rewritten: &Rewritten) -> Vec<EntryName> {
    let mut kept = vec![false; head.zip().entries().len()];
    for copy in &rewritten.copied {
        kept[copy.source_ix as usize] = true;
    }
    let written: BTreeSet<&EntryName> = rewritten
        .rewritten
        .iter()
        .map(|entry| &entry.name)
        .collect();
    let mut names: Vec<EntryName> = rewritten
        .rewritten
        .iter()
        .filter(|entry| !same_as(head.zip().entry(&entry.name), entry.crc32, entry.inflated))
        .map(|entry| entry.name.clone())
        .collect();
    names.extend(
        head.zip()
            .entries()
            .iter()
            .zip(kept)
            .filter(|(entry, kept)| !kept && !written.contains(&entry.name))
            .map(|(entry, _)| entry.name.clone()),
    );
    names
}

fn detail(error: Error) -> String {
    match error {
        Error::Io(error) => crate::plain_io(&error).to_owned(),
        other => other.user_message(),
    }
}

pub(super) struct Syncing(JoinHandle<(Staged, io::Result<()>)>);

impl Syncing {
    pub(super) fn spawn(staged: Staged) -> Syncing {
        Syncing(std::thread::spawn(move || {
            let synced = staged.sync();
            (staged, synced)
        }))
    }

    pub(super) fn finish(self) -> Result<(), String> {
        self.0
            .join()
            .map_err(|_| io::Error::other("the history sync stopped"))
            .and_then(|(staged, synced)| staged.finish(synced))
            .map_err(|error| crate::plain_io(&error).to_owned())
    }
}

pub(super) fn start(
    history: &mut History,
    head: &Reader<File>,
    names: Vec<EntryName>,
    limits: &Limits,
    record: impl FnOnce(Vec<Change>) -> Record,
) -> Result<Staged, String> {
    let mut blobs = Vec::new();
    let mut changes = Vec::new();
    for name in names {
        let before = match head.zip().entry(&name) {
            Some(entry) => {
                let packed = head
                    .zip()
                    .read_packed(head.source(), entry, limits)
                    .map_err(detail)?;
                let key = key_of(&packed);
                blobs.push(packed);
                Some(key)
            }
            None => None,
        };
        changes.push(Change {
            entry: name,
            before,
        });
    }
    history
        .stage(blobs, record(changes))
        .map_err(|error| error.to_string())
}

impl Cause {
    pub(super) fn record(
        self,
        rows: usize,
        file_before: FileHash,
        file_after: FileHash,
        changes: Vec<Change>,
    ) -> Record {
        Record {
            kind: match self {
                Cause::Save => RecordKind::Save,
                Cause::Restore(to) => RecordKind::Restore { to },
            },
            at: now_ms(),
            rows: rows as u32,
            file_before,
            file_after,
            changes,
            carried: Vec::new(),
        }
    }
}

fn not_in_history() -> Error {
    Error::NotEditable("That point is no longer in this book's history.".to_owned())
}

fn text_of(bytes: Vec<u8>, limits: &Limits) -> Result<String, Error> {
    Ok(decode_and_model(bytes, limits)?.text.as_str().to_owned())
}

impl Book {
    fn state_at(&self, state: &mut WriterState, id: PointId) -> Result<(RecordIx, StateAt), Error> {
        let current = self.file_hash(state)?;
        let records = state.history.records()?;
        let view = landed(records, current);
        view.find(id)
            .and_then(|ix| Some((ix, view.state_at(ix)?)))
            .ok_or_else(not_in_history)
    }

    pub fn history(&self) -> Result<Vec<HistoryPoint>, Error> {
        let mut state = self.lock_state();
        let current = self.file_hash(&mut state)?;
        let records = state.history.records()?;
        Ok(landed(records, current)
            .points
            .iter()
            .rev()
            .filter(|(_, record)| record.listed())
            .map(|(_, record)| HistoryPoint {
                id: record.id(),
                at: record.at,
                kind: match &record.kind {
                    RecordKind::Save => PointKind::Save,
                    RecordKind::Restore { to } => PointKind::Restore {
                        to: records.get(to.0 as usize).map(Record::id),
                    },
                    RecordKind::Named { name } => PointKind::Named { name: name.clone() },
                },
                changed: record
                    .changes
                    .iter()
                    .map(|change| change.entry.clone())
                    .collect(),
            })
            .collect())
    }

    pub fn take_snapshot(&self, name: &str) -> Result<PointId, Error> {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        if name.is_empty() {
            return Err(Error::NotEditable("Give the snapshot a name.".to_owned()));
        }
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        self.check_unchanged(&mut state)?;
        let current = self.file_hash(&mut state)?;
        let record = Record {
            kind: RecordKind::Named { name },
            at: now_ms(),
            rows: self.read().rows().len() as u32,
            file_before: current,
            file_after: current,
            changes: Vec::new(),
            carried: Vec::new(),
        };
        let id = record.id();
        state.history.append(Vec::new(), record)?;
        Ok(id)
    }

    pub fn history_changes(&self, id: PointId) -> Result<PointChanges, Error> {
        let mut state = self.lock_state();
        let (_, at) = self.state_at(&mut state, id)?;
        let head = self.read();
        let mut rows = Vec::new();
        for (entry, before) in &at.known {
            let now = head.zip().entry(entry);
            let kind = match (before, now) {
                (None, Some(_)) => RevertKind::Added,
                (Some(_), None) => RevertKind::Removed,
                (Some(key), Some(_)) => {
                    let meta = state.history.blob_meta(key).ok_or_else(damaged)?;
                    if same_as(now, meta.crc32, meta.inflated) {
                        continue;
                    }
                    RevertKind::Changed
                }
                (None, None) => continue,
            };
            rows.push(RevertRow {
                entry: entry.clone(),
                kind,
            });
        }
        rows.sort_by(|a, b| a.entry.cmp(&b.entry));
        Ok(PointChanges {
            rows,
            complete: at.complete,
        })
    }

    pub fn history_diff(&self, id: PointId, entry: &EntryName) -> Result<Vec<DiffPart>, Error> {
        let mut state = self.lock_state();
        let (_, at) = self.state_at(&mut state, id)?;
        let head = self.read();
        let now = match head.zip().entry(entry) {
            Some(found) => text_of(
                head.zip().read_entry(head.source(), found, &self.limits)?,
                &self.limits,
            )?,
            None => String::new(),
        };
        let then = match at.version(entry) {
            Version::Now => now.clone(),
            Version::Then(None) => String::new(),
            Version::Then(Some(key)) => text_of(
                state.history.blob(&key)?.unpack(entry.as_str())?,
                &self.limits,
            )?,
            Version::Unknown => return Err(gap_refusal()),
        };
        Ok(diff_text(&then, &now))
    }

    pub fn restore_point(&self, id: PointId, target: RevertTarget) -> Result<CommitReport, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        let head = self.read();
        self.check_unchanged(&mut state)?;
        let (ix, at) = self.state_at(&mut state, id)?;
        let mut changes = match target {
            RevertTarget::WholeBook => {
                if !at.complete {
                    return Err(Error::NotEditable(
                        "Some saves after this point were not recorded, so the whole book cannot be put back exactly. Single chapters can still be restored."
                            .to_owned(),
                    ));
                }
                let mut targets = HashMap::new();
                for (entry, before) in &at.known {
                    let now = head.zip().entry(entry);
                    match before {
                        None if now.is_some() => {
                            targets.insert(entry.clone(), None);
                        }
                        None => {}
                        Some(key) => {
                            let packed = state.history.blob(key)?;
                            if !same_as(now, packed.crc32, packed.inflated) {
                                targets.insert(
                                    entry.clone(),
                                    Some(NewEntry::from_packed(entry.clone(), packed)),
                                );
                            }
                        }
                    }
                }
                Restoring {
                    targets,
                    spine_delta: i64::from(at.rows) - head.rows().len() as i64,
                }
            }
            RevertTarget::Chapter(name) => {
                let is_package = name == *head.opf_entry()
                    || head.nav_name() == Some(&name)
                    || head.ncx_name() == Some(&name);
                if is_package {
                    return Err(Error::NotEditable(
                        "The contents change with the book, so only the whole book can be restored."
                            .to_owned(),
                    ));
                }
                let key = match at.version(&name) {
                    Version::Now => None,
                    Version::Then(Some(key)) => Some(key),
                    Version::Then(None) => {
                        return Err(Error::NotEditable(
                            "This chapter did not exist at that point, so only the whole book can be restored."
                                .to_owned(),
                        ));
                    }
                    Version::Unknown => return Err(gap_refusal()),
                };
                let Some(now) = head.zip().entry(&name) else {
                    return Err(match key {
                        Some(_) => Error::NotEditable(
                            "This chapter was removed since that point, so only the whole book can be restored."
                                .to_owned(),
                        ),
                        None => no_chapter_file(),
                    });
                };
                let Some(key) = key else {
                    return Ok(CommitReport::nothing_to_do());
                };
                let packed = state.history.blob(&key)?;
                if same_as(Some(now), packed.crc32, packed.inflated) {
                    return Ok(CommitReport::nothing_to_do());
                }
                let entry = self.unwrapped(&head, &at, &name, packed)?;
                Restoring {
                    targets: HashMap::from([(name, Some(entry))]),
                    spine_delta: 0,
                }
            }
        };
        if changes.targets.is_empty() {
            return Ok(CommitReport::nothing_to_do());
        }
        self.commit(
            &mut state,
            &head,
            Payload::Rewrite(&mut changes),
            None,
            Cause::Restore(ix),
        )
    }

    pub fn prune_history(&self, cutoff_ms: u64, dry_run: bool) -> Result<Pruned, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        let current = self.file_hash(&mut state)?;
        state.history.prune(current, cutoff_ms, dry_run)
    }

    pub fn delete_backups(&self) -> Result<u64, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        state.slot = None;
        remove_backups(&self.data)
    }

    fn unwrapped(
        &self,
        head: &Reader<File>,
        at: &StateAt,
        name: &EntryName,
        packed: Packed,
    ) -> Result<NewEntry, Error> {
        let gone: BTreeSet<EntryName> = at
            .known
            .iter()
            .filter(|(entry, before)| before.is_some() && head.zip().entry(entry).is_none())
            .map(|(entry, _)| entry.clone())
            .collect();
        if gone.is_empty() {
            return Ok(NewEntry::from_packed(name.clone(), packed));
        }
        let bytes = packed.clone().unpack(name.as_str())?;
        Ok(match unwrap_links(&bytes, name, &gone, &self.limits)? {
            Some(unwrapped) => NewEntry::deflate(name.clone(), &unwrapped.bytes),
            None => NewEntry::from_packed(name.clone(), packed),
        })
    }
}

fn gap_refusal() -> Error {
    Error::NotEditable(
        "Some saves after this point were not recorded, so this chapter's version from then is not known."
            .to_owned(),
    )
}
