mod diff;
mod prune;
mod text;

pub(crate) use diff::diff_text;
pub use diff::{DiffKind, DiffPart};
pub use prune::Pruned;

use crate::backup::{hex, hex_of, parse_hex};
use crate::commit::seam::{Step, hit, sync_dir};
use crate::zip::{EntryName, Packed, ReadAt};
use crate::{Error, FileHash};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const LOG_FILE: &str = "log";
const MAGIC: &[u8; 4] = b"JSH1";
const HEAD_LEN: u64 = 13;
const BLOB: u8 = 1;
const RECORD: u8 = 2;
const TEXT: u8 = 3;
const KEY_LEN: usize = 32;
const SUM_LEN: usize = 32;
const BLOB_META_LEN: usize = KEY_LEN + 2 + 4 + 8;
const TEXT_META_LEN: usize = KEY_LEN + KEY_LEN + 4 + 8 + SUM_LEN;
const MAX_CHAIN: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BlobKey([u8; 32]);

impl BlobKey {
    fn of(packed: &Packed) -> BlobKey {
        let mut hasher = Sha256::new();
        hasher.update(packed.method.to_le_bytes());
        hasher.update(packed.crc32.to_le_bytes());
        hasher.update(packed.inflated.to_le_bytes());
        hasher.update(&packed.body);
        BlobKey(hasher.finalize().into())
    }

    fn text(&self) -> String {
        hex(&FileHash(self.0))
    }

    fn parse(text: &str) -> Option<BlobKey> {
        parse_hex(text).map(|hash| BlobKey(hash.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RecordIx(pub(crate) u32);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum RecordKind {
    Save,
    Restore { to: RecordIx },
    Named { name: String },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PointId([u8; 32]);

impl Serialize for PointId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex_of(&self.0))
    }
}

impl<'de> Deserialize<'de> for PointId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PointId, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_hex(&text)
            .map(|hash| PointId(hash.0))
            .ok_or_else(|| D::Error::custom("not a history point"))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PointKind {
    Save,
    Restore { to: Option<PointId> },
    Named { name: String },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Packed,
    Text(Option<BlobKey>),
}

impl Form {
    fn base(self) -> Option<BlobKey> {
        match self {
            Form::Packed => None,
            Form::Text(base) => base,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BlobMeta {
    at: u64,
    len: u64,
    pub(crate) crc32: u32,
    pub(crate) inflated: u64,
    form: Form,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Change {
    pub(crate) entry: EntryName,
    pub(crate) before: Option<BlobKey>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Record {
    pub(crate) kind: RecordKind,
    pub(crate) at: u64,
    pub(crate) rows: u32,
    pub(crate) file_before: FileHash,
    pub(crate) file_after: FileHash,
    pub(crate) changes: Vec<Change>,
    pub(crate) carried: Vec<Change>,
}

#[derive(Serialize, Deserialize)]
struct StoredChange {
    entry: String,
    before: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRecord {
    #[serde(flatten)]
    kind: RecordKind,
    at: u64,
    rows: u32,
    file_before: String,
    file_after: String,
    changes: Vec<StoredChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    carried: Vec<StoredChange>,
}

fn stored(changes: &[Change]) -> Vec<StoredChange> {
    changes
        .iter()
        .map(|change| StoredChange {
            entry: change.entry.as_str().to_owned(),
            before: change.before.as_ref().map(BlobKey::text),
        })
        .collect()
}

fn parsed(changes: Vec<StoredChange>) -> Option<Vec<Change>> {
    changes
        .into_iter()
        .map(|change| {
            Some(Change {
                entry: EntryName::parse(&change.entry).ok()?,
                before: match change.before {
                    Some(text) => Some(BlobKey::parse(&text)?),
                    None => None,
                },
            })
        })
        .collect()
}

impl Record {
    fn to_bytes(&self) -> Vec<u8> {
        let stored = StoredRecord {
            kind: self.kind.clone(),
            at: self.at,
            rows: self.rows,
            file_before: hex(&self.file_before),
            file_after: hex(&self.file_after),
            changes: stored(&self.changes),
            carried: stored(&self.carried),
        };
        serde_json::to_vec(&stored).unwrap_or_default()
    }

    fn parse(bytes: &[u8]) -> Option<Record> {
        let stored: StoredRecord = serde_json::from_slice(bytes).ok()?;
        Some(Record {
            kind: stored.kind,
            at: stored.at,
            rows: stored.rows,
            file_before: parse_hex(&stored.file_before)?,
            file_after: parse_hex(&stored.file_after)?,
            changes: parsed(stored.changes)?,
            carried: parsed(stored.carried)?,
        })
    }

    pub(crate) fn listed(&self) -> bool {
        self.kind != RecordKind::Save || !self.changes.is_empty()
    }

    pub(crate) fn id(&self) -> PointId {
        let (tag, name) = match &self.kind {
            RecordKind::Save => (0, ""),
            RecordKind::Restore { .. } => (1, ""),
            RecordKind::Named { name } => (2, name.as_str()),
        };
        let mut hasher = Sha256::new();
        hasher.update([tag]);
        hasher.update(self.at.to_le_bytes());
        hasher.update(self.file_before.0);
        let entries = self.changes.iter().map(|change| change.entry.as_str());
        for text in std::iter::once(name).chain(entries) {
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text);
        }
        PointId(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileId(u64, u64, Option<SystemTime>);

impl FileId {
    #[cfg(unix)]
    fn of(meta: &Metadata) -> FileId {
        use std::os::unix::fs::MetadataExt;
        FileId(meta.dev(), meta.ino(), meta.created().ok())
    }

    #[cfg(not(unix))]
    fn of(meta: &Metadata) -> FileId {
        FileId(0, 0, meta.created().ok())
    }
}

#[derive(Default)]
struct Index {
    file: Option<FileId>,
    end: u64,
    blobs: HashMap<BlobKey, BlobMeta>,
    records: Vec<Record>,
}

pub(crate) struct History {
    log: PathBuf,
    index: Option<Index>,
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEAD_LEN as usize + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default())
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

fn meta_len(kind: u8) -> Option<usize> {
    match kind {
        RECORD => Some(0),
        BLOB => Some(BLOB_META_LEN),
        TEXT => Some(TEXT_META_LEN),
        _ => None,
    }
}

fn read_frame(file: &File, len: u64, at: u64) -> io::Result<Option<(u8, Vec<u8>)>> {
    if at + HEAD_LEN > len {
        return Ok(None);
    }
    let mut head = [0u8; HEAD_LEN as usize];
    file.read_exact_at(&mut head, at)?;
    let size = u64::from(le32(&head, 5));
    let kind = head[4];
    let known = meta_len(kind).is_some_and(|least| size >= least as u64);
    if &head[..4] != MAGIC || !known || at + HEAD_LEN + size > len {
        return Ok(None);
    }
    let mut payload = vec![0u8; size as usize];
    file.read_exact_at(&mut payload, at + HEAD_LEN)?;
    if crc32fast::hash(&payload) != le32(&head, 9) {
        return Ok(None);
    }
    Ok(Some((kind, payload)))
}

fn next_frame(file: &File, len: u64, from: u64) -> io::Result<Option<u64>> {
    let mut chunk = vec![0u8; 1 << 16];
    let mut at = from;
    while at + HEAD_LEN <= len {
        let read = chunk.len().min((len - at) as usize);
        file.read_exact_at(&mut chunk[..read], at)?;
        for offset in memchr::memmem::find_iter(&chunk[..read], MAGIC) {
            let candidate = at + offset as u64;
            if read_frame(file, len, candidate)?.is_some() {
                return Ok(Some(candidate));
            }
        }
        at += (read - (MAGIC.len() - 1)) as u64;
    }
    Ok(None)
}

fn scan_from(file: &File, len: u64, index: &mut Index) -> io::Result<()> {
    let mut at = index.end;
    while at + HEAD_LEN <= len {
        let Some((kind, payload)) = read_frame(file, len, at)? else {
            match next_frame(file, len, at + 1)? {
                Some(next) => {
                    at = next;
                    continue;
                }
                None => return Ok(()),
            }
        };
        let body = at + HEAD_LEN;
        if kind == RECORD {
            if let Some(record) = Record::parse(&payload) {
                index.records.push(record);
            }
        } else {
            let (form, sizes_at) = match kind {
                TEXT => (Form::Text(base_at(&payload)), 2 * KEY_LEN),
                _ => (Form::Packed, KEY_LEN + 2),
            };
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&payload[..KEY_LEN]);
            index.blobs.entry(BlobKey(key)).or_insert(BlobMeta {
                at: body,
                len: payload.len() as u64,
                crc32: le32(&payload, sizes_at),
                inflated: le64(&payload, sizes_at + 4),
                form,
            });
        }
        at = body + payload.len() as u64;
        index.end = at;
    }
    Ok(())
}

fn base_at(payload: &[u8]) -> Option<BlobKey> {
    let base: [u8; KEY_LEN] = payload[KEY_LEN..2 * KEY_LEN].try_into().ok()?;
    (base != [0; KEY_LEN]).then_some(BlobKey(base))
}

fn text_payload(key: BlobKey, base: Option<BlobKey>, text: &[u8], body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(TEXT_META_LEN + body.len());
    payload.extend_from_slice(&key.0);
    payload.extend_from_slice(&base.map_or([0; KEY_LEN], |base| base.0));
    payload.extend_from_slice(&crc32fast::hash(text).to_le_bytes());
    payload.extend_from_slice(&(text.len() as u64).to_le_bytes());
    payload.extend_from_slice(&Sha256::digest(text));
    payload.extend_from_slice(body);
    payload
}

fn checked_payload(file: &File, meta: &BlobMeta, key: &BlobKey) -> Option<Vec<u8>> {
    let mut head = [0u8; HEAD_LEN as usize];
    file.read_exact_at(&mut head, meta.at.checked_sub(HEAD_LEN)?)
        .ok()?;
    let mut payload = vec![0u8; meta.len as usize];
    file.read_exact_at(&mut payload, meta.at).ok()?;
    (crc32fast::hash(&payload) == le32(&head, 9) && payload[..KEY_LEN] == key.0).then_some(payload)
}

fn packed_from(payload: &[u8], meta: &BlobMeta, key: &BlobKey) -> Option<Packed> {
    let packed = Packed {
        method: u16::from_le_bytes([payload[KEY_LEN], payload[KEY_LEN + 1]]),
        crc32: meta.crc32,
        inflated: meta.inflated,
        body: payload[BLOB_META_LEN..].to_vec(),
    };
    (BlobKey::of(&packed) == *key).then_some(packed)
}

fn rebuilt(payload: &[u8], meta: &BlobMeta, base: Option<&[u8]>) -> Option<Vec<u8>> {
    let text = text::expand(&payload[TEXT_META_LEN..], base, meta.inflated)?;
    let sum = Sha256::digest(&text);
    let same = crc32fast::hash(&text) == meta.crc32
        && sum[..] == payload[TEXT_META_LEN - SUM_LEN..TEXT_META_LEN];
    same.then_some(text)
}

enum Stored {
    Packed(Packed),
    Text(Vec<u8>),
}

fn text_within(
    file: &File,
    index: &Index,
    key: &BlobKey,
    newer: Option<(&BlobKey, &[u8])>,
) -> Option<Stored> {
    let mut chain = vec![(*key, *index.blobs.get(key)?)];
    let mut known = None;
    while let Some(base) = chain.last().and_then(|(_, meta)| meta.form.base()) {
        if let Some((newer_key, newer_text)) = newer
            && *newer_key == base
        {
            known = Some(newer_text.to_vec());
            break;
        }
        if chain.len() > MAX_CHAIN {
            return None;
        }
        chain.push((base, *index.blobs.get(&base)?));
    }
    let mut text = match known {
        Some(text) => text,
        None => {
            let (key, meta) = chain.pop()?;
            let payload = checked_payload(file, &meta, &key)?;
            match meta.form {
                Form::Packed => {
                    let packed = packed_from(&payload, &meta, &key)?;
                    if chain.is_empty() {
                        return Some(Stored::Packed(packed));
                    }
                    packed.unpack("").ok()?
                }
                Form::Text(_) => rebuilt(&payload, &meta, None)?,
            }
        }
    };
    for (key, meta) in chain.iter().rev() {
        text = rebuilt(&checked_payload(file, meta, key)?, meta, Some(&text))?;
    }
    Some(Stored::Text(text))
}

fn blob_payload(key: BlobKey, packed: &Packed) -> Vec<u8> {
    let mut payload = Vec::with_capacity(BLOB_META_LEN + packed.body.len());
    payload.extend_from_slice(&key.0);
    payload.extend_from_slice(&packed.method.to_le_bytes());
    payload.extend_from_slice(&packed.crc32.to_le_bytes());
    payload.extend_from_slice(&packed.inflated.to_le_bytes());
    payload.extend_from_slice(&packed.body);
    payload
}

pub(crate) fn key_of(packed: &Packed) -> BlobKey {
    BlobKey::of(packed)
}

pub(crate) fn damaged() -> Error {
    Error::Book("This saved version is damaged, so it cannot be used.".to_owned())
}

impl History {
    pub(crate) fn new(book_root: &Path) -> History {
        History::at(book_root.join("history").join(LOG_FILE))
    }

    fn at(log: PathBuf) -> History {
        History { log, index: None }
    }

    fn dir(&self) -> &Path {
        self.log.parent().unwrap_or(Path::new("."))
    }

    fn refresh(&mut self) -> io::Result<&Index> {
        match File::open(&self.log) {
            Ok(file) => self.refresh_from(&file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(self.index.insert(Index::default()))
            }
            Err(error) => Err(error),
        }
    }

    fn refresh_from(&mut self, file: &File) -> io::Result<&Index> {
        let meta = file.metadata()?;
        let id = FileId::of(&meta);
        let index = self.index.get_or_insert_with(Index::default);
        if index.end > meta.len() || index.file != Some(id) {
            *index = Index {
                file: Some(id),
                ..Index::default()
            };
        }
        scan_from(file, meta.len(), index)?;
        Ok(index)
    }

    pub(crate) fn records(&mut self) -> io::Result<&[Record]> {
        Ok(&self.refresh()?.records)
    }

    pub(crate) fn blob_meta(&self, key: &BlobKey) -> Option<BlobMeta> {
        self.index.as_ref()?.blobs.get(key).copied()
    }

    fn stored(&mut self, key: &BlobKey, newer: Option<(&BlobKey, &[u8])>) -> Result<Stored, Error> {
        let file = File::open(&self.log).map_err(|_| damaged())?;
        let index = self.refresh_from(&file).map_err(|_| damaged())?;
        text_within(&file, index, key, newer).ok_or_else(damaged)
    }

    pub(crate) fn blob(&mut self, key: &BlobKey) -> Result<Packed, Error> {
        Ok(match self.stored(key, None)? {
            Stored::Packed(packed) => packed,
            Stored::Text(text) => Packed::of(&text),
        })
    }

    pub(crate) fn text(&mut self, key: &BlobKey) -> Result<Vec<u8>, Error> {
        self.text_from(key, None)
    }

    fn text_from(
        &mut self,
        key: &BlobKey,
        newer: Option<(&BlobKey, &[u8])>,
    ) -> Result<Vec<u8>, Error> {
        match self.stored(key, newer)? {
            Stored::Packed(packed) => packed.unpack("").map_err(|_| damaged()),
            Stored::Text(text) => Ok(text),
        }
    }

    pub(crate) fn readable(&mut self, key: &BlobKey) -> bool {
        self.stored(key, None).is_ok()
    }

    fn copy_of(&mut self, key: &BlobKey) -> Option<Vec<u8>> {
        let file = File::open(&self.log).ok()?;
        let meta = *self.refresh_from(&file).ok()?.blobs.get(key)?;
        let payload = checked_payload(&file, &meta, key)?;
        match meta.form {
            Form::Packed => packed_from(&payload, &meta, key).map(|_| frame(BLOB, &payload)),
            Form::Text(_) => Some(frame(TEXT, &payload)),
        }
    }

    pub(crate) fn append(&mut self, blobs: Vec<Packed>, record: Record) -> io::Result<()> {
        let staged = self.stage(blobs, record)?;
        let synced = staged.sync();
        staged.finish(synced)
    }

    pub(crate) fn stage(&mut self, blobs: Vec<Packed>, record: Record) -> io::Result<Staged> {
        let end = self.refresh()?.end;
        let mut out = Vec::new();
        let mut written = HashSet::new();
        for packed in &blobs {
            let key = BlobKey::of(packed);
            if self.blob_meta(&key).is_none() && written.insert(key) {
                out.extend_from_slice(&frame(BLOB, &blob_payload(key, packed)));
            }
        }
        out.extend_from_slice(&frame(RECORD, &record.to_bytes()));
        let created = !self.log.exists();
        fs::create_dir_all(self.dir())?;
        let mut file = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.log)?;
        if file.metadata()?.len() != end {
            file.set_len(end)?;
        }
        file.seek(SeekFrom::Start(end))?;
        let half = out.len() / 2;
        file.write_all(&out[..half])?;
        hit(Step::HistoryWriting)?;
        file.write_all(&out[half..])?;
        hit(Step::HistoryWritten)?;
        Ok(Staged {
            file,
            start: end,
            dir: self.dir().to_owned(),
            created,
        })
    }
}

pub(crate) struct Staged {
    file: File,
    start: u64,
    dir: PathBuf,
    created: bool,
}

impl Staged {
    pub(crate) fn sync(&self) -> io::Result<()> {
        self.file.sync_data()?;
        sync_dir(&self.dir)?;
        if self.created
            && let Some(parent) = self.dir.parent()
        {
            sync_dir(parent)?;
        }
        Ok(())
    }

    pub(crate) fn finish(&self, synced: io::Result<()>) -> io::Result<()> {
        let finished = synced.and_then(|()| hit(Step::HistorySynced));
        if finished.is_err() {
            self.file.set_len(self.start).ok();
        }
        finished
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Version {
    Now,
    Then(Option<BlobKey>),
    Unknown,
}

pub(crate) struct Landed<'r> {
    pub(crate) points: Vec<(RecordIx, &'r Record)>,
    gap_before: Vec<bool>,
    gap_at_end: bool,
}

pub(crate) fn landed(records: &[Record], current: FileHash) -> Landed<'_> {
    let mut points: Vec<(RecordIx, &Record)> = Vec::new();
    for (ix, record) in records.iter().enumerate() {
        while points.last().is_some_and(|(_, last)| {
            last.file_after != record.file_before && last.file_before == record.file_before
        }) {
            points.pop();
        }
        points.push((RecordIx(ix as u32), record));
    }
    while points
        .last()
        .is_some_and(|(_, last)| last.file_after != current && last.file_before == current)
    {
        points.pop();
    }
    let gap_before = points
        .iter()
        .enumerate()
        .map(|(ix, (_, record))| ix > 0 && points[ix - 1].1.file_after != record.file_before)
        .collect();
    let gap_at_end = points
        .last()
        .is_some_and(|(_, last)| last.file_after != current);
    Landed {
        points,
        gap_before,
        gap_at_end,
    }
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) struct StateAt {
    pub(crate) rows: u32,
    pub(crate) known: HashMap<EntryName, Option<BlobKey>>,
    pub(crate) complete: bool,
}

impl StateAt {
    pub(crate) fn version(&self, entry: &EntryName) -> Version {
        match self.known.get(entry) {
            Some(before) => Version::Then(*before),
            None if self.complete => Version::Now,
            None => Version::Unknown,
        }
    }
}

impl Landed<'_> {
    pub(crate) fn find(&self, id: PointId) -> Option<RecordIx> {
        self.points
            .iter()
            .find(|(_, record)| record.id() == id)
            .map(|(ix, _)| *ix)
    }

    pub(crate) fn state_at(&self, ix: RecordIx) -> Option<StateAt> {
        let from = self.points.iter().position(|(point, _)| *point == ix)?;
        let mut known = HashMap::new();
        let mut complete = !self.gap_at_end;
        for (offset, (_, record)) in self.points[from..].iter().enumerate() {
            if offset > 0 && self.gap_before[from + offset] {
                complete = false;
                break;
            }
            for change in record.changes.iter().chain(&record.carried) {
                known.entry(change.entry.clone()).or_insert(change.before);
            }
        }
        Some(StateAt {
            rows: self.points[from].1.rows,
            known,
            complete,
        })
    }
}

#[cfg(test)]
mod tests;
