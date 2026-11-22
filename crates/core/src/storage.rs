use crate::backup::{BookData, hash_of};
use crate::commit::seam::sync_dir;
use crate::history::{History, Pruned, RecordKind};
use crate::{BookKey, Error, parse_key};
use std::fs::{self, File};
use std::io;
use std::path::Path;

const BOOKS_DIR: &str = "books";
const HISTORY_DIR: &str = "history";
const BACKUPS_DIR: &str = "backups";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pruning {
    Done(Pruned),
    Skipped,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StorageUsage {
    pub total: u64,
    pub books: Vec<BookUsage>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BookUsage {
    pub key: BookKey,
    pub history: u64,
    pub backup: u64,
    pub other: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PointCount {
    pub automatic: u32,
    pub named: u32,
}

fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    fs::read_dir(path).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| tree_bytes(&entry.path()))
            .sum()
    })
}

fn book_usage(root: &Path, key: BookKey) -> BookUsage {
    let history = tree_bytes(&root.join(HISTORY_DIR));
    let backup = tree_bytes(&root.join(BACKUPS_DIR));
    BookUsage {
        key,
        history,
        backup,
        other: tree_bytes(root).saturating_sub(history + backup),
    }
}

pub fn usage(data_dir: &Path) -> StorageUsage {
    let mut books: Vec<BookUsage> = fs::read_dir(data_dir.join(BOOKS_DIR))
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let key = parse_key(entry.file_name().to_str()?)?;
                    entry
                        .file_type()
                        .ok()?
                        .is_dir()
                        .then(|| book_usage(&entry.path(), key))
                })
                .collect()
        })
        .unwrap_or_default();
    books.sort_by_key(|book| book.key);
    StorageUsage {
        total: tree_bytes(data_dir),
        books,
    }
}

pub fn history_points(data_dir: &Path, key: BookKey) -> PointCount {
    let mut history = History::new(BookData::new(data_dir, key).root());
    let Ok(records) = history.records() else {
        return PointCount::default();
    };
    let named = records
        .iter()
        .filter(|record| matches!(record.kind, RecordKind::Named { .. }))
        .count() as u32;
    let listed = records.iter().filter(|record| record.listed()).count() as u32;
    PointCount {
        automatic: listed - named,
        named,
    }
}

fn lock_unless_saving(data: &BookData) -> Result<Option<File>, Error> {
    match data.lock() {
        Ok(lock) => Ok(Some(lock)),
        Err(Error::NotEditable(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn prune_closed(
    data_dir: &Path,
    key: BookKey,
    book: &Path,
    cutoff_ms: u64,
    dry_run: bool,
) -> Result<Pruning, Error> {
    let data = BookData::new(data_dir, key);
    if !data.root().join(HISTORY_DIR).exists() {
        return Ok(Pruning::Done(Pruned::default()));
    }
    let Some(_lock) = lock_unless_saving(&data)? else {
        return Ok(Pruning::Skipped);
    };
    if let Some(folder) = book.parent()
        && sync_dir(folder).is_err()
    {
        return Ok(Pruning::Skipped);
    }
    let Ok(current) = File::open(book).and_then(|file| hash_of(&file)) else {
        return Ok(Pruning::Skipped);
    };
    History::new(data.root())
        .prune(current, cutoff_ms, dry_run)
        .map(Pruning::Done)
}

pub fn delete_backups(data_dir: &Path, key: BookKey) -> Result<u64, Error> {
    let data = BookData::new(data_dir, key);
    if !data.root().join(BACKUPS_DIR).exists() {
        return Ok(0);
    }
    let _lock = data.lock()?;
    remove_backups(&data)
}

pub(crate) fn remove_backups(data: &BookData) -> Result<u64, Error> {
    let backups = data.root().join(BACKUPS_DIR);
    let bytes = tree_bytes(&backups);
    let removed = match fs::remove_dir_all(&backups) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => sync_dir(data.root()),
    };
    removed.map_err(|error| {
        Error::Book(format!(
            "The original backup could not be deleted. {}",
            crate::plain_io(&error)
        ))
    })?;
    Ok(bytes)
}
