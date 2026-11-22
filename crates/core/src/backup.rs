use crate::commit::seam::{LedgerWrite, Step, hit, sync_dir, sync_file};
use crate::manage::{DeletedChapter, DeletedId, DeletedRecord};
use crate::temp::TempFile;
use crate::zip::{EntryName, ReadAt};
use crate::{BookKey, Error, FileHash, Fingerprint};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BACKUP_FILE: &str = "book.epub";
const LEDGER_FILE: &str = "ledger.json";
const TEMP_SUFFIX: &str = ".tmp";
const LEDGER_VERSION: u64 = 1;
const HASH_CHUNK: u64 = 1 << 20;
const LOCK_ATTEMPTS: u32 = 100;
const LOCK_RETRY: Duration = Duration::from_millis(1);
const DELETED_DIR: &str = "deleted";
pub(crate) const ALIASES_DIR: &str = "aliases";

pub enum BackupStatus {
    None,
    Valid {
        created_at: u64,
        deleted: Vec<DeletedChapter>,
    },
    OtherFileOnly,
}

pub enum RevertTarget {
    Chapter(EntryName),
    WholeBook,
}

pub(crate) fn hash_of<R: ReadAt>(source: &R) -> io::Result<FileHash> {
    let len = source.len()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; HASH_CHUNK.min(len) as usize];
    let mut at = 0;
    while at < len {
        let take = HASH_CHUNK.min(len - at) as usize;
        source.read_exact_at(&mut buf[..take], at)?;
        hasher.update(&buf[..take]);
        at += take as u64;
    }
    Ok(FileHash(hasher.finalize().into()))
}

pub(crate) fn hex_of(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn hex(hash: &FileHash) -> String {
    hex_of(&hash.0)
}

pub(crate) fn parse_hex(text: &str) -> Option<FileHash> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut hash = [0; 32];
    for (ix, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[ix * 2..ix * 2 + 2], 16).ok()?;
    }
    Some(FileHash(hash))
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn backup_failed(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::StorageFull {
        Error::SaveFailed(
            "There is not enough disk space for the backup, so the book was not changed."
                .to_owned(),
        )
    } else {
        Error::SaveFailed(format!(
            "The backup could not be written, so the book was not changed. {}",
            crate::plain_io(&error)
        ))
    }
}

pub fn resolve_identity(data_dir: &Path, fingerprint: &Fingerprint) -> Fingerprint {
    let path = data_dir.join(ALIASES_DIR).join(hex_of(&fingerprint.0));
    fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_hex(text.trim()))
        .map_or(*fingerprint, |root| Fingerprint(root.0))
}

pub(crate) fn record_alias(
    data_dir: &Path,
    new: &Fingerprint,
    root: &Fingerprint,
) -> io::Result<()> {
    let dir = data_dir.join(ALIASES_DIR);
    let path = dir.join(hex_of(&new.0));
    if new == root || path.exists() {
        return Ok(());
    }
    fs::create_dir_all(&dir)?;
    let temp = TempFile::beside(&path)?;
    temp.file().write_all(hex_of(&root.0).as_bytes())?;
    temp.file().sync_all()?;
    temp.replace(&path)?;
    sync_dir(&dir)?;
    sync_dir(data_dir)?;
    hit(Step::AliasWritten)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct BackupLedger {
    pub(crate) base: FileHash,
    pub(crate) head: FileHash,
    pub(crate) pending_head: Option<FileHash>,
    pub(crate) created_from: PathBuf,
    pub(crate) created_at: u64,
}

impl BackupLedger {
    fn parse(text: &str) -> Option<BackupLedger> {
        let value: Value = serde_json::from_str(text).ok()?;
        if value["version"].as_u64()? != LEDGER_VERSION {
            return None;
        }
        Some(BackupLedger {
            base: parse_hex(value["base"].as_str()?)?,
            head: parse_hex(value["head"].as_str()?)?,
            pending_head: match &value["pending_head"] {
                Value::Null => None,
                pending => Some(parse_hex(pending.as_str()?)?),
            },
            created_from: PathBuf::from(value["created_from"].as_str()?),
            created_at: value["created_at"].as_u64()?,
        })
    }

    fn to_json(&self) -> String {
        json!({
            "version": LEDGER_VERSION,
            "base": hex(&self.base),
            "head": hex(&self.head),
            "pending_head": self.pending_head.as_ref().map(hex),
            "created_from": self.created_from.to_string_lossy(),
            "created_at": self.created_at,
        })
        .to_string()
    }

    fn matches(&self, current: FileHash) -> bool {
        self.head == current || self.pending_head == Some(current)
    }
}

pub(crate) struct Slot {
    dir: PathBuf,
    pub(crate) ledger: BackupLedger,
}

impl Slot {
    pub(crate) fn backup_path(&self) -> PathBuf {
        self.dir.join(BACKUP_FILE)
    }

    pub(crate) fn is_stored(&self) -> bool {
        self.dir.join(LEDGER_FILE).is_file()
    }

    pub(crate) fn open_verified(&self) -> Result<File, Error> {
        let refused = || {
            Error::NotEditable(
                "The backup for this book failed its check, so it will not be used.".to_owned(),
            )
        };
        let file = File::open(self.backup_path()).map_err(|_| refused())?;
        if hash_of(&file).map_err(|_| refused())? != self.ledger.base {
            return Err(refused());
        }
        Ok(file)
    }

    fn store(&self, stage: LedgerWrite) -> io::Result<()> {
        let path = self.dir.join(LEDGER_FILE);
        let temp = TempFile::beside(&path)?;
        temp.file().write_all(self.ledger.to_json().as_bytes())?;
        hit(Step::LedgerTempWritten(stage))?;
        sync_file(temp.file(), Step::LedgerSynced(stage))?;
        temp.replace(&path)?;
        hit(Step::LedgerRenamed(stage))?;
        sync_dir(&self.dir)?;
        hit(Step::LedgerDirSynced(stage))
    }

    pub(crate) fn set_pending(&mut self, hash: FileHash) -> io::Result<()> {
        self.ledger.pending_head = Some(hash);
        self.store(LedgerWrite::Pending)
    }

    pub(crate) fn clear_pending(&mut self) -> io::Result<()> {
        self.ledger.pending_head = None;
        self.store(LedgerWrite::Head)
    }

    fn deleted_dir(&self) -> PathBuf {
        self.dir.join(DELETED_DIR)
    }

    pub(crate) fn deleted_records(&self) -> Vec<DeletedRecord> {
        let Ok(entries) = fs::read_dir(self.deleted_dir()) else {
            return Vec::new();
        };
        let mut records: Vec<DeletedRecord> = entries
            .filter_map(|entry| fs::read(entry.ok()?.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();
        records.sort_by_key(|record| record.chapter.id);
        records
    }

    pub(crate) fn write_deleted(&self, record: &DeletedRecord) -> io::Result<()> {
        let dir = self.deleted_dir();
        fs::create_dir_all(&dir)?;
        let name = format!("{}.json", record.chapter.id.0);
        let path = dir.join(name);
        let temp = TempFile::beside(&path)?;
        temp.file().write_all(&serde_json::to_vec(record)?)?;
        sync_file(temp.file(), Step::DeletedSynced)?;
        temp.replace(&path)?;
        sync_dir(&dir)?;
        sync_dir(&self.dir)
    }

    pub(crate) fn remove_deleted(&self, id: DeletedId) -> io::Result<()> {
        fs::remove_file(self.deleted_dir().join(format!("{}.json", id.0)))
    }

    pub(crate) fn clear_deleted(&self) -> io::Result<()> {
        match fs::remove_dir_all(self.deleted_dir()) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub(crate) fn settle(&mut self, hash: FileHash) -> io::Result<()> {
        self.ledger.head = hash;
        self.ledger.pending_head = None;
        self.store(LedgerWrite::Head)
    }
}

pub(crate) struct BookData {
    root: PathBuf,
    data_dir: PathBuf,
}

impl BookData {
    pub(crate) fn new(data_dir: &Path, key: BookKey) -> BookData {
        BookData {
            root: data_dir.join("books").join(format!("{:032x}", key.0)),
            data_dir: data_dir.to_owned(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn lock(&self) -> Result<File, Error> {
        fs::create_dir_all(&self.root).map_err(backup_failed)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join("commit.lock"))
            .map_err(backup_failed)?;
        for _ in 0..LOCK_ATTEMPTS {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) => std::thread::sleep(LOCK_RETRY),
                Err(fs::TryLockError::Error(error)) => return Err(backup_failed(error)),
            }
        }
        Err(Error::NotEditable(
            "Another Jadeslip window is saving this book. Try again in a moment.".to_owned(),
        ))
    }

    fn slot_dirs(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.backups()) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect();
        dirs.sort();
        dirs
    }

    fn slots(&self) -> Vec<Slot> {
        self.slot_dirs()
            .into_iter()
            .filter_map(|dir| {
                let text = fs::read_to_string(dir.join(LEDGER_FILE)).ok()?;
                let ledger = BackupLedger::parse(&text)?;
                let named = dir.file_name()?.to_str()? == hex(&ledger.base);
                named.then_some(Slot { dir, ledger })
            })
            .collect()
    }

    pub(crate) fn has_any_slot(&self) -> bool {
        !self.slot_dirs().is_empty()
    }

    pub(crate) fn recover(&self, current: &File) -> io::Result<u32> {
        let mut settled = 0;
        let mut current_hash = None;
        for dir in self.slot_dirs() {
            for folder in [dir.clone(), dir.join(DELETED_DIR)] {
                let Ok(entries) = fs::read_dir(&folder) else {
                    continue;
                };
                for entry in entries {
                    let entry = entry?;
                    if entry.file_name().to_string_lossy().ends_with(TEMP_SUFFIX) {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
        }
        for mut slot in self.slots() {
            let Some(pending) = slot.ledger.pending_head else {
                continue;
            };
            let hash = match current_hash {
                Some(hash) => hash,
                None => *current_hash.insert(hash_of(current)?),
            };
            if hash == pending {
                slot.settle(pending)?;
                settled += 1;
            } else if hash == slot.ledger.head {
                slot.clear_pending()?;
                settled += 1;
            }
        }
        Ok(settled)
    }

    pub(crate) fn find_valid(&self, current: FileHash) -> Option<Slot> {
        let mut candidates: Vec<Slot> = self
            .slots()
            .into_iter()
            .filter(|slot| slot.ledger.matches(current))
            .collect();
        candidates.sort_by_key(|slot| std::cmp::Reverse(slot.ledger.created_at));
        let mut slot = candidates
            .into_iter()
            .find(|slot| slot.open_verified().is_ok())?;
        slot.ledger.head = current;
        Some(slot)
    }

    pub(crate) fn ensure(&self, book_path: &Path, current: FileHash) -> Result<Slot, Error> {
        match self.find_valid(current) {
            Some(slot) => Ok(slot),
            None => self.create(book_path, current),
        }
    }

    fn create(&self, book_path: &Path, current: FileHash) -> Result<Slot, Error> {
        let dir = self.backups().join(hex(&current));
        fs::create_dir_all(&dir).map_err(backup_failed)?;
        let backup = dir.join(BACKUP_FILE);
        let keep = File::open(&backup)
            .and_then(|file| hash_of(&file))
            .is_ok_and(|hash| hash == current);
        if !keep {
            let temp = TempFile::beside(&backup).map_err(backup_failed)?;
            self.copy_verified(book_path, temp.file(), current)?;
            temp.replace(&backup).map_err(backup_failed)?;
            hit(Step::BackupRenamed).map_err(backup_failed)?;
        }
        self.sync_slot_dirs(&dir).map_err(backup_failed)?;
        hit(Step::BackupDirSynced).map_err(backup_failed)?;
        let slot = Slot {
            dir,
            ledger: BackupLedger {
                base: current,
                head: current,
                pending_head: None,
                created_from: book_path.to_owned(),
                created_at: now(),
            },
        };
        slot.store(LedgerWrite::Created).map_err(backup_failed)?;
        Ok(slot)
    }

    fn copy_verified(&self, from: &Path, mut to: &File, expected: FileHash) -> Result<(), Error> {
        io::copy(&mut File::open(from).map_err(backup_failed)?, &mut to).map_err(backup_failed)?;
        hit(Step::BackupCopied).map_err(backup_failed)?;
        if hash_of(to).map_err(backup_failed)? != expected {
            return Err(Error::ChangedOnDisk);
        }
        sync_file(to, Step::BackupSynced).map_err(backup_failed)
    }

    fn sync_slot_dirs(&self, dir: &Path) -> io::Result<()> {
        for path in dir.ancestors() {
            sync_dir(path)?;
            if path == self.data_dir {
                break;
            }
        }
        Ok(())
    }
}
