use crate::anchor::Anchor;
use crate::backup::BookData;
use crate::backup::resolve_identity;
use crate::book::Reader;
use crate::chapter::VisibleText;
use crate::commit::seam::{Step, hit, sync_dir, sync_file};
use crate::commit::{Book, FileStamp};
use crate::manage::NewBookMeta;
use crate::spell::dictionaries::SpellChoice;
use crate::temp::TempFile;
use crate::trash::{Trash, Trashed};
use crate::zip::ReadAt;
use crate::{BookKey, ChapterIx, DEFAULT_LIMITS, DomPoint, Error, VisRange};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const LIBRARY_FILE: &str = "library.json";
const MARKS_FILE: &str = "marks.json";
const SPELLING_FILE: &str = "spelling.json";
const COVER_FILE: &str = "cover.jpg";
const STASH_FILE: &str = "shelf.json";
const MAX_COVER_FILE_BYTES: u64 = 4 << 20;
const VERSION: u64 = 1;
const MAX_FILE_BYTES: u64 = 64 << 20;
const MAX_NOTE_CHARS: usize = 20_000;
const PASSAGE_CHARS: usize = 240;
const PASSAGE_MIN_CHARS: usize = 60;
const TITLE_LINE_CHARS: usize = 80;

mod managed;

pub use managed::{AddOutcome, Migration, ShelfAdd};

pub struct LibraryConfig {
    pub data_dir: PathBuf,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ResumePoint {
    pub href: String,
    pub point: Option<DomPoint>,
    pub chapter_label: String,
    #[serde(default)]
    pub ordinal: Option<u32>,
    pub progress: f32,
    pub passage: String,
}

mod path_text {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    const RAW_MARK: &str = "raw-bytes:";

    #[cfg(unix)]
    fn raw_bytes(path: &Path) -> Vec<u8> {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }

    #[cfg(not(unix))]
    fn raw_bytes(path: &Path) -> Vec<u8> {
        path.to_string_lossy().into_owned().into_bytes()
    }

    #[cfg(unix)]
    fn from_raw(bytes: Vec<u8>) -> PathBuf {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    }

    #[cfg(not(unix))]
    fn from_raw(bytes: Vec<u8>) -> PathBuf {
        PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(super) fn serialize<S: Serializer>(path: &Path, out: S) -> Result<S::Ok, S::Error> {
        match path.to_str() {
            Some(text) if !text.starts_with(RAW_MARK) => out.serialize_str(text),
            _ => out.serialize_str(&format!(
                "{RAW_MARK}{}",
                crate::manage::hex_encode(&raw_bytes(path))
            )),
        }
    }

    pub(super) fn deserialize<'d, D: Deserializer<'d>>(input: D) -> Result<PathBuf, D::Error> {
        let text = String::deserialize(input)?;
        match text.strip_prefix(RAW_MARK) {
            Some(hex) => crate::manage::hex_decode(hex)
                .map(from_raw)
                .ok_or_else(|| serde::de::Error::custom("a bad raw path")),
            None => Ok(PathBuf::from(text)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverFound {
    Absent,
    Thumbnail,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CoverStamp {
    pub file: String,
    pub found: CoverFound,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WordStamp {
    pub file: String,
    pub count: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Removal {
    pub title: String,
    pub data_bytes: u64,
    pub file_bytes: Option<u64>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ShelfEntry {
    #[serde(with = "path_text")]
    pub path: PathBuf,
    pub title: String,
    pub author: Option<String>,
    pub inflated_spine_bytes: u64,
    pub linear_chapters: u32,
    pub own: bool,
    pub read_point: Option<ResumePoint>,
    pub write_point: Option<ResumePoint>,
    pub added_at: u64,
    pub touched_at: u64,
    #[serde(default)]
    pub cover: Option<CoverStamp>,
    #[serde(default)]
    pub words: Option<WordStamp>,
}

impl ShelfEntry {
    fn fresh(path: &Path, summary: &crate::BookSummary, own: bool, at: u64) -> ShelfEntry {
        ShelfEntry {
            path: path.to_owned(),
            title: summary.title.clone(),
            author: summary.author.clone(),
            inflated_spine_bytes: summary.inflated_spine_bytes,
            linear_chapters: summary.linear_chapters,
            own,
            read_point: None,
            write_point: None,
            added_at: at,
            touched_at: at,
            cover: None,
            words: None,
        }
    }

    fn refresh_from(&mut self, summary: &crate::BookSummary) {
        self.title.clone_from(&summary.title);
        self.author.clone_from(&summary.author);
        self.inflated_spine_bytes = summary.inflated_spine_bytes;
        self.linear_chapters = summary.linear_chapters;
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Mark {
    pub id: u64,
    pub anchor: Anchor,
    pub note: Option<String>,
    pub created_at: u64,
}

type Records = BTreeMap<BookKey, ShelfEntry>;

pub struct Library {
    data_dir: PathBuf,
    records: Mutex<Records>,
    covers: Mutex<()>,
    words: Mutex<()>,
}

const fn is_space(c: char) -> bool {
    matches!(c, '\0'..=' ' | '\u{a0}' | '\u{2000}'..='\u{200b}' | '\u{3000}')
}

pub fn count_words(text: &str) -> u64 {
    let mut words = 0;
    let mut in_word = false;
    for c in text.chars() {
        let space = is_space(c);
        if !space && !in_word {
            words += 1;
        }
        in_word = !space;
    }
    words
}

fn size_of_tree(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path).map_or(0, |entries| {
        entries
            .flatten()
            .map(|entry| size_of_tree(&entry.path()))
            .sum()
    })
}

fn put_back(moved: Vec<Trashed>, error: Error) -> Error {
    let stuck: Vec<String> = moved
        .into_iter()
        .rev()
        .filter_map(|item| {
            let (place, in_trash) = (item.place().to_owned(), item.in_trash().to_owned());
            let failed = item.restore().err()?;
            let why = if failed.kind() == io::ErrorKind::AlreadyExists {
                "Something new is in its place."
            } else {
                crate::plain_io(&failed)
            };
            Some(format!(
                "{} could not be put back. {why} It is still in the trash at {}.",
                place.display(),
                in_trash.display()
            ))
        })
        .collect();
    if stuck.is_empty() {
        return error;
    }
    Error::Book(format!("{} {}", error.user_message(), stuck.join(" ")))
}

fn trash_failed(what: &str, error: &io::Error) -> Error {
    Error::Book(format!(
        "{what} could not be moved to the trash, so nothing was removed. {}",
        crate::plain_io(error)
    ))
}

pub fn key_hex(key: BookKey) -> String {
    format!("{:032x}", key.0)
}

pub fn parse_key(text: &str) -> Option<BookKey> {
    (text.len() == 32 && text.is_ascii())
        .then(|| u128::from_str_radix(text, 16).ok().map(BookKey))
        .flatten()
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn read_capped(path: &Path) -> io::Result<Option<Vec<u8>>> {
    read_at_most(path, MAX_FILE_BYTES)
}

fn read_at_most(path: &Path, most: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(most + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > most {
        return Err(io::Error::other("the file is too large"));
    }
    Ok(Some(bytes))
}

pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let temp = TempFile::beside(path)?;
    temp.file().write_all(bytes)?;
    sync_file(temp.file(), Step::LibrarySynced)?;
    temp.replace(path)?;
    sync_dir(dir)?;
    hit(Step::LibraryDirSynced)
}

fn parse_records(bytes: &[u8]) -> Option<(Records, bool)> {
    let root: Value = serde_json::from_slice(bytes).ok()?;
    let books = root.get("books")?.as_object()?;
    let records: Records = books
        .iter()
        .filter_map(|(key, entry)| {
            Some((parse_key(key)?, serde_json::from_value(entry.clone()).ok()?))
        })
        .collect();
    let lost_some = records.len() != books.len();
    Some((records, lost_some))
}

fn damaged_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{LIBRARY_FILE}.damaged-{}", now_ms()))
}

fn set_aside(path: &Path) {
    let _ = fs::rename(path, damaged_path(path));
}

fn no_marks(error: impl std::fmt::Display) -> Error {
    Error::Book(format!(
        "The bookmarks for this book could not be read, so nothing was changed. {error}."
    ))
}

fn parse_marks(bytes: &[u8]) -> Result<Vec<Mark>, Error> {
    let root: Value = serde_json::from_slice(bytes).map_err(no_marks)?;
    if root["version"].as_u64() != Some(VERSION) {
        return Err(no_marks("The file is from another version"));
    }
    serde_json::from_value(root["marks"].clone()).map_err(no_marks)
}

impl Library {
    pub fn open(config: LibraryConfig) -> Result<Arc<Library>, Error> {
        let data_dir = fs::create_dir_all(&config.data_dir)
            .and_then(|()| fs::canonicalize(&config.data_dir))
            .unwrap_or(config.data_dir);
        let path = data_dir.join(LIBRARY_FILE);
        let records = match read_capped(&path) {
            Ok(None) => Records::new(),
            Ok(Some(bytes)) => match parse_records(&bytes) {
                Some((records, lost_some)) => {
                    if lost_some {
                        let _ = fs::copy(&path, damaged_path(&path));
                    }
                    records
                }
                None => {
                    set_aside(&path);
                    Records::new()
                }
            },
            Err(_) => {
                set_aside(&path);
                Records::new()
            }
        };
        Ok(Arc::new(Library {
            data_dir,
            records: Mutex::new(records),
            covers: Mutex::new(()),
            words: Mutex::new(()),
        }))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn lock(&self) -> MutexGuard<'_, Records> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn library_path(&self) -> PathBuf {
        self.data_dir.join(LIBRARY_FILE)
    }

    fn change(&self, work: impl FnOnce(&mut Records)) -> Result<(), Error> {
        let mut records = self.lock();
        let path = self.library_path();
        if let Ok(Some(bytes)) = read_capped(&path) {
            match parse_records(&bytes) {
                Some((fresh, lost_some)) => {
                    if lost_some {
                        let _ = fs::copy(&path, damaged_path(&path));
                    }
                    *records = fresh;
                }
                None => set_aside(&path),
            }
        }
        work(&mut records);
        let books: BTreeMap<String, &ShelfEntry> = records
            .iter()
            .map(|(key, entry)| (key_hex(*key), entry))
            .collect();
        let books = serde_json::to_value(books).map_err(|error| Error::Book(error.to_string()))?;
        let bytes = serde_json::to_vec(&serde_json::json!({"version": VERSION, "books": books}))
            .map_err(|error| Error::Book(error.to_string()))?;
        write_file(&path, &bytes)?;
        Ok(())
    }

    pub fn shelf(&self) -> Vec<(BookKey, ShelfEntry)> {
        let mut books: Vec<(BookKey, ShelfEntry)> = self
            .lock()
            .iter()
            .map(|(key, entry)| (*key, entry.clone()))
            .collect();
        books.sort_by_key(|(key, entry)| (entry.added_at, *key));
        books
    }

    pub fn entry(&self, key: BookKey) -> Option<ShelfEntry> {
        self.lock().get(&key).cloned()
    }

    pub fn key_at(&self, path: &Path) -> Option<BookKey> {
        self.lock()
            .iter()
            .find(|(_, entry)| entry.path == path)
            .map(|(key, _)| *key)
    }

    pub fn key_of_file(&self, path: &Path) -> Result<BookKey, Error> {
        let reader = Reader::open(File::open(path)?, DEFAULT_LIMITS)?;
        Ok(BookKey::of(&resolve_identity(
            &self.data_dir,
            &reader.summary().fingerprint,
        )))
    }

    pub fn has_backup(&self, key: BookKey) -> bool {
        BookData::new(&self.data_dir, key).has_any_slot()
    }

    pub fn known(&self, key: BookKey) -> Option<ShelfEntry> {
        self.entry(key).or_else(|| self.stashed(key))
    }

    fn stashed(&self, key: BookKey) -> Option<ShelfEntry> {
        read_capped(&self.stash_path(key))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    fn write_stash(&self, key: BookKey, entry: &ShelfEntry) -> Result<(), Error> {
        let bytes = serde_json::to_vec(entry).map_err(|error| Error::Book(error.to_string()))?;
        write_file(&self.stash_path(key), &bytes)?;
        Ok(())
    }

    fn note(&self, book: &Book, remember: bool) -> Result<(), Error> {
        let reader = book.read();
        let summary = reader.summary();
        let key = BookKey::of(&book.identity());
        let at = now_ms();
        if self.entry(key).is_some() {
            return self.change(|records| {
                if let Some(entry) = records.get_mut(&key) {
                    if entry.path == book.path() {
                        entry.refresh_from(summary);
                    }
                    entry.touched_at = at;
                }
            });
        }
        let mut entry = match self.stashed(key) {
            Some(entry) => entry,
            None if remember => ShelfEntry::fresh(book.path(), summary, false, at),
            None => return Ok(()),
        };
        if entry.path == book.path() || !self.is_managed(&entry.path) {
            entry.path = book.path().to_owned();
            entry.refresh_from(summary);
        }
        entry.touched_at = at;
        self.write_stash(key, &entry)
    }

    pub fn open_book(self: &Arc<Self>, path: &Path) -> Result<Book, Error> {
        let book = Book::open_by_fingerprint(path, &self.data_dir, DEFAULT_LIMITS)?;
        let _ = self.note(&book, true);
        Ok(book)
    }

    pub fn create_book(self: &Arc<Self>, meta: &NewBookMeta) -> Result<Book, Error> {
        let path = self.create_in_library(meta)?;
        Book::open_by_fingerprint(&path, &self.data_dir, DEFAULT_LIMITS)
    }

    pub fn refresh(&self, book: &Book) {
        let _ = self.note(book, false);
    }

    pub fn save_resume_point(
        &self,
        key: BookKey,
        writing: bool,
        point: ResumePoint,
    ) -> Result<(), Error> {
        let at = now_ms();
        let place = |entry: &mut ShelfEntry| {
            if writing {
                entry.write_point = Some(point.clone());
            }
            entry.read_point = Some(point.clone());
            entry.touched_at = at;
        };
        if self.entry(key).is_some() {
            return self.change(|records| {
                if let Some(entry) = records.get_mut(&key) {
                    place(entry);
                }
            });
        }
        match self.stashed(key) {
            Some(mut entry) => {
                place(&mut entry);
                self.write_stash(key, &entry)
            }
            None => Err(Error::Book(
                "Jadeslip could not keep the place, because its data folder could not be written."
                    .to_owned(),
            )),
        }
    }

    pub fn set_own(&self, key: BookKey, own: bool) -> Result<(), Error> {
        self.change(|records| {
            if let Some(entry) = records.get_mut(&key) {
                entry.own = own;
            }
        })
    }

    fn stash_path(&self, key: BookKey) -> PathBuf {
        BookData::new(&self.data_dir, key).root().join(STASH_FILE)
    }

    pub fn remove(&self, key: BookKey) -> Result<(), Error> {
        if let Some(entry) = self.entry(key) {
            self.write_stash(key, &entry)?;
        }
        self.change(|records| {
            records.remove(&key);
        })
    }

    fn aliases_of(&self, key: BookKey) -> Vec<PathBuf> {
        fs::read_dir(self.data_dir.join(crate::backup::ALIASES_DIR)).map_or_else(
            |_| Vec::new(),
            |entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        fs::read_to_string(path)
                            .ok()
                            .and_then(|text| crate::backup::parse_hex(text.trim()))
                            .is_some_and(|root| BookKey::of(&crate::Fingerprint(root.0)) == key)
                    })
                    .collect()
            },
        )
    }

    pub fn removal(&self, key: BookKey) -> Option<Removal> {
        let entry = self.entry(key)?;
        let data_bytes = size_of_tree(BookData::new(&self.data_dir, key).root())
            + self
                .aliases_of(key)
                .iter()
                .map(|path| size_of_tree(path))
                .sum::<u64>();
        Some(Removal {
            title: entry.title,
            data_bytes,
            file_bytes: fs::metadata(&entry.path)
                .ok()
                .filter(|meta| meta.is_file() && self.is_managed(&entry.path))
                .map(|meta| meta.len()),
        })
    }

    pub fn remove_with(
        &self,
        key: BookKey,
        data: bool,
        file: bool,
        trash: &Trash,
    ) -> Result<(), Error> {
        let entry = self
            .entry(key)
            .ok_or_else(|| Error::Book("That book is not on the shelf.".to_owned()))?;
        let book_data = BookData::new(&self.data_dir, key);
        let _lock = if data && book_data.root().exists() {
            Some(book_data.lock()?)
        } else {
            None
        };
        let mut moved: Vec<Trashed> = Vec::new();
        if file && entry.path.is_file() && self.is_managed(&entry.path) {
            match trash.put(&entry.path) {
                Ok(item) => moved.push(item),
                Err(error) => return Err(trash_failed("Jadeslip's copy of the EPUB", &error)),
            }
        }
        if data {
            let mut items = vec![book_data.root().to_owned()];
            items.extend(self.aliases_of(key));
            for item in items.iter().filter(|item| fs::symlink_metadata(item).is_ok()) {
                match trash.put(item) {
                    Ok(item) => moved.push(item),
                    Err(error) => {
                        return Err(put_back(
                            moved,
                            trash_failed("Jadeslip's data for this book", &error),
                        ));
                    }
                }
            }
        }
        let removed = if data {
            self.change(|records| {
                records.remove(&key);
            })
        } else {
            self.remove(key)
        };
        removed.map_err(|error| put_back(moved, error))
    }

    fn cover_path(&self, key: BookKey) -> PathBuf {
        BookData::new(&self.data_dir, key).root().join(COVER_FILE)
    }

    pub fn cover_bytes(&self, key: BookKey) -> Option<Vec<u8>> {
        read_at_most(&self.cover_path(key), MAX_COVER_FILE_BYTES)
            .ok()
            .flatten()
    }

    pub fn update_cover(&self, key: BookKey, reader: &Reader<File>) -> Result<bool, Error> {
        if self.entry(key).is_none() {
            return Ok(false);
        }
        let file = FileStamp::of(&reader.source().metadata()?).version();
        self.stamp_cover(key, file, || self.thumbnail_of(key, reader))
    }

    fn thumbnail_of(&self, key: BookKey, reader: &Reader<File>) -> Option<Vec<u8>> {
        let identity = resolve_identity(&self.data_dir, &reader.summary().fingerprint);
        (BookKey::of(&identity) == key)
            .then(|| reader.cover_thumbnail())
            .flatten()
    }

    pub fn fill_covers(&self) -> usize {
        self.shelf()
            .into_iter()
            .filter(|(_, entry)| entry.cover.is_none())
            .filter(|(key, entry)| self.fill_cover(*key, &entry.path).unwrap_or(false))
            .count()
    }

    fn fill_cover(&self, key: BookKey, path: &Path) -> Result<bool, Error> {
        let file = File::open(path)?;
        let stamp = FileStamp::of(&file.metadata()?).version();
        self.stamp_cover(key, stamp, || {
            self.thumbnail_of(key, &Reader::open(file, DEFAULT_LIMITS).ok()?)
        })
    }

    fn stamp_cover(
        &self,
        key: BookKey,
        file: String,
        thumbnail: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Result<bool, Error> {
        let _covers = self
            .covers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self.entry(key).and_then(|entry| entry.cover);
        if current.is_some_and(|cover| cover.file == file) {
            return Ok(false);
        }
        let path = self.cover_path(key);
        let found = match thumbnail() {
            Some(bytes) => {
                write_file(&path, &bytes)?;
                CoverFound::Thumbnail
            }
            None => {
                let _ = fs::remove_file(&path);
                CoverFound::Absent
            }
        };
        self.change(|records| {
            if let Some(entry) = records.get_mut(&key) {
                entry.cover = Some(CoverStamp { file, found });
            }
        })?;
        Ok(true)
    }

    pub fn export_copy_of(&self, key: BookKey, dest: &Path) -> Result<(), Error> {
        let entry = self
            .entry(key)
            .ok_or_else(|| Error::Book("That book is not on the shelf.".to_owned()))?;
        let book = Book::open_by_fingerprint(&entry.path, &self.data_dir, DEFAULT_LIMITS)?;
        if BookKey::of(&book.identity()) != key {
            return Err(Error::Book(format!(
                "{} now holds a different book, so no copy was made.",
                entry.path.display()
            )));
        }
        book.export_copy(dest)
    }

    pub fn update_words(&self, key: BookKey, reader: &Reader<File>) -> Result<bool, Error> {
        let file = FileStamp::of(&reader.source().metadata()?).version();
        let _words = self
            .words
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = self.entry(key).filter(|entry| entry.own) else {
            return Ok(false);
        };
        if entry.words.is_some_and(|words| words.file == file) {
            return Ok(false);
        }
        let newer = fs::metadata(&entry.path)
            .ok()
            .is_some_and(|meta| FileStamp::of(&meta).version() != file);
        if newer {
            return Ok(false);
        }
        let count = reader.word_count();
        self.change(|records| {
            if let Some(entry) = records.get_mut(&key).filter(|entry| entry.own) {
                entry.words = Some(WordStamp { file, count });
            }
        })?;
        Ok(true)
    }

    pub fn update_words_from_file(&self, key: BookKey) -> Result<bool, Error> {
        let Some(entry) = self.entry(key).filter(|entry| entry.own) else {
            return Ok(false);
        };
        let reader = Reader::open(File::open(&entry.path)?, DEFAULT_LIMITS)?;
        let identity = resolve_identity(&self.data_dir, &reader.summary().fingerprint);
        if BookKey::of(&identity) != key {
            return Ok(false);
        }
        self.update_words(key, &reader)
    }

    fn marks_path(&self, key: BookKey) -> PathBuf {
        BookData::new(&self.data_dir, key).root().join(MARKS_FILE)
    }

    fn write_marks(&self, key: BookKey, marks: &[Mark]) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&serde_json::json!({"version": VERSION, "marks": marks}))
            .map_err(|error| Error::Book(error.to_string()))?;
        write_file(&self.marks_path(key), &bytes)?;
        Ok(())
    }

    pub fn marks(&self, key: BookKey) -> Result<Vec<Mark>, Error> {
        let _records = self.lock();
        self.read_marks(key)
    }

    fn read_marks(&self, key: BookKey) -> Result<Vec<Mark>, Error> {
        match read_capped(&self.marks_path(key)).map_err(no_marks)? {
            Some(bytes) => parse_marks(&bytes),
            None => Ok(Vec::new()),
        }
    }

    pub fn add_mark(
        &self,
        key: BookKey,
        anchor: Anchor,
        note: Option<String>,
    ) -> Result<Mark, Error> {
        let _records = self.lock();
        let mut marks = self.read_marks(key)?;
        let mark = Mark {
            id: marks
                .iter()
                .map(|mark| mark.id)
                .max()
                .map_or(1, |id| id + 1),
            anchor,
            note: note
                .map(|text| text.trim().chars().take(MAX_NOTE_CHARS).collect::<String>())
                .filter(|text| !text.is_empty()),
            created_at: now_ms(),
        };
        marks.push(mark.clone());
        self.write_marks(key, &marks)?;
        Ok(mark)
    }

    pub fn delete_mark(&self, key: BookKey, id: u64) -> Result<(), Error> {
        let _records = self.lock();
        let mut marks = self.read_marks(key)?;
        let before = marks.len();
        marks.retain(|mark| mark.id != id);
        if marks.len() != before {
            self.write_marks(key, &marks)?;
        }
        Ok(())
    }

    fn spelling_path(&self, key: BookKey) -> PathBuf {
        BookData::new(&self.data_dir, key)
            .root()
            .join(SPELLING_FILE)
    }

    pub fn spelling_choice(&self, key: BookKey) -> SpellChoice {
        read_capped(&self.spelling_path(key))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|root| root["version"].as_u64() == Some(VERSION))
            .and_then(|root| serde_json::from_value(root["choice"].clone()).ok())
            .unwrap_or_default()
    }

    pub fn set_spelling_choice(&self, key: BookKey, choice: &SpellChoice) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&serde_json::json!({"version": VERSION, "choice": choice}))
            .map_err(|error| Error::Book(error.to_string()))?;
        write_file(&self.spelling_path(key), &bytes)?;
        Ok(())
    }
}

fn sentence_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut best = None;
    for (at, c) in text.char_indices() {
        if !matches!(
            c,
            '.' | '!' | '?' | '"' | '\u{201D}' | '\u{3002}' | '\u{FF01}' | '\u{FF1F}'
        ) {
            continue;
        }
        let end = at + c.len_utf8();
        let closes = end == text.len() || bytes[end] == b' ' || c >= '\u{3000}';
        if closes && text[..end].chars().count() >= PASSAGE_MIN_CHARS {
            best = Some(end);
        }
    }
    best
}

fn word_end(text: &str) -> usize {
    text.rfind(char::is_whitespace).unwrap_or(text.len())
}

pub(crate) fn passage_at(text: &str, at: usize, skip_title: bool) -> String {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let line_start = text[..at].rfind('\n').map_or(0, |found| found + 1);
    let mut from = line_start;
    if skip_title {
        let first_line = text[from..].find('\n').map_or(text.len(), |end| from + end);
        let more = text[first_line..].trim_start().is_empty();
        if !more && text[from..first_line].chars().count() <= TITLE_LINE_CHARS {
            from = first_line;
        }
    }
    let flat: String = text[from..]
        .chars()
        .take(PASSAGE_CHARS * 2)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clipped: String = flat.chars().take(PASSAGE_CHARS).collect();
    if clipped.len() == flat.len() {
        return clipped;
    }
    let end = sentence_end(&clipped).unwrap_or_else(|| word_end(&clipped));
    clipped[..end].to_owned()
}

impl<R: ReadAt> Reader<R> {
    pub fn word_count(&self) -> u64 {
        self.rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.linear_ordinal.is_some())
            .filter_map(|(ix, _)| self.visible_text(ChapterIx(ix as u32)).ok())
            .map(|text| count_words(text.as_str()))
            .sum()
    }

    pub fn chapter_label(&self, ix: ChapterIx, ordinal: Option<u32>) -> String {
        self.toc()
            .ok()
            .and_then(|toc| {
                toc.entries
                    .iter()
                    .find(|entry| entry.target == Some(ix))
                    .map(|entry| entry.label.clone())
            })
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| format!("Chapter {}", ordinal.unwrap_or(ix.0 + 1)))
    }

    pub fn resume_point(
        &self,
        ix: ChapterIx,
        point: Option<DomPoint>,
    ) -> Result<ResumePoint, Error> {
        let row = self
            .rows()
            .get(ix.0 as usize)
            .ok_or_else(|| Error::Book("That chapter is not in this book.".to_owned()))?;
        let href = row
            .href
            .as_ref()
            .ok_or_else(|| Error::Book("That chapter has no file in this book.".to_owned()))?
            .as_str()
            .to_owned();
        let visible = self
            .visible_text(ix)
            .unwrap_or_else(|_| VisibleText::empty());
        let text = visible.as_str();
        let at = point.filter(|_| !text.is_empty()).map_or(0, |point| {
            let range = crate::DomRange {
                start: point,
                end: point,
            };
            let VisRange { start, .. } = visible.from_dom(range);
            start as usize
        });
        let within = if text.is_empty() {
            0.0
        } else {
            at as f64 / text.len() as f64
        };
        let chapters = self.summary().linear_chapters.max(1);
        let done = f64::from(row.linear_ordinal.unwrap_or(1).saturating_sub(1));
        Ok(ResumePoint {
            href,
            point,
            chapter_label: self.chapter_label(ix, row.linear_ordinal),
            ordinal: row.linear_ordinal,
            progress: (((done + within) / f64::from(chapters)) as f32).clamp(0.0, 1.0),
            passage: passage_at(text, at, point.is_none()),
        })
    }
}

#[cfg(test)]
mod tests;
