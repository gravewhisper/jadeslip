mod draft;
mod history;
pub(crate) mod seam;

pub use draft::{DraftFit, DraftOffer};
pub use history::{HistoryPoint, PointChanges};

use history::{Cause, Syncing};

use crate::backup::{
    BackupStatus, BookData, RevertTarget, Slot, hash_of, hex_of, record_alias, resolve_identity,
};
use crate::book::{Extracted, Reader};
use crate::chapter::decode_source;
use crate::edit::{AfterBlock, BlockEdits, BlockOrigin, ChapterPlan, SaveOutcome, SavedChapter};
use crate::history::History;
use crate::manage::{
    ChapterOp, DeletedId, DeletedRecord, Gathered, Managed, PackageChanges, PackagePlan,
    check_plan, encode_text, hex_encode, stylesheets_of, sweep, unwrap_links,
};
use crate::replace::offers_between;
use crate::search::{
    ByteBudget, CHAPTER_BYTE_BUDGET, Query, RunPlan, SearchEnd, SearchEvent, SearchOptions,
    SearchScope, run_search,
};
use crate::zip::{
    Changes, EntryFate, EntryName, HashingOut, NewEntry, ReadAt, RevertRow, Rewritten,
    RewrittenEntry, ZipEntry, ZipIndex, directory_diff, rewrite,
};
use crate::{BookKey, CancelToken, ChapterIx, Error, FileHash, Fingerprint, Limits};
use seam::{Step, hit, sync_dir, sync_file};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const TEMP_MARK: &str = ".jadeslip-";
const TEMP_SUFFIX: &str = ".tmp";
const MAX_NAME_IN_TEMP: usize = 100;
const PARALLEL_CHECK_FROM: usize = 64;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
}

impl FileStamp {
    pub(crate) fn version(&self) -> String {
        format!("{:x}-{:x}", self.len, self.mtime_ns)
    }

    #[cfg(unix)]
    pub(crate) fn of(meta: &Metadata) -> FileStamp {
        use std::os::unix::fs::MetadataExt;
        FileStamp {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
        }
    }

    #[cfg(not(unix))]
    pub(crate) fn of(meta: &Metadata) -> FileStamp {
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos() as i128);
        FileStamp {
            dev: 0,
            ino: 0,
            len: meta.len(),
            mtime_ns,
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct CommitPhases {
    pub backup: Duration,
    pub read: Duration,
    pub hash: Duration,
    pub write: Duration,
    pub sync_temp: Duration,
    pub verify: Duration,
    pub ledger_pending: Duration,
    pub recheck: Duration,
    pub rename: Duration,
    pub sync_dir: Duration,
    pub ledger_head: Duration,
    pub history: Duration,
    pub total: Duration,
}

#[derive(Debug)]
pub struct CommitReport {
    pub bytes_written: u64,
    pub entries_rewritten: u32,
    pub entries_copied: u32,
    pub mimetype_normalised: bool,
    pub warnings: Vec<String>,
    pub phases: CommitPhases,
}

impl CommitReport {
    pub(crate) fn nothing_to_do() -> CommitReport {
        CommitReport {
            bytes_written: 0,
            entries_rewritten: 0,
            entries_copied: 0,
            mimetype_normalised: false,
            warnings: Vec::new(),
            phases: CommitPhases::default(),
        }
    }
}

struct WriterState {
    stamp: FileStamp,
    hash: FileHash,
    slot: Option<Slot>,
    history: History,
    drafts: draft::Drafts,
}

pub struct Book {
    path: PathBuf,
    identity: Fingerprint,
    data: BookData,
    limits: Limits,
    head: RwLock<Arc<Reader<File>>>,
    writer: Mutex<WriterState>,
    search_budget: Arc<ByteBudget>,
}

struct Replacements(HashMap<EntryName, NewEntry>);

impl Changes for Replacements {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error> {
        Ok(match self.0.remove(&entry.name) {
            Some(new) => EntryFate::Write(new),
            None => EntryFate::RawCopy,
        })
    }

    fn added(&mut self) -> Vec<NewEntry> {
        let mut added: Vec<NewEntry> = self.0.drain().map(|(_, new)| new).collect();
        added.sort_by(|a, b| a.name().cmp(b.name()));
        added
    }
}

struct NewFiles(Vec<NewEntry>);

impl Changes for NewFiles {
    fn take(&mut self, _entry: &ZipEntry) -> Result<EntryFate, Error> {
        Ok(EntryFate::RawCopy)
    }

    fn added(&mut self) -> Vec<NewEntry> {
        std::mem::take(&mut self.0)
    }
}

struct NoSource;

impl ReadAt for NoSource {
    fn read_exact_at(&self, _buf: &mut [u8], _offset: u64) -> io::Result<()> {
        Err(ErrorKind::UnexpectedEof.into())
    }

    fn len(&self) -> io::Result<u64> {
        Ok(0)
    }
}

enum Payload<'a> {
    Rewrite(&'a mut dyn Changes),
    Restore { backup: &'a File, hash: FileHash },
}

struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct TempWriter(File);

impl Write for TempWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        hit(Step::TempWriting)?;
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn save_failed(error: io::Error) -> Error {
    if matches!(
        error.kind(),
        ErrorKind::StorageFull | ErrorKind::FileTooLarge | ErrorKind::QuotaExceeded
    ) {
        Error::SaveFailed(
            "There is not enough disk space to save this book, so it was not changed.".to_owned(),
        )
    } else {
        Error::SaveFailed(format!(
            "The save failed, so the book was not changed. {}",
            crate::plain_io(&error)
        ))
    }
}

fn refused_as_read_only(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
    )
}

fn would_break(detail: impl std::fmt::Display) -> Error {
    Error::WouldBreak(format!(
        "This save failed its final check, so the book was not changed. {detail}."
    ))
}

#[cfg(unix)]
fn name_bytes(name: &OsStr) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes()
}

#[cfg(not(unix))]
fn name_bytes(name: &OsStr) -> &[u8] {
    name.as_encoded_bytes()
}

fn temp_prefix(book: &Path) -> Vec<u8> {
    let name = name_bytes(book.file_name().unwrap_or_default());
    let mut prefix = vec![b'.'];
    prefix.extend_from_slice(&name[..name.len().min(MAX_NAME_IN_TEMP)]);
    prefix.push(b'-');
    prefix.extend_from_slice(&hex_of(&Sha256::digest(name).into()).as_bytes()[..16]);
    prefix.extend_from_slice(TEMP_MARK.as_bytes());
    prefix
}

fn temp_path(book: &Path) -> PathBuf {
    let mut name = temp_prefix(book);
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    name.extend_from_slice(format!("{}-{serial}{TEMP_SUFFIX}", std::process::id()).as_bytes());
    book.with_file_name(temp_file_name(name))
}

#[cfg(unix)]
fn temp_file_name(name: Vec<u8>) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(name)
}

#[cfg(not(unix))]
fn temp_file_name(name: Vec<u8>) -> std::ffi::OsString {
    String::from_utf8_lossy(&name).into_owned().into()
}

fn remove_stale_temps(book: &Path) -> io::Result<u32> {
    let prefix = temp_prefix(book);
    let dir = book.parent().unwrap_or(Path::new("."));
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name_bytes(&name);
        if name.starts_with(&prefix) && name.ends_with(TEMP_SUFFIX.as_bytes()) {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(unix)]
fn create_temp(path: &Path, original: &Metadata) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, fchown};
    let file = File::options()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let _ = fchown(&file, Some(original.uid()), Some(original.gid()));
    file.set_permissions(original.permissions())?;
    Ok(file)
}

#[cfg(not(unix))]
fn create_temp(path: &Path, original: &Metadata) -> io::Result<File> {
    let file = File::options().write(true).create_new(true).open(path)?;
    file.set_permissions(original.permissions())?;
    Ok(file)
}

fn verify_rewrite(
    temp: &Reader<File>,
    head: &Reader<File>,
    rewritten: &Rewritten,
    spine_delta: i64,
    limits: &Limits,
) -> Result<(), Error> {
    let expected_rows = head.rows().len() as i64 + spine_delta;
    if temp.rows().len() as i64 != expected_rows {
        return Err(would_break("The chapter count came out wrong"));
    }
    let new_zip = temp.zip();
    let total = rewritten.copied.len() + rewritten.rewritten.len();
    if new_zip.entries().len() != total {
        return Err(would_break("The file count came out wrong"));
    }
    if !new_zip.mimetype_is_first_and_stored() {
        return Err(would_break("The mimetype record came out wrong"));
    }
    new_zip
        .check_copied_records(temp.source(), head.zip(), head.source(), &rewritten.copied)
        .map_err(would_break)?;
    let check = |slice: &[RewrittenEntry]| -> Result<(), Error> {
        for expected in slice {
            let entry = new_zip
                .entry(&expected.name)
                .ok_or_else(|| would_break("A changed file is missing"))?;
            if entry.crc32 != expected.crc32 || entry.inflated != expected.inflated {
                return Err(would_break("A changed file's directory record is wrong"));
            }
            let bytes = new_zip
                .read_entry(temp.source(), entry, limits)
                .map_err(would_break)?;
            if crc32fast::hash(&bytes) != expected.crc32 {
                return Err(would_break("A changed file failed its checksum"));
            }
        }
        Ok(())
    };
    if rewritten.rewritten.len() < PARALLEL_CHECK_FROM {
        return check(&rewritten.rewritten);
    }
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let per_thread = rewritten.rewritten.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let workers: Vec<_> = rewritten
            .rewritten
            .chunks(per_thread)
            .map(|chunk| scope.spawn(|| check(chunk)))
            .collect();
        workers
            .into_iter()
            .try_for_each(|worker| worker.join().expect("a verify thread does not panic"))
    })
}

impl Book {
    pub fn open(path: &Path, data_dir: &Path, key: BookKey, limits: Limits) -> Result<Book, Error> {
        Book::open_with(path, data_dir, limits, |_| key)
    }

    pub fn open_by_fingerprint(
        path: &Path,
        data_dir: &Path,
        limits: Limits,
    ) -> Result<Book, Error> {
        Book::open_with(path, data_dir, limits, BookKey::of)
    }

    fn open_with(
        path: &Path,
        data_dir: &Path,
        limits: Limits,
        key_of: impl FnOnce(&Fingerprint) -> BookKey,
    ) -> Result<Book, Error> {
        let path = fs::canonicalize(path)?;
        let file = File::open(&path)?;
        let stamp = FileStamp::of(&file.metadata()?);
        let hash = hash_of(&file)?;
        let reader = Reader::open(file, limits)?;
        let identity = resolve_identity(data_dir, &reader.summary().fingerprint);
        let data = BookData::new(data_dir, key_of(&identity));
        let history = History::new(data.root());
        let book = Book {
            path,
            identity,
            data,
            limits,
            head: RwLock::new(Arc::new(reader)),
            writer: Mutex::new(WriterState {
                stamp,
                hash,
                slot: None,
                history,
                drafts: draft::Drafts::default(),
            }),
            search_budget: ByteBudget::new(CHAPTER_BYTE_BUDGET),
        };
        if book.data.has_any_slot() {
            let _ = book.recover();
        } else {
            let _ = remove_stale_temps(&book.path);
        }
        Ok(book)
    }

    #[cfg(test)]
    pub(crate) fn data_has_any_slot(&self) -> bool {
        self.data.has_any_slot()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> Fingerprint {
        self.identity
    }

    pub fn search(
        &self,
        query: &Query,
        scope: SearchScope,
        options: &SearchOptions,
        cancel: &CancelToken,
        sink: &mut dyn FnMut(SearchEvent),
    ) -> SearchEnd {
        let reader = self.read();
        let chapters = scope.rows(&reader).map(|row| row.ix).collect();
        run_search(
            reader,
            self.search_budget.clone(),
            query,
            options,
            RunPlan {
                chapters,
                scan: None,
            },
            cancel,
            sink,
        )
    }

    pub fn search_high_water(&self) -> u64 {
        self.search_budget.high_water()
    }

    pub fn read(&self) -> Arc<Reader<File>> {
        self.head
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn changed_on_disk(&self) -> bool {
        let Ok(state) = self.writer.try_lock() else {
            return false;
        };
        fs::metadata(&self.path).map_or(true, |meta| FileStamp::of(&meta) != state.stamp)
    }

    pub fn recover(&self) -> Result<u32, Error> {
        let _state = self.lock_state();
        let _lock = self.data.lock()?;
        self.recover_locked()
    }

    fn recover_locked(&self) -> Result<u32, Error> {
        let removed = remove_stale_temps(&self.path)?;
        sync_dir(self.path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| hit(Step::RecoveryDirSynced))
            .map_err(save_failed)?;
        let current = File::open(&self.path)?;
        let settled = self.data.recover(&current)?;
        Ok(removed + settled)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, WriterState> {
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_unchanged(&self, state: &mut WriterState) -> Result<(), Error> {
        let meta = fs::metadata(&self.path).map_err(|_| Error::ChangedOnDisk)?;
        let stamp = FileStamp::of(&meta);
        if stamp == state.stamp {
            return Ok(());
        }
        let on_disk = File::open(&self.path).and_then(|file| hash_of(&file))?;
        if state.slot.as_ref().map(|slot| slot.ledger.head) != Some(on_disk) {
            return Err(Error::ChangedOnDisk);
        }
        state.stamp = stamp;
        state.hash = on_disk;
        state.slot = None;
        Ok(())
    }

    fn check_writable(&self) -> Result<Metadata, Error> {
        let read_only = || {
            Error::NotEditable(
                "This book is read-only, so it cannot be saved. Change the file's permissions and try again."
                    .to_owned(),
            )
        };
        let meta = fs::metadata(&self.path)?;
        if meta.permissions().readonly() {
            return Err(read_only());
        }
        match File::options().write(true).open(&self.path) {
            Ok(_) => Ok(meta),
            Err(error) if refused_as_read_only(&error) => Err(read_only()),
            Err(error) => Err(error.into()),
        }
    }

    fn ensure_slot<'s>(
        &self,
        slot: &'s mut Option<Slot>,
        current: FileHash,
    ) -> Result<&'s mut Slot, Error> {
        if let Some(slot) = slot {
            return Ok(slot);
        }
        self.recover_locked()?;
        Ok(slot.insert(self.data.ensure(&self.path, current)?))
    }

    fn valid_slot<'s>(
        &self,
        slot: &'s mut Option<Slot>,
        head: &Reader<File>,
    ) -> Result<Option<&'s mut Slot>, Error> {
        if slot.as_ref().is_none_or(|slot| !slot.is_stored()) {
            *slot = self.data.find_valid(hash_of(head.source())?);
        }
        Ok(slot.as_mut())
    }

    pub fn backup_status(&self) -> Result<BackupStatus, Error> {
        let mut state = self.lock_state();
        let head = self.read();
        self.check_unchanged(&mut state)?;
        if let Some(slot) = self.valid_slot(&mut state.slot, &head)? {
            let names = head.entry_names();
            let deleted = slot
                .deleted_records()
                .into_iter()
                .filter(|record| !entry_in(&names, &record.entry))
                .map(|record| record.chapter)
                .collect();
            return Ok(BackupStatus::Valid {
                created_at: slot.ledger.created_at,
                deleted,
            });
        }
        Ok(if self.data.has_any_slot() {
            BackupStatus::OtherFileOnly
        } else {
            BackupStatus::None
        })
    }

    pub fn revert_list(&self) -> Result<Vec<RevertRow>, Error> {
        let mut state = self.lock_state();
        let head = self.read();
        self.check_unchanged(&mut state)?;
        let slot = self
            .valid_slot(&mut state.slot, &head)?
            .ok_or_else(no_usable_backup)?;
        let backup = File::open(slot.backup_path()).map_err(|_| no_usable_backup())?;
        let backup_zip = ZipIndex::parse(&backup, &self.limits)?;
        Ok(directory_diff(head.zip(), &backup_zip))
    }

    pub fn revert(&self, target: RevertTarget) -> Result<CommitReport, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        let head = self.read();
        self.check_unchanged(&mut state)?;
        let slot = self
            .valid_slot(&mut state.slot, &head)?
            .ok_or_else(no_usable_backup)?;
        let backup = slot.open_verified()?;
        let base = slot.ledger.base;
        let at_base = slot.ledger.head == base;
        match target {
            RevertTarget::WholeBook => {
                if at_base {
                    return Ok(CommitReport::nothing_to_do());
                }
                let report = self.commit(
                    &mut state,
                    &head,
                    Payload::Restore {
                        backup: &backup,
                        hash: base,
                    },
                    None,
                    Cause::Save,
                )?;
                if let Some(slot) = &state.slot {
                    let _ = slot.clear_deleted();
                }
                Ok(report)
            }
            RevertTarget::Chapter(name) => {
                let is_package = name == *head.opf_entry()
                    || head.nav_name() == Some(&name)
                    || head.ncx_name() == Some(&name);
                if is_package {
                    return Err(Error::NotEditable(
                        "The contents change with the book, so only the whole book can be reverted."
                            .to_owned(),
                    ));
                }
                let backup_zip = ZipIndex::parse(&backup, &self.limits)?;
                let (Some(old), Some(new)) = (backup_zip.entry(&name), head.zip().entry(&name))
                else {
                    return Err(Error::NotEditable(
                        "This chapter was added or removed since the backup, so only the whole book can be reverted."
                            .to_owned(),
                    ));
                };
                if old.crc32 == new.crc32 && old.inflated == new.inflated {
                    return Ok(CommitReport::nothing_to_do());
                }
                let mut bytes = backup_zip.read_entry(&backup, old, &self.limits)?;
                let gone: BTreeSet<EntryName> = backup_zip
                    .entries()
                    .iter()
                    .map(|entry| entry.name.clone())
                    .filter(|entry| head.zip().entry(entry).is_none())
                    .collect();
                let mut warnings = Vec::new();
                if let Some(unwrapped) = unwrap_links(&bytes, &name, &gone, &self.limits)? {
                    bytes = unwrapped.bytes;
                    warnings = unwrapped.warnings;
                    let current = head.zip().read_entry(head.source(), new, &self.limits)?;
                    if current == bytes {
                        return Ok(CommitReport::nothing_to_do());
                    }
                }
                let entry = NewEntry::deflate(name.clone(), &bytes);
                let mut changes = Replacements(HashMap::from([(name, entry)]));
                let mut report = self.commit(
                    &mut state,
                    &head,
                    Payload::Rewrite(&mut changes),
                    None,
                    Cause::Save,
                )?;
                report.warnings.append(&mut warnings);
                Ok(report)
            }
        }
    }

    pub fn export_copy(&self, dest: &Path) -> Result<(), Error> {
        let refused = || {
            Error::NotEditable(
                "A copy cannot replace the book itself. Choose another name or folder.".to_owned(),
            )
        };
        if fs::canonicalize(dest).is_ok_and(|found| found == self.path) {
            return Err(refused());
        }
        let head = self.read();
        let len = head.source().len().map_err(save_failed)?;
        let temp = TempGuard {
            path: temp_path(dest),
            armed: true,
        };
        let mut out = HashingOut::new(TempWriter(
            File::options()
                .write(true)
                .create_new(true)
                .open(&temp.path)
                .map_err(save_failed)?,
        ));
        out.copy_from(head.source(), 0..len).map_err(save_failed)?;
        let (TempWriter(file), hash, ..) = out.finish().map_err(save_failed)?;
        file.sync_all().map_err(save_failed)?;
        drop(file);
        let written = File::open(&temp.path).and_then(|file| hash_of(&file));
        if written.map_err(save_failed)? != hash {
            return Err(would_break("The copy did not match the book"));
        }
        fs::rename(&temp.path, dest).map_err(save_failed)?;
        drop(temp);
        sync_dir(dest.parent().unwrap_or(Path::new("."))).map_err(save_failed)
    }

    pub fn save_entries(&self, entries: Vec<NewEntry>) -> Result<CommitReport, Error> {
        let mut changes = Replacements(
            entries
                .into_iter()
                .map(|entry| (entry.name().clone(), entry))
                .collect(),
        );
        self.save_changes(&mut changes)
    }

    pub fn save_chapter(&self, ix: ChapterIx, edits: &BlockEdits) -> Result<SavedChapter, Error> {
        let mut state = self.lock_state();
        let head = self.read();
        let (name, plan) = head.plan_save(ix, edits)?;
        let (outcome, warnings, order) = match plan {
            ChapterPlan::Unchanged { order } => (SaveOutcome::Unchanged, Vec::new(), order),
            ChapterPlan::Write(planned) => {
                let _lock = self.data.lock()?;
                let entry = NewEntry::deflate(name.clone(), &planned.bytes);
                let mut changes = Replacements(HashMap::from([(name, entry)]));
                let report = self.commit(
                    &mut state,
                    &head,
                    Payload::Rewrite(&mut changes),
                    None,
                    Cause::Save,
                )?;
                let mut warnings = planned.warnings;
                warnings.extend(report.warnings.iter().cloned());
                (
                    SaveOutcome::Written(Box::new(report)),
                    warnings,
                    planned.order,
                )
            }
        };
        self.drop_draft_locked(&state);
        let current = self.read();
        let rev = current.rev_of(ix)?;
        let fix_offers = if matches!(outcome, SaveOutcome::Written(_)) {
            offers_between(&head, &current, ix)
        } else {
            Vec::new()
        };
        let after = if matches!(outcome, SaveOutcome::Unchanged) {
            let kept = |origin| AfterBlock {
                origin,
                display: None,
            };
            Some(order.into_iter().map(kept).collect())
        } else {
            current
                .display_blocks(ix)
                .ok()
                .filter(|shown| shown.len() == order.len())
                .map(|shown| {
                    order
                        .into_iter()
                        .zip(shown)
                        .map(|(origin, display)| AfterBlock {
                            origin,
                            display: matches!(origin, BlockOrigin::Change(_)).then_some(display),
                        })
                        .collect()
                })
        };
        Ok(SavedChapter {
            rev,
            outcome,
            after,
            warnings,
            fix_offers,
        })
    }

    pub fn manage(&self, op: ChapterOp, shown: Option<ChapterIx>) -> Result<Managed, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        let head = self.read();
        self.check_unchanged(&mut state)?;
        let names = head.entry_names();
        let records = self
            .valid_slot(&mut state.slot, &head)?
            .map_or_else(Vec::new, |slot| {
                let (stale, live): (Vec<_>, Vec<_>) = slot
                    .deleted_records()
                    .into_iter()
                    .partition(|record| entry_in(&names, &record.entry));
                for record in stale {
                    let _ = slot.remove_deleted(record.chapter.id);
                }
                live
            });
        let reserved = records
            .iter()
            .filter_map(|record| EntryName::parse(&record.entry).ok())
            .collect();
        let docs: Vec<_> = head.toc_docs()?.into_iter().flatten().collect();
        let package = head.package(&docs, &names, reserved);
        package.ensure_editable()?;
        let mut gathered = Gathered::default();
        match &op {
            ChapterOp::Add { after, .. } => {
                if let Some(row) = package.add_neighbour(*after)
                    && let Ok(Extracted::Document(bytes)) = head.extract(ChapterIx(row))
                {
                    let (source, _) = decode_source(bytes);
                    gathered.stylesheets = stylesheets_of(&source, &self.limits);
                }
            }
            ChapterOp::Rename { row, .. } => {
                if let (_, Some(row)) = package.rename_target(*row)? {
                    gathered.chapter = head.model(ChapterIx(row)).ok().flatten();
                }
            }
            ChapterOp::UndoDelete { deleted } => {
                gathered.record = records
                    .iter()
                    .find(|record| record.chapter.id == *deleted)
                    .cloned();
            }
            ChapterOp::Move { .. } | ChapterOp::Delete { .. } => {}
        }
        let mut plan = package.plan(&op, gathered)?;
        check_plan(&head, &package, &plan, &self.limits)?;
        let shown_name = shown
            .and_then(|ix| head.rows().get(ix.0 as usize))
            .and_then(|row| row.href.clone());
        if let (Some(name), Some(record)) = (&plan.removed, &mut plan.record) {
            let bytes = head
                .zip()
                .entry(name)
                .ok_or_else(no_chapter_file)
                .and_then(|entry| head.zip().read_entry(head.source(), entry, &self.limits))?;
            record.crc32 = crc32fast::hash(&bytes);
            record.bytes = hex_encode(&bytes);
            record.chapter.deleted_at = crate::backup::now();
            let highest = records.iter().map(|found| found.chapter.id.0).max();
            record.chapter.id = DeletedId(highest.map_or(1, |id| id + 1));
        }
        let mut link_writes = Vec::new();
        let mut warnings = Vec::new();
        match &op {
            ChapterOp::Delete { .. } => {
                if let (Some(name), Some(record)) = (&plan.removed, &mut plan.record) {
                    let documents = package.documents_except(name);
                    let gone = BTreeSet::from([name.clone()]);
                    let (rewritten, found) =
                        sweep(&head, &documents, &gone, &self.limits).all_warnings();
                    warnings = found;
                    for chapter in rewritten {
                        record.linking.push(chapter.window);
                        link_writes.push((chapter.name, chapter.bytes));
                    }
                }
            }
            ChapterOp::UndoDelete { .. } => {
                let found = self.undo_links(&head, &mut plan, &records);
                link_writes = found.0;
                warnings = found.1;
            }
            _ => {}
        }
        let (restored, sidecar) = match &op {
            ChapterOp::UndoDelete { deleted } => (Some(*deleted), None),
            _ => (None, plan.record.as_ref()),
        };
        let mut writes = HashMap::new();
        let mut write = |name: &EntryName, new: &[u8], old: &[u8]| {
            if new != old {
                writes.insert(name.clone(), NewEntry::deflate(name.clone(), new));
            }
        };
        write(
            package.opf_name,
            &plan.opf,
            &encode_text(package.opf.text_form, package.opf.source.clone()),
        );
        for (doc, bytes) in package.docs.iter().zip(&plan.docs) {
            write(
                doc.name,
                bytes,
                &encode_text(doc.toc.text_form, doc.toc.source.clone()),
            );
        }
        if let Some((name, bytes)) = &plan.chapter {
            writes.insert(name.clone(), NewEntry::deflate(name.clone(), bytes));
        }
        for (name, bytes) in &link_writes {
            writes.insert(name.clone(), NewEntry::deflate(name.clone(), bytes));
        }
        let unchanged = writes.is_empty() && plan.added.is_none() && plan.removed.is_none();
        let landing_name = plan.landing.clone();
        let deleted_id = sidecar.map(|record| record.chapter.id);
        let mut report = if unchanged {
            CommitReport::nothing_to_do()
        } else {
            let mut changes = PackageChanges {
                writes,
                drops: plan.removed.iter().cloned().collect(),
                added: plan
                    .added
                    .iter()
                    .map(|(name, bytes)| NewEntry::deflate(name.clone(), bytes))
                    .collect(),
                delta: plan.delta,
            };
            self.commit(
                &mut state,
                &head,
                Payload::Rewrite(&mut changes),
                sidecar,
                Cause::Save,
            )?
        };
        report.warnings.append(&mut warnings);
        let current = self.read();
        if let (Some(id), Some(slot)) = (restored, &state.slot) {
            let _ = slot.remove_deleted(id);
        }
        let row_of = |name: &EntryName| current.chapter_of_entry(name.as_str());
        Ok(Managed {
            report,
            landing: landing_name.as_ref().and_then(row_of),
            shown: shown_name.as_ref().and_then(row_of),
            deleted: deleted_id,
        })
    }

    fn undo_links(
        &self,
        head: &Reader<File>,
        plan: &mut PackagePlan,
        records: &[DeletedRecord],
    ) -> (Vec<(EntryName, Vec<u8>)>, Vec<String>) {
        let mut writes = Vec::new();
        let mut warnings = Vec::new();
        let Some(record) = &plan.record else {
            return (writes, warnings);
        };
        let mut left_plain = 0;
        for window in &record.linking {
            let restored = EntryName::parse(&window.name).ok().and_then(|name| {
                let entry = head.zip().entry(&name)?;
                let current = head
                    .zip()
                    .read_entry(head.source(), entry, &self.limits)
                    .ok()?;
                Some((name, window.undo(&current)?))
            });
            match restored {
                Some(found) => writes.push(found),
                None => left_plain += 1,
            }
        }
        if left_plain > 0 {
            warnings.push(format!(
                "{left_plain} chapters changed after the delete, so their links to this chapter stay as plain text."
            ));
        }
        let gone: BTreeSet<EntryName> = records
            .iter()
            .filter(|other| other.chapter.id != record.chapter.id)
            .filter_map(|other| EntryName::parse(&other.entry).ok())
            .collect();
        if let Some((name, bytes)) = &mut plan.added
            && let Ok(Some(unwrapped)) = unwrap_links(bytes.as_slice(), name, &gone, &self.limits)
        {
            *bytes = unwrapped.bytes;
            warnings.extend(unwrapped.warnings);
        }
        (writes, warnings)
    }

    pub(crate) fn create(
        path: &Path,
        data_dir: &Path,
        entries: Vec<NewEntry>,
        limits: Limits,
    ) -> Result<Book, Error> {
        let name_taken = || {
            Error::NotEditable(
                "A file with that name already exists, so nothing was written. Choose another name."
                    .to_owned(),
            )
        };
        if path.exists() {
            return Err(name_taken());
        }
        let mut temp = TempGuard {
            path: temp_path(path),
            armed: false,
        };
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&temp.path)
            .map_err(save_failed)?;
        temp.armed = true;
        let mut out = HashingOut::new(TempWriter(file));
        let mut changes = NewFiles(entries);
        rewrite(
            &NoSource,
            &ZipIndex::empty(),
            &mut changes,
            &limits,
            &mut out,
        )
        .map_err(as_save_error)?;
        let (TempWriter(file), ..) = out.finish().map_err(save_failed)?;
        sync_file(&file, Step::TempSynced).map_err(save_failed)?;
        drop(file);
        let reader = Reader::open(File::open(&temp.path).map_err(save_failed)?, limits)
            .map_err(would_break)?;
        reader.check_whole_package()?;
        hit(Step::TempVerified).map_err(save_failed)?;
        match fs::hard_link(&temp.path, path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => return Err(name_taken()),
            Err(_) if path.exists() => return Err(name_taken()),
            Err(_) => fs::rename(&temp.path, path).map_err(save_failed)?,
        }
        hit(Step::BookRenamed).map_err(save_failed)?;
        drop(temp);
        sync_dir(path.parent().unwrap_or(Path::new("."))).map_err(save_failed)?;
        Book::open_by_fingerprint(path, data_dir, limits)
    }

    pub(crate) fn snapshot(&self) -> (FileStamp, Arc<Reader<File>>) {
        let state = self.lock_state();
        (state.stamp, self.read())
    }

    pub(crate) fn budget(&self) -> Arc<ByteBudget> {
        self.search_budget.clone()
    }

    pub(crate) fn save_if_current(
        &self,
        snapshot: FileStamp,
        changes: &mut dyn Changes,
    ) -> Result<CommitReport, Error> {
        let mut state = self.lock_state();
        if state.stamp != snapshot {
            return Err(Error::PlanStale);
        }
        let _lock = self.data.lock()?;
        let head = self.read();
        self.commit(
            &mut state,
            &head,
            Payload::Rewrite(changes),
            None,
            Cause::Save,
        )
    }

    pub(crate) fn save_changes(&self, changes: &mut dyn Changes) -> Result<CommitReport, Error> {
        let mut state = self.lock_state();
        let _lock = self.data.lock()?;
        let head = self.read();
        self.commit(
            &mut state,
            &head,
            Payload::Rewrite(changes),
            None,
            Cause::Save,
        )
    }

    fn commit(
        &self,
        state: &mut WriterState,
        head: &Reader<File>,
        payload: Payload<'_>,
        deleted: Option<&DeletedRecord>,
        cause: Cause,
    ) -> Result<CommitReport, Error> {
        let started = Instant::now();
        let mut phases = CommitPhases::default();
        self.check_unchanged(state)?;
        let original = self.check_writable()?;
        let (stamp, seen) = (state.stamp, state.hash);
        if state
            .slot
            .as_ref()
            .is_some_and(|slot| slot.ledger.pending_head.is_some() || !slot.is_stored())
        {
            state.slot = None;
        }
        let slot = self.ensure_slot(&mut state.slot, seen)?;
        phases.backup = started.elapsed();

        remove_stale_temps(&self.path).map_err(save_failed)?;
        let mut temp = TempGuard {
            path: temp_path(&self.path),
            armed: false,
        };
        let file = create_temp(&temp.path, &original).map_err(|error| {
            if refused_as_read_only(&error) {
                Error::NotEditable(
                    "The folder holding this book cannot be written, so it cannot be saved."
                        .to_owned(),
                )
            } else {
                save_failed(error)
            }
        })?;
        temp.armed = true;
        hit(Step::TempCreated).map_err(save_failed)?;

        let mut out = HashingOut::new(TempWriter(file));
        let (rewritten, spine_delta, restoring) = match payload {
            Payload::Rewrite(changes) => {
                let rewritten = rewrite(head.source(), head.zip(), changes, &self.limits, &mut out)
                    .map_err(as_save_error)?;
                (Some(rewritten), changes.spine_delta(), None)
            }
            Payload::Restore { backup, hash } => {
                let len = backup.len().map_err(save_failed)?;
                out.copy_from(backup, 0..len).map_err(save_failed)?;
                (None, 0, Some(hash))
            }
        };
        let (TempWriter(file), hash, bytes_written, times) = out.finish().map_err(save_failed)?;
        phases.read = times.read;
        phases.hash = times.hash;
        phases.write = times.write;
        hit(Step::TempWritten).map_err(save_failed)?;
        if restoring.is_some_and(|expected| expected != hash) {
            return Err(would_break("The backup changed while it was being read"));
        }

        let mut warnings = Vec::new();
        let timer = Instant::now();
        let (rows, file_before) = (head.rows().len(), slot.ledger.head);
        let record = move |changes| cause.record(rows, file_before, hash, changes);
        let staged = rewritten.as_ref().map(|rewritten| {
            let names = history::rewritten_names(head, rewritten);
            history::start(&mut state.history, head, names, &self.limits, record)
        });
        phases.history = timer.elapsed();

        let timer = Instant::now();
        sync_file(&file, Step::TempSynced).map_err(save_failed)?;
        phases.sync_temp = timer.elapsed();
        let mut syncing = staged.map(|staged| staged.map(Syncing::spawn));
        let new_stamp = FileStamp::of(&file.metadata().map_err(save_failed)?);
        drop(file);

        let timer = Instant::now();
        let reopened = File::open(&temp.path).map_err(save_failed)?;
        let mut reader = Reader::open(reopened, self.limits).map_err(would_break)?;
        reader.keep_session(head.session());
        if let Some(rewritten) = &rewritten {
            verify_rewrite(&reader, head, rewritten, spine_delta, &self.limits)?;
        }
        phases.verify = timer.elapsed();
        hit(Step::TempVerified).map_err(save_failed)?;

        if reader.summary().fingerprint != head.summary().fingerprint {
            record_alias(
                self.data.data_dir(),
                &reader.summary().fingerprint,
                &self.identity,
            )
            .map_err(save_failed)?;
        }
        if let Some(record) = deleted {
            slot.write_deleted(record).map_err(save_failed)?;
        }

        if syncing.is_none() {
            let names = history::diffed_names(head, &reader);
            syncing = Some(
                history::start(&mut state.history, head, names, &self.limits, record)
                    .map(Syncing::spawn),
            );
        }

        let timer = Instant::now();
        let pending = slot.set_pending(hash).map_err(save_failed);
        if let Some(Err(detail)) = syncing.map(|started| started.and_then(Syncing::finish)) {
            warnings.push(history_warning(&detail));
        }
        phases.ledger_pending = timer.elapsed();
        let replaced = pending.and_then(|()| {
            let timer = Instant::now();
            let unchanged = File::open(&self.path)
                .and_then(|file| hash_of(&file))
                .is_ok_and(|on_disk| on_disk == seen);
            phases.recheck = timer.elapsed();
            if !unchanged {
                return Err(Error::ChangedOnDisk);
            }
            let on_disk = fs::metadata(&self.path).map_err(|_| Error::ChangedOnDisk)?;
            if FileStamp::of(&on_disk) != stamp {
                return Err(Error::ChangedOnDisk);
            }
            hit(Step::StampRechecked).map_err(save_failed)?;
            let renaming = Instant::now();
            fs::rename(&temp.path, &self.path).map_err(save_failed)?;
            phases.rename = renaming.elapsed();
            Ok(())
        });
        if let Err(error) = replaced {
            let _ = slot.clear_pending();
            return Err(error);
        }
        temp.armed = false;

        let timer = Instant::now();
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let dir_synced = hit(Step::BookRenamed)
            .and_then(|()| sync_dir(dir))
            .and_then(|()| hit(Step::BookDirSynced));
        phases.sync_dir = timer.elapsed();
        let timer = Instant::now();
        match dir_synced {
            Err(error) => warnings.push(format!(
                "The book was saved, but its folder could not be flushed to disk, so a power cut could still lose this save. {}",
                crate::plain_io(&error)
            )),
            Ok(()) => {
                if let Err(error) = slot.settle(hash) {
                    warnings.push(format!(
                        "The book was saved, but its backup record was not updated. It will be repaired on the next open. {}",
                        crate::plain_io(&error)
                    ));
                }
            }
        }
        phases.ledger_head = timer.elapsed();
        phases.total = started.elapsed();

        state.stamp = new_stamp;
        state.hash = hash;
        let total_entries = reader.zip().entries().len() as u32;
        *self
            .head
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(reader);

        let entries_rewritten = rewritten
            .as_ref()
            .map_or(0, |rewritten| rewritten.rewritten.len() as u32);
        Ok(CommitReport {
            bytes_written,
            entries_rewritten,
            entries_copied: total_entries - entries_rewritten,
            mimetype_normalised: rewritten
                .as_ref()
                .is_some_and(|rewritten| rewritten.mimetype_normalised),
            warnings,
            phases,
        })
    }
}

fn entry_in(names: &std::collections::BTreeSet<EntryName>, entry: &str) -> bool {
    EntryName::parse(entry).is_ok_and(|name| names.contains(&name))
}

pub(crate) fn no_chapter_file() -> Error {
    Error::Book("This chapter's file is missing from the book.".to_owned())
}

fn history_warning(detail: &str) -> String {
    format!("The book was saved, but this save could not be added to its history. {detail}")
}

fn no_usable_backup() -> Error {
    Error::NotEditable(
        "This book has no backup that matches this file, so nothing can be reverted.".to_owned(),
    )
}

fn as_save_error(error: Error) -> Error {
    match error {
        Error::Io(error) => save_failed(error),
        other => other,
    }
}

#[cfg(test)]
mod tests;
