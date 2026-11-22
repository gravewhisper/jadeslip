use crate::external::reveal;
use crate::session::{Session, data_dir, guarded};
use jadeslip_core::storage::{self, Pruning};
use jadeslip_core::{
    Book, BookKey, Keep, ShelfEntry, TempFile, dictionaries_dir, installed, key_hex, parse_key,
    plain_io,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::State;

const STAMP_FILE: &str = "cleanup.json";
const SWEEP_DELAY: Duration = Duration::from_secs(10);
const SWEEP_EVERY_MS: u64 = 24 * 60 * 60 * 1000;
const ENGLISH_CODE: &str = "en_US";
const ENGLISH_WORDS: &str = "spelling-words.txt";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    data_dir: String,
    total: u64,
    books: Vec<BookStorage>,
    last_cleanup: Option<u64>,
    last_freed: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BookStorage {
    key: String,
    title: String,
    author: String,
    on_shelf: bool,
    open: bool,
    book: u64,
    history: u64,
    backup: u64,
    other: u64,
    points: u32,
    named: u32,
}

#[derive(Serialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Removed {
    points: u32,
    books: u32,
    bytes: u64,
}

#[derive(Serialize)]
pub struct Freed {
    bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
struct Stamp {
    at: u64,
    freed: u64,
    points: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Folder {
    Data,
    Dictionaries,
}

#[derive(Serialize)]
pub struct Dictionaries {
    folder: String,
    languages: Vec<Dictionary>,
}

#[derive(Serialize)]
struct Dictionary {
    code: String,
    name: Option<String>,
    bundled: bool,
    words: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenBook {
    Included,
    Skipped,
}

#[cfg(feature = "e2e")]
fn clock_ahead_ms() -> u64 {
    crate::e2e::clock_ahead_ms()
}

#[cfg(not(feature = "e2e"))]
const fn clock_ahead_ms() -> u64 {
    0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
        .saturating_add(clock_ahead_ms())
}

fn open_book(session: &Session) -> Option<(BookKey, Arc<Book>)> {
    let book = session.current()?.book();
    Some((BookKey::of(&book.identity()), book))
}

fn read_stamp(data: &Path) -> Option<Stamp> {
    serde_json::from_slice(&fs::read(data.join(STAMP_FILE)).ok()?).ok()
}

fn write_stamp(data: &Path, stamp: Stamp) -> io::Result<()> {
    let path = data.join(STAMP_FILE);
    let temp = TempFile::beside(&path)?;
    temp.file().write_all(&serde_json::to_vec(&stamp)?)?;
    temp.replace(&path)
}

fn due(stamp: Option<Stamp>, now: u64) -> bool {
    stamp.is_none_or(|stamp| stamp.at.abs_diff(now) >= SWEEP_EVERY_MS)
}

fn added_words(path: &Path) -> u64 {
    fs::read_to_string(path).map_or(0, |text| {
        text.lines().filter(|line| !line.trim().is_empty()).count() as u64
    })
}

fn clean(session: &Session, cutoff_ms: u64, dry_run: bool, open: OpenBook) -> Removed {
    let Ok(library) = session.library() else {
        return Removed::default();
    };
    let open_now = open_book(session);
    let prune_open = |book: &Book| match open {
        OpenBook::Included => book.prune_history(cutoff_ms, dry_run).ok(),
        OpenBook::Skipped => None,
    };
    let shelf = library.shelf();
    let mut pruned: Vec<_> = shelf
        .iter()
        .filter_map(|(key, entry)| match &open_now {
            Some((open_key, book)) if open_key == key => prune_open(book),
            _ => match storage::prune_closed(
                library.data_dir(),
                *key,
                &entry.path,
                cutoff_ms,
                dry_run,
            ) {
                Ok(Pruning::Done(pruned)) => Some(pruned),
                Ok(Pruning::Skipped) | Err(_) => None,
            },
        })
        .collect();
    if let Some((open_key, book)) = &open_now
        && !shelf.iter().any(|(key, _)| key == open_key)
    {
        pruned.extend(prune_open(book));
    }
    let mut removed = Removed::default();
    for pruned in pruned {
        removed.points += pruned.points;
        let freed = pruned.bytes_before.saturating_sub(pruned.bytes_after);
        removed.books += u32::from(pruned.points > 0 || freed > 0);
        removed.bytes += freed;
    }
    removed
}

fn sweep_if_due(session: &Session, now: u64) {
    let cutoff = session
        .saved
        .prefs()
        .keep_snapshots
        .cutoff_ms(now)
        .unwrap_or(0);
    let Ok(library) = session.library() else {
        return;
    };
    if !due(read_stamp(library.data_dir()), now) {
        return;
    }
    let removed = clean(session, cutoff, false, OpenBook::Skipped);
    let _ = write_stamp(
        library.data_dir(),
        Stamp {
            at: now,
            freed: removed.bytes,
            points: removed.points,
        },
    );
}

pub fn sweep_later(session: Session) {
    std::thread::spawn(move || {
        session.wait();
        std::thread::sleep(SWEEP_DELAY);
        sweep_if_due(&session, now_ms());
    });
}

#[tauri::command(async)]
pub fn storage_info(session: State<'_, Session>) -> Result<StorageInfo, String> {
    guarded(|| {
        let library = session.library()?;
        let data = library.data_dir();
        let open_now = open_book(&session);
        let open = open_now.as_ref().map(|(key, _)| *key);
        let open_names = open_now.map(|(key, book)| {
            let reader = book.read();
            let summary = reader.summary();
            (key, summary.title.clone(), summary.author.clone())
        });
        let usage = storage::usage(data);
        let library_dir = library.library_dir();
        let row = |key: BookKey, entry: Option<&ShelfEntry>, book: u64| {
            let found = usage.books.iter().find(|usage| usage.key == key);
            let count = storage::history_points(data, key);
            let (title, author) = match (entry, &open_names) {
                (Some(entry), _) => (entry.title.clone(), entry.author.clone()),
                (None, Some((open_key, title, author))) if *open_key == key => {
                    (title.clone(), author.clone())
                }
                (None, _) => ("Not on the shelf".to_owned(), None),
            };
            BookStorage {
                key: key_hex(key),
                title,
                author: author.unwrap_or_default(),
                on_shelf: entry.is_some(),
                open: open == Some(key),
                book,
                history: found.map_or(0, |usage| usage.history),
                backup: found.map_or(0, |usage| usage.backup),
                other: found.map_or(0, |usage| usage.other),
                points: count.automatic,
                named: count.named,
            }
        };
        let shelf = library.shelf();
        let mut books: Vec<BookStorage> = shelf
            .iter()
            .map(|(key, entry)| {
                let book = if entry.path.starts_with(&library_dir) {
                    fs::metadata(&entry.path).map_or(0, |meta| meta.len())
                } else {
                    0
                };
                row(*key, Some(entry), book)
            })
            .collect();
        books.extend(
            usage
                .books
                .iter()
                .filter(|usage| !shelf.iter().any(|(key, _)| *key == usage.key))
                .map(|usage| row(usage.key, None, 0)),
        );
        books.sort_by_key(|book| Reverse(book.book + book.history + book.backup + book.other));
        let stamp = read_stamp(data);
        Ok(StorageInfo {
            data_dir: data.display().to_string(),
            total: usage.total,
            books,
            last_cleanup: stamp.map(|stamp| stamp.at),
            last_freed: stamp.map(|stamp| stamp.freed),
        })
    })
}

#[tauri::command(async)]
pub fn storage_reveal(which: Folder) -> Result<(), String> {
    let data = data_dir().map_err(|error| error.user_message())?;
    let folder = match which {
        Folder::Data => data,
        Folder::Dictionaries => dictionaries_dir(&data),
    };
    fs::create_dir_all(&folder)
        .map_err(|error| format!("The folder could not be made. {}", plain_io(&error)))?;
    reveal(&folder);
    Ok(())
}

#[tauri::command(async)]
pub fn storage_preview(session: State<'_, Session>, keep: Keep) -> Result<Removed, String> {
    guarded(|| {
        Ok(keep
            .cutoff_ms(now_ms())
            .map_or_else(Removed::default, |cutoff| {
                clean(&session, cutoff, true, OpenBook::Included)
            }))
    })
}

#[tauri::command(async)]
pub fn storage_clean(session: State<'_, Session>, keep: Keep) -> Result<Removed, String> {
    guarded(|| {
        let now = now_ms();
        let cutoff = keep.cutoff_ms(now).unwrap_or(0);
        let removed = clean(&session, cutoff, false, OpenBook::Included);
        let stamp = Stamp {
            at: now,
            freed: removed.bytes,
            points: removed.points,
        };
        write_stamp(session.library()?.data_dir(), stamp).map_err(|error| {
            format!(
                "The cleanup ran, but its date was not saved. {}",
                plain_io(&error)
            )
        })?;
        Ok(removed)
    })
}

#[tauri::command(async)]
pub fn delete_original_backup(session: State<'_, Session>, key: String) -> Result<Freed, String> {
    guarded(|| {
        let key = parse_key(&key).ok_or("That book is not in the data folder.")?;
        let deleted = match open_book(&session) {
            Some((open, book)) if open == key => book.delete_backups(),
            _ => storage::delete_backups(session.library()?.data_dir(), key),
        };
        let bytes = deleted.map_err(|error| error.user_message())?;
        Ok(Freed { bytes })
    })
}

#[tauri::command(async)]
pub fn dictionaries_info() -> Result<Dictionaries, String> {
    let data = data_dir().map_err(|error| error.user_message())?;
    let folder = dictionaries_dir(&data);
    let mut languages = vec![Dictionary {
        code: ENGLISH_CODE.to_owned(),
        name: None,
        bundled: true,
        words: added_words(&data.join(ENGLISH_WORDS)),
    }];
    languages.extend(installed(&folder).into_iter().map(|code| Dictionary {
        words: added_words(&data.join(format!("spelling-words.{code}.txt"))),
        code,
        name: None,
        bundled: false,
    }));
    Ok(Dictionaries {
        folder: folder.display().to_string(),
        languages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = SWEEP_EVERY_MS;

    fn stamp(at: u64) -> Option<Stamp> {
        Some(Stamp {
            at,
            freed: 0,
            points: 0,
        })
    }

    #[test]
    fn the_page_gets_the_field_names_it_reads() {
        let info = StorageInfo {
            data_dir: "/data".to_owned(),
            total: 9,
            books: vec![BookStorage {
                key: "00000000000000000000000000005150".to_owned(),
                title: "Not on the shelf".to_owned(),
                author: String::new(),
                on_shelf: false,
                open: true,
                book: 6,
                history: 4,
                backup: 3,
                other: 2,
                points: 5,
                named: 1,
            }],
            last_cleanup: None,
            last_freed: Some(0),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "dataDir": "/data",
                "total": 9,
                "books": [{
                    "key": "00000000000000000000000000005150",
                    "title": "Not on the shelf",
                    "author": "",
                    "onShelf": false,
                    "open": true,
                    "book": 6,
                    "history": 4,
                    "backup": 3,
                    "other": 2,
                    "points": 5,
                    "named": 1
                }],
                "lastCleanup": null,
                "lastFreed": 0
            })
        );
        let removed = Removed {
            points: 2,
            books: 1,
            bytes: 300,
        };
        assert_eq!(
            serde_json::to_value(removed).unwrap(),
            serde_json::json!({"points": 2, "books": 1, "bytes": 300})
        );
        let dictionaries = Dictionaries {
            folder: "/data/dictionaries".to_owned(),
            languages: vec![Dictionary {
                code: ENGLISH_CODE.to_owned(),
                name: None,
                bundled: true,
                words: 7,
            }],
        };
        assert_eq!(
            serde_json::to_value(&dictionaries).unwrap(),
            serde_json::json!({
                "folder": "/data/dictionaries",
                "languages": [{"code": "en_US", "name": null, "bundled": true, "words": 7}]
            })
        );
        assert_eq!(
            serde_json::to_value(Freed { bytes: 12 }).unwrap(),
            serde_json::json!({"bytes": 12})
        );
        assert!(matches!(
            serde_json::from_str::<Folder>("\"dictionaries\"").unwrap(),
            Folder::Dictionaries
        ));
        assert!(matches!(
            serde_json::from_str::<Folder>("\"data\"").unwrap(),
            Folder::Data
        ));
        assert_eq!(
            serde_json::from_str::<Keep>("\"days90\"").unwrap(),
            Keep::Days90
        );
    }

    #[test]
    fn a_sweep_is_due_with_no_stamp_or_one_a_day_away() {
        let now = 10 * DAY;
        assert!(due(None, now));
        assert!(!due(stamp(now - DAY + 1), now));
        assert!(due(stamp(now - DAY), now));
        assert!(
            due(stamp(now + 3 * DAY), now),
            "a clock set back does not stop it for good"
        );
    }

    #[test]
    fn the_stamp_reads_back_and_added_words_count_non_empty_lines() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/storage-tests")
            .join(std::process::id().to_string());
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_stamp(&dir), None);
        let written = Stamp {
            at: 1_700_000_000_000,
            freed: 4096,
            points: 3,
        };
        write_stamp(&dir, written).unwrap();
        assert_eq!(read_stamp(&dir), Some(written));
        assert_eq!(
            fs::read_to_string(dir.join(STAMP_FILE)).unwrap(),
            r#"{"at":1700000000000,"freed":4096,"points":3}"#
        );
        fs::write(dir.join("words.txt"), "Aelin\n\nRowan\n  \nManon").unwrap();
        assert_eq!(added_words(&dir.join("words.txt")), 3);
        assert_eq!(added_words(&dir.join("missing.txt")), 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
