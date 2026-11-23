use super::{
    BlobKey, Form, HEAD_LEN, History, Index, RECORD, Record, RecordIx, RecordKind, TEXT,
    TEXT_META_LEN, frame, landed, text, text_payload,
};
use crate::commit::seam::{Step, hit, sync_dir};
use crate::{EntryName, Error, FileHash};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

const PRUNE_TEMP: &str = "log.new";
const CHAIN: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pruned {
    pub points: u32,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

struct Plan {
    records: Vec<Record>,
    kept: Vec<RecordIx>,
    dropped: u32,
    texts: HashMap<BlobKey, Vec<u8>>,
    lost: HashSet<BlobKey>,
}

struct Link<'a> {
    key: BlobKey,
    base: Option<BlobKey>,
    entry: &'a EntryName,
}

#[derive(Default)]
struct Newest<'a>(HashMap<&'a EntryName, (BlobKey, Vec<u8>)>);

impl<'a> Newest<'a> {
    fn read(&self, history: &mut History, link: &Link) -> Option<Vec<u8>> {
        let newer = self
            .0
            .get(link.entry)
            .map(|(key, text)| (key, text.as_slice()));
        history.text_from(&link.key, newer).ok()
    }

    fn of(&self, link: &Link, key: &BlobKey) -> Option<&[u8]> {
        let (newest, text) = self.0.get(link.entry)?;
        (newest == key).then_some(text.as_slice())
    }

    fn keep(&mut self, link: &Link<'a>, text: Vec<u8>) {
        self.0.insert(link.entry, (link.key, text));
    }
}

fn links(records: &[Record]) -> Vec<Link<'_>> {
    let mut newer: HashMap<&EntryName, Option<BlobKey>> = HashMap::new();
    let mut linked = HashSet::new();
    let mut links = Vec::new();
    for record in records.iter().rev() {
        let mut entries = HashSet::new();
        for change in record.changes.iter().chain(&record.carried) {
            if let Some(key) = change.before
                && linked.insert(key)
            {
                links.push(Link {
                    key,
                    base: newer.get(&change.entry).copied().flatten(),
                    entry: &change.entry,
                });
            }
            if entries.insert(&change.entry) {
                newer.insert(&change.entry, change.before);
            }
        }
    }
    links
}

fn limit_chains(links: &mut [Link]) {
    let mut depth: HashMap<BlobKey, usize> = HashMap::new();
    for link in links.iter_mut() {
        let below = link
            .base
            .and_then(|base| depth.get(&base).copied())
            .filter(|steps| steps + 1 < CHAIN);
        if below.is_none() {
            link.base = None;
        }
        depth.insert(link.key, below.map_or(0, |steps| steps + 1));
    }
}

fn cited(record: &Record) -> impl Iterator<Item = BlobKey> + '_ {
    record
        .changes
        .iter()
        .chain(&record.carried)
        .filter_map(|change| change.before)
}

fn fold(into: &mut Record, seen: &mut HashSet<EntryName>, from: &Record) {
    for change in from.changes.iter().chain(&from.carried) {
        if seen.insert(change.entry.clone()) {
            into.carried.push(change.clone());
        }
    }
    into.file_after = from.file_after;
}

fn plan(records: &[Record], current: FileHash, cutoff_ms: u64) -> Plan {
    let view = landed(records, current);
    let newest = view.points.len().saturating_sub(1);
    let mut plan = Plan {
        records: Vec::new(),
        kept: Vec::new(),
        dropped: 0,
        texts: HashMap::new(),
        lost: HashSet::new(),
    };
    let mut seen = HashSet::new();
    let mut folding = false;
    for (ix, (id, record)) in view.points.iter().enumerate() {
        let keep = matches!(record.kind, RecordKind::Named { .. })
            || record.at >= cutoff_ms
            || ix == newest;
        if keep {
            seen = record
                .changes
                .iter()
                .chain(&record.carried)
                .map(|change| change.entry.clone())
                .collect();
            plan.records.push((*record).clone());
            plan.kept.push(*id);
            folding = true;
            continue;
        }
        if record.listed() {
            plan.dropped += 1;
        }
        folding &= !view.gap_before[ix];
        if folding && let Some(into) = plan.records.last_mut() {
            fold(into, &mut seen, record);
        }
    }
    let renumbered: HashMap<RecordIx, RecordIx> = plan
        .kept
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, RecordIx(new as u32)))
        .collect();
    for record in &mut plan.records {
        if let RecordKind::Restore { to } = &mut record.kind {
            *to = renumbered.get(to).copied().unwrap_or(RecordIx(u32::MAX));
        }
    }
    plan
}

impl Plan {
    fn compact(&mut self, history: &mut History) {
        let mut links = links(&self.records);
        limit_chains(&mut links);
        let mut placed = HashSet::new();
        let mut newest = Newest::default();
        for link in &links {
            let Some(meta) = history.blob_meta(&link.key) else {
                continue;
            };
            let Some(text) = newest.read(history, link) else {
                if meta.form != Form::Packed {
                    self.lost.insert(link.key);
                }
                continue;
            };
            let base = link.base.filter(|base| placed.contains(base));
            if meta.form.base() != base {
                let base_text = base.and_then(|base| match newest.of(link, &base) {
                    Some(text) => Some(Cow::Borrowed(text)),
                    None => history.text(&base).ok().map(Cow::Owned),
                });
                let delta = base.zip(base_text).and_then(|(base, base_text)| {
                    Some((base, text::squeeze(&text, Some(&base_text))?))
                });
                let full = match meta.form {
                    Form::Text(Some(_)) => text::squeeze(&text, None),
                    _ => None,
                };
                let full_len = full
                    .as_ref()
                    .map_or(meta.len as usize, |body| TEXT_META_LEN + body.len());
                let payload = match (delta, full) {
                    (Some((base, body)), _)
                        if (TEXT_META_LEN + body.len()) * 10 <= full_len * 9 =>
                    {
                        Some(text_payload(link.key, Some(base), &text, &body))
                    }
                    (_, Some(body)) => Some(text_payload(link.key, None, &text, &body)),
                    _ => None,
                };
                match payload {
                    Some(payload) => {
                        self.texts.insert(link.key, payload);
                    }
                    None if meta.form.base().is_some() => {
                        newest.keep(link, text);
                        continue;
                    }
                    None => {}
                }
            }
            placed.insert(link.key);
            newest.keep(link, text);
        }
    }

    fn size(&self, index: &Index) -> u64 {
        let mut counted = HashSet::new();
        let mut size = 0;
        for record in &self.records {
            for key in cited(record) {
                if let Some(meta) = index.blobs.get(&key).and_then(|copies| copies.first())
                    && counted.insert(key)
                {
                    size += HEAD_LEN + meta.len;
                }
            }
            size += HEAD_LEN + record.to_bytes().len() as u64;
        }
        size
    }

    fn write(&self, history: &mut History, temp: &Path) -> io::Result<HashSet<BlobKey>> {
        let mut out = BufWriter::new(File::create(temp)?);
        let mut written = HashSet::new();
        for (ix, record) in self.records.iter().enumerate() {
            for key in cited(record) {
                if written.contains(&key) || self.lost.contains(&key) {
                    continue;
                }
                let copied = match self.texts.get(&key) {
                    Some(payload) => Some(frame(TEXT, payload)),
                    None => history.copy_of(&key),
                };
                if let Some(bytes) = copied {
                    out.write_all(&bytes)?;
                    written.insert(key);
                }
            }
            out.write_all(&frame(RECORD, &record.to_bytes()))?;
            if ix == self.records.len() / 2 {
                out.flush()?;
                hit(Step::PruneWriting)?;
            }
        }
        let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
        hit(Step::PruneWritten)?;
        file.sync_data()?;
        hit(Step::PruneSynced)?;
        Ok(written)
    }

    fn verify(
        &self,
        history: &mut History,
        old: &[Record],
        current: FileHash,
        written: &HashSet<BlobKey>,
        temp: &Path,
    ) -> Result<(), Error> {
        let mut new = History::at(temp.to_owned());
        let records = new.records().map_err(prune_failed)?.to_vec();
        let old_view = landed(old, current);
        let new_view = landed(&records, current);
        let landed_ids: Vec<RecordIx> = new_view.points.iter().map(|(id, _)| *id).collect();
        let every_kept: Vec<RecordIx> = (0..self.kept.len() as u32).map(RecordIx).collect();
        let same = records == self.records
            && landed_ids == every_kept
            && old_view.points.last().map(|(id, _)| id) == self.kept.last()
            && self.kept.iter().enumerate().all(|(new_ix, old_id)| {
                let new_id = RecordIx(new_ix as u32);
                let (was, now) = (&old[old_id.0 as usize], &records[new_ix]);
                let kind_kept = match (&was.kind, &now.kind) {
                    (RecordKind::Restore { to: before }, RecordKind::Restore { to: after }) => {
                        let target = self.kept.iter().position(|id| id == before);
                        *after == RecordIx(target.map_or(u32::MAX, |ix| ix as u32))
                    }
                    (before, after) => before == after,
                };
                kind_kept
                    && was.id() == now.id()
                    && old_view.state_at(*old_id) == new_view.state_at(new_id)
            });
        if !same {
            return Err(not_safe());
        }
        let mut newest = Newest::default();
        for link in &links(&records) {
            let was = newest.read(history, link);
            let now = newest.read(&mut new, link);
            let unreadable =
                now.is_none() && written.contains(&link.key) && !new.readable(&link.key);
            if was != now || unreadable {
                return Err(not_safe());
            }
            if let Some(text) = now {
                newest.keep(link, text);
            }
        }
        Ok(())
    }
}

fn not_safe() -> Error {
    Error::Book(
        "This book's history could not be cleaned up safely, so it was left as it was.".to_owned(),
    )
}

fn prune_failed(error: io::Error) -> Error {
    Error::Book(format!(
        "This book's history could not be cleaned up, so it was left as it was. {}",
        crate::plain_io(&error)
    ))
}

impl History {
    fn temp(&self) -> PathBuf {
        self.dir().join(PRUNE_TEMP)
    }

    fn replace_with(
        &mut self,
        plan: &Plan,
        old: &[Record],
        current: FileHash,
        temp: &Path,
    ) -> Result<u64, Error> {
        let written = plan.write(self, temp).map_err(prune_failed)?;
        plan.verify(self, old, current, &written, temp)?;
        let bytes_after = fs::metadata(temp).map_err(prune_failed)?.len();
        fs::rename(temp, &self.log).map_err(prune_failed)?;
        self.trusted.extend(written);
        Ok(bytes_after)
    }

    pub(crate) fn prune(
        &mut self,
        current: FileHash,
        cutoff_ms: u64,
        dry_run: bool,
    ) -> Result<Pruned, Error> {
        if !dry_run {
            match fs::remove_file(self.temp()) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    return Err(prune_failed(error));
                }
                _ => {}
            }
        }
        let bytes_before = match fs::metadata(&self.log) {
            Ok(meta) => meta.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Pruned::default());
            }
            Err(error) => return Err(prune_failed(error)),
        };
        let old = self.records().map_err(prune_failed)?.to_vec();
        let mut plan = plan(&old, current, cutoff_ms);
        if !dry_run {
            plan.compact(self);
        }
        let unchanged = Pruned {
            points: 0,
            bytes_before,
            bytes_after: bytes_before,
        };
        if plan.records.len() == old.len() && plan.texts.is_empty() {
            return Ok(unchanged);
        }
        if dry_run {
            let index = self.refresh().map_err(prune_failed)?;
            return Ok(Pruned {
                points: plan.dropped,
                bytes_after: plan.size(index),
                ..unchanged
            });
        }
        let temp = self.temp();
        let bytes_after = match self.replace_with(&plan, &old, current, &temp) {
            Ok(bytes_after) => bytes_after,
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(error);
            }
        };
        self.index = None;
        let _ = hit(Step::PruneRenamed)
            .and_then(|()| sync_dir(self.dir()))
            .and_then(|()| hit(Step::PruneDirSynced));
        Ok(Pruned {
            points: plan.dropped,
            bytes_after,
            ..unchanged
        })
    }
}
