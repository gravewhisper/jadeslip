use super::{Library, LIBRARY_FILE, Records, ShelfEntry, now_ms, read_capped, write_file};
use crate::backup::{hash_of, resolve_identity};
use crate::book::Reader;
use crate::commit::FileStamp;
use crate::commit::seam::{Step, hit, sync_dir, sync_file};
use crate::{BookKey, DEFAULT_LIMITS, Error, FileHash};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const LIBRARY_DIR: &str = "library";
const PRE_MANAGED_FILE: &str = "library.json.pre-managed";
const STAGE_MARK: &str = ".adding-";
const STAGE_SUFFIX: &str = ".tmp";
const COPY_CHUNK: usize = 1 << 20;
const TITLE_CHARS: usize = 80;
const TITLE_BYTES: usize = 160;
const KEY_CHARS: usize = 8;
const MAX_NAMES: u32 = 100;

static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddOutcome {
    Added,
    AlreadyOnShelf,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShelfAdd {
    pub key: BookKey,
    pub title: String,
    pub outcome: AddOutcome,
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Migration {
    pub total: usize,
    pub done: usize,
    pub copied: usize,
    pub left: usize,
    pub failed: Vec<String>,
}

enum Adopted {
    Copied,
    Left,
}

struct StageFile(PathBuf);

impl Drop for StageFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Staged {
    file: StageFile,
    hash: FileHash,
    key: BookKey,
    title: String,
}

fn safe_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut name = String::new();
    for c in cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        if name.chars().count() >= TITLE_CHARS || name.len() + c.len_utf8() > TITLE_BYTES {
            break;
        }
        name.push(c);
    }
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if name.is_empty() {
        "Book".to_owned()
    } else {
        name.to_owned()
    }
}

fn copy_name(title: &str, key: BookKey, attempt: u32) -> String {
    let tag = &super::key_hex(key)[..KEY_CHARS];
    match attempt {
        0 => format!("{title} - {tag}.epub"),
        n => format!("{title} - {tag} ({}).epub", n + 1),
    }
}

fn same_bytes(path: &Path, hash: FileHash) -> bool {
    File::open(path)
        .and_then(|file| hash_of(&file))
        .is_ok_and(|found| found == hash)
}

fn stage_owner_gone(name: &str) -> bool {
    let Some(pid) = name
        .strip_prefix(STAGE_MARK)
        .and_then(|rest| rest.split('-').next())
        .and_then(|pid| pid.parse::<u32>().ok())
    else {
        return false;
    };
    pid != std::process::id()
        && Path::new("/proc/self").exists()
        && !Path::new("/proc").join(pid.to_string()).exists()
}

impl Library {
    pub fn library_dir(&self) -> PathBuf {
        self.data_dir.join(LIBRARY_DIR)
    }

    pub fn is_managed(&self, path: &Path) -> bool {
        let Some(parent) = path.parent() else {
            return false;
        };
        let dir = self.library_dir();
        parent == dir || fs::canonicalize(&dir).is_ok_and(|real| parent == real)
    }

    fn shelved_copy(&self, key: BookKey) -> Option<ShelfEntry> {
        self.entry(key)
            .filter(|entry| self.is_managed(&entry.path) && entry.path.is_file())
    }

    fn kept_copy(&self, key: BookKey) -> Option<PathBuf> {
        self.stashed(key)
            .map(|entry| entry.path)
            .filter(|path| self.is_managed(path) && path.is_file())
            .filter(|path| self.key_of_file(path).is_ok_and(|found| found == key))
    }

    fn stage_path(&self) -> PathBuf {
        self.library_dir().join(format!(
            "{STAGE_MARK}{}-{}{STAGE_SUFFIX}",
            std::process::id(),
            STAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn stage(&self, file: StageFile, hash: FileHash) -> Result<Staged, Error> {
        let reader = Reader::open(File::open(&file.0)?, DEFAULT_LIMITS)?;
        let summary = reader.summary();
        Ok(Staged {
            key: BookKey::of(&resolve_identity(&self.data_dir, &summary.fingerprint)),
            title: summary.title.clone(),
            file,
            hash,
        })
    }

    fn stage_copy(&self, source: &Path) -> Result<Staged, Error> {
        fs::create_dir_all(self.library_dir())?;
        let mut from = File::open(source)?;
        let before = from.metadata()?;
        let file = StageFile(self.stage_path());
        let mut out = File::options()
            .write(true)
            .create_new(true)
            .open(&file.0)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0; COPY_CHUNK];
        loop {
            let read = match from.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            hasher.update(&buf[..read]);
            out.write_all(&buf[..read])?;
        }
        hit(Step::CopyWritten)?;
        if let Ok(modified) = before.modified() {
            out.set_modified(modified)?;
        }
        sync_file(&out, Step::CopySynced)?;
        drop(out);
        if FileStamp::of(&fs::metadata(source)?) != FileStamp::of(&before) {
            return Err(Error::Book(format!(
                "{} changed while Jadeslip was copying it, so it was not added. Try again.",
                source.display()
            )));
        }
        let hash = FileHash(hasher.finalize().into());
        if !same_bytes(&file.0, hash) {
            return Err(Error::Book(format!(
                "The copy of {} did not match the file, so it was not added.",
                source.display()
            )));
        }
        self.stage(file, hash)
    }

    fn settle(&self, staged: Staged) -> Result<PathBuf, Error> {
        let dir = self.library_dir();
        let title = safe_title(&staged.title);
        let mut settled = None;
        for attempt in 0..MAX_NAMES {
            let path = dir.join(copy_name(&title, staged.key, attempt));
            match fs::hard_link(&staged.file.0, &path) {
                Ok(()) => {
                    hit(Step::CopyLinked)?;
                    settled = Some(path);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if same_bytes(&path, staged.hash) {
                        settled = Some(path);
                        break;
                    }
                }
                Err(_) if path.exists() => {
                    if same_bytes(&path, staged.hash) {
                        settled = Some(path);
                        break;
                    }
                }
                Err(_) => {
                    fs::rename(&staged.file.0, &path)?;
                    hit(Step::CopyLinked)?;
                    settled = Some(path);
                    break;
                }
            }
        }
        let path = settled.ok_or_else(|| {
            Error::Book(format!(
                "Jadeslip's library already holds {MAX_NAMES} different files named for {title}, so nothing was added."
            ))
        })?;
        drop(staged);
        sync_dir(&dir)?;
        hit(Step::CopyDirSynced)?;
        Ok(fs::canonicalize(path)?)
    }

    fn sweep_stages(&self) {
        let Ok(entries) = fs::read_dir(self.library_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(STAGE_SUFFIX) && stage_owner_gone(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn put_on_shelf(
        &self,
        key: BookKey,
        path: &Path,
        own: bool,
    ) -> Result<ShelfAdd, Error> {
        let reader = Reader::open(File::open(path)?, DEFAULT_LIMITS)?;
        let summary = reader.summary();
        let at = now_ms();
        let stash = self.stashed(key);
        let mut outcome = AddOutcome::Added;
        let mut unstashed = false;
        self.change(|records: &mut Records| match records.get_mut(&key) {
            Some(entry) if self.is_managed(&entry.path) && entry.path.is_file() => {
                outcome = AddOutcome::AlreadyOnShelf;
            }
            Some(entry) => entry.path = path.to_owned(),
            None => {
                unstashed = true;
                let mut entry = stash.unwrap_or_else(|| ShelfEntry::fresh(path, summary, own, at));
                entry.path = path.to_owned();
                entry.refresh_from(summary);
                entry.own |= own;
                entry.touched_at = at;
                records.insert(key, entry);
            }
        })?;
        if unstashed {
            let _ = fs::remove_file(self.stash_path(key));
        }
        Ok(ShelfAdd {
            key,
            title: self.entry(key).map_or_else(|| summary.title.clone(), |entry| entry.title),
            outcome,
        })
    }

    pub fn add_to_shelf(&self, source: &Path) -> Result<ShelfAdd, Error> {
        let key = self.key_of_file(source)?;
        if let Some(entry) = self.shelved_copy(key) {
            return Ok(ShelfAdd {
                key,
                title: entry.title,
                outcome: AddOutcome::AlreadyOnShelf,
            });
        }
        let path = match self.kept_copy(key) {
            Some(path) => path,
            None => {
                let staged = self.stage_copy(source)?;
                if staged.key != key {
                    return Err(Error::Book(format!(
                        "{} changed while Jadeslip was copying it, so it was not added. Try again.",
                        source.display()
                    )));
                }
                self.settle(staged)?
            }
        };
        self.put_on_shelf(key, &path, false)
    }

    pub(super) fn create_in_library(
        &self,
        meta: &crate::manage::NewBookMeta,
    ) -> Result<PathBuf, Error> {
        fs::create_dir_all(self.library_dir())?;
        let file = StageFile(self.stage_path());
        let book = crate::manage::new_book(&file.0, &self.data_dir, meta, DEFAULT_LIMITS)?;
        drop(book);
        let hash = hash_of(&File::open(&file.0)?)?;
        let staged = self.stage(file, hash)?;
        let key = staged.key;
        let path = self.settle(staged)?;
        self.put_on_shelf(key, &path, true)?;
        Ok(path)
    }

    fn keep_pre_managed(&self) -> Result<(), Error> {
        let backup = self.data_dir.join(PRE_MANAGED_FILE);
        if backup.exists() {
            return Ok(());
        }
        let Some(bytes) = read_capped(&self.data_dir.join(LIBRARY_FILE))? else {
            return Ok(());
        };
        write_file(&backup, &bytes)?;
        Ok(())
    }

    fn adopt(&self, key: BookKey, source: &Path) -> Result<Adopted, Error> {
        if !source.is_file() || self.key_of_file(source).ok() != Some(key) {
            return Ok(Adopted::Left);
        }
        let staged = self.stage_copy(source)?;
        if staged.key != key {
            return Ok(Adopted::Left);
        }
        let copy = self.settle(staged)?;
        self.change(|records| {
            if let Some(entry) = records.get_mut(&key).filter(|entry| entry.path == source) {
                entry.path = copy;
            }
        })?;
        Ok(Adopted::Copied)
    }

    pub fn migrate(&self, mut progress: impl FnMut(&Migration)) -> Migration {
        self.sweep_stages();
        let pending: Vec<(BookKey, ShelfEntry)> = self
            .shelf()
            .into_iter()
            .filter(|(key, entry)| {
                !self.is_managed(&entry.path)
                    && entry.path.is_file()
                    && self.key_of_file(&entry.path).ok() == Some(*key)
            })
            .collect();
        let mut state = Migration {
            total: pending.len(),
            ..Migration::default()
        };
        if pending.is_empty() {
            return state;
        }
        progress(&state);
        if let Err(error) = self.keep_pre_managed() {
            state.failed.push(format!(
                "The shelf could not be backed up before copying, so no book was copied. {}",
                error.user_message()
            ));
            return state;
        }
        for (key, entry) in pending {
            match self.adopt(key, &entry.path) {
                Ok(Adopted::Copied) => state.copied += 1,
                Ok(Adopted::Left) => state.left += 1,
                Err(error) => state
                    .failed
                    .push(format!("{}: {}", entry.title, error.user_message())),
            }
            state.done += 1;
            progress(&state);
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::{copy_name, safe_title};
    use crate::BookKey;

    #[test]
    fn a_copy_is_named_for_its_title_and_key_and_never_for_a_path() {
        let key = BookKey(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        assert_eq!(copy_name("Iron Monsoon", key, 0), "Iron Monsoon - 01234567.epub");
        assert_eq!(copy_name("Book", key, 2), "Book - 01234567 (3).epub");
        assert_eq!(safe_title("../../etc/passwd"), "etc passwd");
        assert_eq!(safe_title("a/b\\c:d*e?f\"g<h>i|j\u{7}k"), "a b c d e f g h i j k");
        assert_eq!(safe_title("  ...  "), "Book");
        assert_eq!(safe_title(".hidden"), "hidden");
        assert_eq!(safe_title("万古神帝"), "万古神帝");
        assert_eq!(safe_title(&"word ".repeat(100)).chars().count(), 79);
        assert_eq!(safe_title(&"字".repeat(200)), "字".repeat(53));
    }
}
