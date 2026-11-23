use super::{Book, FileStamp, WriterState};
use crate::backup::{hex, parse_hex};
use crate::edit::BlockEdits;
use crate::library::{now_ms, write_file};
use crate::{ChapterIx, Error, FileHash};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DRAFTS_DIR: &str = "drafts";
const DRAFT_VERSION: u64 = 1;

static NEXT_OWNER: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored<E> {
    version: u64,
    book: String,
    chapter: ChapterIx,
    edits: E,
    saved_at: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DraftFit {
    Fits,
    BookChanged,
    Unreadable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftOffer {
    pub chapter: Option<ChapterIx>,
    pub saved_at: Option<u64>,
    pub fit: DraftFit,
}

#[derive(Default)]
pub(super) struct Drafts {
    own: Option<Owner>,
    offered: Option<String>,
}

struct Owner {
    id: String,
    lock_path: PathBuf,
    _lock: File,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

enum Found {
    Absent,
    Unreadable,
    Stored(Stored<BlockEdits>),
}

fn found(path: &Path) -> Found {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Found::Absent,
        Err(_) => return Found::Unreadable,
    };
    match serde_json::from_slice::<Stored<BlockEdits>>(&bytes) {
        Ok(stored) if stored.version == DRAFT_VERSION => Found::Stored(stored),
        _ => Found::Unreadable,
    }
}

fn json_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn lock_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.lock"))
}

#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), fs::metadata(path)) {
        (Ok(held), Ok(named)) => held.dev() == named.dev() && held.ino() == named.ino(),
        _ => false,
    }
}

#[cfg(windows)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileInformationByHandle,
    };
    fn identity(file: &File) -> Option<(u32, u32, u32)> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
        (ok != 0).then_some((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    let named = fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path);
    match (identity(file), named.ok().as_ref().and_then(identity)) {
        (Some(held), Some(named)) => held == named,
        _ => false,
    }
}

fn claim(dir: &Path) -> io::Result<Owner> {
    fs::create_dir_all(dir)?;
    loop {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let id = format!(
            "{:x}-{nanos:x}-{:x}",
            std::process::id(),
            NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
        );
        let lock_path = lock_of(dir, &id);
        let lock = match File::options()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        lock.lock()?;
        if same_file(&lock, &lock_path) {
            return Ok(Owner {
                id,
                lock_path,
                _lock: lock,
            });
        }
    }
}

enum Holder {
    Alive,
    Gone(Option<File>),
}

fn holder(dir: &Path, id: &str) -> Holder {
    let lock = match File::options().write(true).open(lock_of(dir, id)) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Holder::Gone(None),
        Err(_) => return Holder::Alive,
    };
    match lock.try_lock() {
        Ok(()) => Holder::Gone(Some(lock)),
        Err(_) => Holder::Alive,
    }
}

fn remove_quietly(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn sweep_temps(dir: &Path, id: &str) {
    let prefix = format!("{id}.json.");
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with(&prefix) && file.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn forget(dir: &Path, id: &str) -> io::Result<()> {
    let Holder::Gone(_held) = holder(dir, id) else {
        return Ok(());
    };
    sweep_temps(dir, id);
    remove_quietly(&json_of(dir, id))?;
    remove_quietly(&lock_of(dir, id))
}

fn left_behind(dir: &Path, own: Option<&str>) -> Vec<(String, Found)> {
    let mut ids: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let id = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".lock"))?;
            (Some(id) != own).then(|| id.to_owned())
        })
        .collect();
    ids.sort();
    ids.dedup();
    let mut kept = Vec::new();
    for id in ids {
        let Holder::Gone(_held) = holder(dir, &id) else {
            continue;
        };
        sweep_temps(dir, &id);
        match found(&json_of(dir, &id)) {
            Found::Absent => {
                let _ = fs::remove_file(lock_of(dir, &id));
            }
            draft => kept.push((id, draft)),
        }
    }
    kept.sort_by_key(|(_, draft)| match draft {
        Found::Stored(stored) => std::cmp::Reverse(stored.saved_at),
        _ => std::cmp::Reverse(0),
    });
    kept
}

fn no_longer_fits() -> Error {
    Error::Book(
        "These unsaved edits were made to a different version of this book, so they were not applied."
            .to_owned(),
    )
}

impl Book {
    fn drafts_dir(&self) -> PathBuf {
        self.data.root().join(DRAFTS_DIR)
    }

    pub(super) fn file_hash(&self, state: &mut WriterState) -> Result<FileHash, Error> {
        let meta = fs::metadata(&self.path).map_err(|_| Error::ChangedOnDisk)?;
        if FileStamp::of(&meta) != state.stamp {
            return Err(Error::ChangedOnDisk);
        }
        Ok(state.hash)
    }

    fn fits(&self, state: &mut WriterState, stored: &Stored<BlockEdits>) -> bool {
        let same_file = self
            .file_hash(state)
            .is_ok_and(|hash| parse_hex(&stored.book) == Some(hash));
        same_file && self.read().rev_of(stored.chapter).ok() == Some(stored.edits.base)
    }

    fn own_json(&self, state: &mut WriterState) -> io::Result<PathBuf> {
        let dir = self.drafts_dir();
        if state.drafts.own.is_none() {
            state.drafts.own = Some(claim(&dir)?);
        }
        let own = state.drafts.own.as_ref().map(|own| own.id.as_str());
        Ok(json_of(&dir, own.unwrap_or_default()))
    }

    pub fn write_draft(&self, ix: ChapterIx, edits: &BlockEdits) -> Result<(), Error> {
        let mut state = self.lock_state();
        if edits.changes.is_empty() {
            return Ok(self.remove_own(&state)?);
        }
        if self.read().rev_of(ix)? != edits.base {
            return Err(Error::PlanStale);
        }
        let stored = Stored {
            version: DRAFT_VERSION,
            book: hex(&self.file_hash(&mut state)?),
            chapter: ix,
            edits,
            saved_at: now_ms(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(io::Error::other)?;
        let path = self.own_json(&mut state)?;
        write_file(&path, &bytes)?;
        Ok(())
    }

    pub fn draft_offer(&self) -> Option<DraftOffer> {
        let mut state = self.lock_state();
        let own = state.drafts.own.as_ref().map(|own| own.id.clone());
        let (id, draft) = left_behind(&self.drafts_dir(), own.as_deref())
            .into_iter()
            .next()
            .unzip();
        state.drafts.offered = id;
        match draft? {
            Found::Stored(stored) => {
                let fit = if self.fits(&mut state, &stored) {
                    DraftFit::Fits
                } else {
                    DraftFit::BookChanged
                };
                Some(DraftOffer {
                    chapter: Some(stored.chapter),
                    saved_at: Some(stored.saved_at),
                    fit,
                })
            }
            _ => Some(DraftOffer {
                chapter: None,
                saved_at: None,
                fit: DraftFit::Unreadable,
            }),
        }
    }

    pub fn recover_draft(&self) -> Result<(ChapterIx, BlockEdits), Error> {
        let mut state = self.lock_state();
        let dir = self.drafts_dir();
        let Some(id) = state.drafts.offered.clone() else {
            return Err(no_longer_fits());
        };
        let Holder::Gone(_held) = holder(&dir, &id) else {
            return Err(no_longer_fits());
        };
        let Found::Stored(stored) = found(&json_of(&dir, &id)) else {
            return Err(no_longer_fits());
        };
        if !self.fits(&mut state, &stored) {
            return Err(no_longer_fits());
        }
        let own = self.own_json(&mut state)?;
        fs::rename(json_of(&dir, &id), own).map_err(|_| no_longer_fits())?;
        sweep_temps(&dir, &id);
        let _ = fs::remove_file(lock_of(&dir, &id));
        state.drafts.offered = None;
        Ok((stored.chapter, stored.edits))
    }

    pub fn discard_draft(&self) -> Result<(), Error> {
        let mut state = self.lock_state();
        if let Some(id) = state.drafts.offered.take() {
            forget(&self.drafts_dir(), &id)?;
        }
        Ok(())
    }

    pub fn drop_draft(&self) -> Result<(), Error> {
        Ok(self.remove_own(&self.lock_state())?)
    }

    pub(super) fn drop_draft_locked(&self, state: &WriterState) {
        let _ = self.remove_own(state);
    }

    fn remove_own(&self, state: &WriterState) -> io::Result<()> {
        let Some(own) = &state.drafts.own else {
            return Ok(());
        };
        let dir = self.drafts_dir();
        sweep_temps(&dir, &own.id);
        remove_quietly(&json_of(&dir, &own.id))
    }
}

#[cfg(all(test, unix))]
mod tests;
