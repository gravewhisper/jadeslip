use crate::commands::{Chapter, chapter_view};
use crate::headings::Headings;
use crate::search::{Applies, Searches};
use jadeslip_core::{
    Book, BookKey, ChapterIx, DomPoint, Error, Library, LibraryConfig, Migration, NewBookMeta,
    Prefs, Reader, SearchMemory, Store,
};
use std::ffi::OsString;
use std::fs::File;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use crate::library::OpenFailure;
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::Instant;

pub enum Opening {
    NoBook,
    Failed(OpenFailure),
    Opened,
}

pub struct Ready {
    path: PathBuf,
    book: RwLock<Arc<Book>>,
    pub first: Mutex<Option<Result<Chapter, String>>>,
    pub start: ChapterIx,
    pub place: Option<DomPoint>,
}

impl Ready {
    pub fn reader(&self) -> Arc<Reader<File>> {
        self.book().read()
    }

    pub fn book(&self) -> Arc<Book> {
        self.book
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub struct CloseGuard {
    pub editing: bool,
    pub asked_at: Option<Instant>,
    pub allowed: bool,
    pub flushing: bool,
}

#[derive(PartialEq, Eq, Debug)]
pub enum CloseStep {
    Close,
    FlushPlace,
    AskPage,
}

#[derive(Clone, Default)]
pub struct Moving {
    pub finished: bool,
    pub migration: Migration,
}

#[derive(Default)]
pub struct Mover {
    state: Mutex<Moving>,
    finished: Condvar,
}

impl Mover {
    pub fn state(&self) -> Moving {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, migration: &Migration, finished: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.migration = migration.clone();
        state.finished = finished;
        if finished {
            self.finished.notify_all();
        }
    }

    fn wait(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _done = self
            .finished
            .wait_while(state, |state| !state.finished)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

#[derive(Clone)]
pub struct Session {
    opening: Arc<OnceLock<Opening>>,
    current: Arc<RwLock<Option<Arc<Ready>>>>,
    library: Option<Arc<Library>>,
    pub mover: Arc<Mover>,
    pub dropped: Arc<Mutex<Vec<PathBuf>>>,
    pub close: Arc<Mutex<CloseGuard>>,
    pub searches: Arc<Searches>,
    pub offers: Arc<Searches>,
    pub cleans: Arc<Searches>,
    pub applies: Arc<Applies>,
    pub headings: Arc<Headings>,
    pub saved: Arc<Saved>,
}

pub struct Saved {
    store: Option<Store>,
    prefs: Mutex<Prefs>,
    writing: Mutex<()>,
}

impl Saved {
    pub fn load() -> Saved {
        let store = data_dir().ok().map(Store::new);
        let prefs = store.as_ref().map(Store::prefs).unwrap_or_default();
        Saved {
            store,
            prefs: Mutex::new(prefs),
            writing: Mutex::new(()),
        }
    }

    pub fn prefs(&self) -> Prefs {
        *self
            .prefs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn store(&self) -> Result<&Store, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "The HOME folder is not set, so settings have nowhere to go.".to_owned())
    }

    pub fn searches(&self) -> SearchMemory {
        self.store().map(Store::searches).unwrap_or_default()
    }

    pub fn set_searches(&self, memory: &SearchMemory) -> Result<(), String> {
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.store()?
            .set_searches(memory)
            .map_err(|error| error.user_message())
    }

    pub fn set_prefs(&self, prefs: Prefs) -> Result<(), String> {
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.store()?
            .set_prefs(&prefs)
            .map_err(|error| error.user_message())?;
        *self
            .prefs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = prefs;
        Ok(())
    }
}

pub fn guarded<T, E: From<String>>(work: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
        Err(E::from(
            "Something went wrong while reading this book. The book file was not changed."
                .to_owned(),
        ))
    })
}

pub(crate) fn data_dir() -> Result<PathBuf, Error> {
    if let Some(dir) = std::env::var_os("JADESLIP_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(feature = "e2e")]
    return Err(Error::Book(
        "Test builds need JADESLIP_DATA_DIR, so they never touch the real data folder."
            .to_owned(),
    ));
    #[cfg(all(not(feature = "e2e"), not(windows)))]
    return std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share/jadeslip"))
        .ok_or_else(|| {
            Error::Book("The HOME folder is not set, so backups have nowhere to go.".to_owned())
        });
    #[cfg(all(not(feature = "e2e"), windows))]
    return std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join("dev.jadeslip.app"))
        .ok_or_else(|| {
            Error::Book(
                "The LOCALAPPDATA folder is not set, so backups have nowhere to go.".to_owned(),
            )
        });
}

fn start_of(
    library: &Library,
    book: &Book,
    writing: bool,
) -> Option<(ChapterIx, Option<DomPoint>)> {
    let entry = library.known(BookKey::of(&book.identity()))?;
    let point = if writing {
        entry.write_point.or(entry.read_point)
    } else {
        entry.read_point
    }?;
    Some((book.read().chapter_of_entry(&point.href)?, point.point))
}

fn ready_from(book: Book, library: &Library, writing: bool) -> Result<Ready, Error> {
    crate::mark!("book_open");
    let reader = book.read();
    let (start, place) = match start_of(library, &book, writing) {
        Some(found) => found,
        None => (reader.start_chapter()?, None),
    };
    crate::mark!("start_chapter");
    #[cfg(feature = "e2e")]
    if reader.toc_is_parsed() {
        crate::mark!("toc_parsed_to_start");
    }
    let first = chapter_view(&reader, start).map_err(|error| error.user_message());
    crate::mark!("first_render");
    Ok(Ready {
        path: book.path().to_owned(),
        book: RwLock::new(Arc::new(book)),
        first: Mutex::new(Some(first)),
        start,
        place,
    })
}

impl Session {
    pub fn start(path: Option<OsString>) -> Session {
        let library = data_dir()
            .ok()
            .and_then(|data_dir| Library::open(LibraryConfig { data_dir }).ok());
        let session = Session {
            opening: Arc::new(OnceLock::new()),
            current: Arc::new(RwLock::new(None)),
            library,
            mover: Arc::new(Mover::default()),
            dropped: Arc::new(Mutex::new(Vec::new())),
            close: Arc::new(Mutex::new(CloseGuard {
                editing: false,
                asked_at: None,
                allowed: false,
                flushing: false,
            })),
            searches: Arc::new(Searches::default()),
            offers: Arc::new(Searches::default()),
            cleans: Arc::new(Searches::default()),
            applies: Arc::new(Applies::default()),
            headings: Arc::new(Headings::default()),
            saved: Arc::new(Saved::load()),
        };
        let mover = session.mover.clone();
        let moving = session.library.clone();
        std::thread::spawn(move || {
            let migration = match moving {
                Some(library) => guarded(|| {
                    Ok::<_, String>(library.migrate(|state| mover.set(state, false)))
                })
                .unwrap_or_default(),
                None => Migration::default(),
            };
            mover.set(&migration, true);
            crate::mark!("migration_done");
        });
        let cell = session.opening.clone();
        let opener = session.clone();
        std::thread::spawn(move || {
            let opening = match path {
                None => Opening::NoBook,
                Some(path) => guarded(|| {
                    let path = Path::new(&path);
                    opener
                        .open_path(path, false)
                        .map_err(|error| OpenFailure::of(&opener, path, None, &error))
                })
                .map_or_else(Opening::Failed, |_| Opening::Opened),
            };
            let _ = cell.set(opening);
        });
        session
    }

    pub fn library(&self) -> Result<&Arc<Library>, String> {
        self.library.as_ref().ok_or_else(|| {
            "The HOME folder is not set, so the shelf has nowhere to live.".to_owned()
        })
    }

    pub fn shelf(&self) -> Result<&Arc<Library>, String> {
        self.mover.wait();
        self.library()
    }

    pub fn current(&self) -> Option<Arc<Ready>> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_current(&self, ready: Option<Arc<Ready>>) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = ready;
    }

    pub fn open_path(&self, path: &Path, writing: bool) -> Result<Arc<Ready>, Error> {
        let library = self.library().map_err(Error::Book)?;
        let book = library.open_book(path)?;
        let ready = Arc::new(ready_from(book, library, writing)?);
        self.set_current(Some(ready.clone()));
        let library = library.clone();
        let book = ready.book();
        std::thread::spawn(move || {
            let key = BookKey::of(&book.identity());
            let _ = catch_unwind(AssertUnwindSafe(|| library.update_cover(key, &book.read())));
            crate::mark!("cover_done");
            let _ = catch_unwind(AssertUnwindSafe(|| library.update_words(key, &book.read())));
        });
        Ok(ready)
    }

    pub fn count_words_later(&self, book: &Book) {
        let Ok(library) = self.library() else { return };
        let key = BookKey::of(&book.identity());
        if !library.entry(key).is_some_and(|entry| entry.own) {
            return;
        }
        let library = library.clone();
        let reader = book.read();
        std::thread::spawn(move || {
            let _ = catch_unwind(AssertUnwindSafe(|| library.update_words(key, &reader)));
            crate::mark!("words_done");
        });
    }

    pub fn count_words_of_file_later(&self, key: BookKey) {
        let Ok(library) = self.library() else { return };
        let library = library.clone();
        std::thread::spawn(move || {
            let _ = catch_unwind(AssertUnwindSafe(|| library.update_words_from_file(key)));
            crate::mark!("words_done");
        });
    }

    pub fn close_book(&self) {
        self.set_current(None);
    }

    pub fn wait(&self) -> &Opening {
        self.opening.wait()
    }

    fn ready(&self) -> Result<Arc<Ready>, String> {
        let opening = self.wait();
        if let Some(ready) = self
            .current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok(ready.clone());
        }
        match opening {
            Opening::Failed(failure) => Err(failure.message.clone()),
            Opening::NoBook | Opening::Opened => Err("No book is open.".to_owned()),
        }
    }

    pub fn create_book(&self, meta: &NewBookMeta) -> Result<Arc<Ready>, Error> {
        let library = self.shelf().map_err(Error::Book)?;
        let book = library.create_book(meta)?;
        let ready = Arc::new(ready_from(book, library, true)?);
        self.set_current(Some(ready.clone()));
        self.count_words_later(&ready.book());
        Ok(ready)
    }

    pub fn save_place(
        &self,
        ix: ChapterIx,
        point: Option<DomPoint>,
        writing: bool,
    ) -> Result<(), String> {
        let book = self.book()?;
        let reader = book.read();
        let row = reader
            .rows()
            .get(ix.0 as usize)
            .ok_or_else(|| "That chapter is not in this book.".to_owned())?;
        if row.href.is_none() {
            return Ok(());
        }
        let resume = reader
            .resume_point(ix, point)
            .map_err(|error| error.user_message())?;
        self.library()?
            .save_resume_point(BookKey::of(&book.identity()), writing, resume)
            .map_err(|error| error.user_message())
    }

    pub fn on_shelf(&self, book: &Book) -> bool {
        self.library().is_ok_and(|library| {
            library
                .entry(BookKey::of(&book.identity()))
                .is_some_and(|entry| entry.path == book.path())
        })
    }

    pub fn book(&self) -> Result<Arc<Book>, String> {
        Ok(self.ready()?.book())
    }

    pub fn reader(&self) -> Result<Arc<Reader<File>>, String> {
        Ok(self.ready()?.reader())
    }

    pub fn reload(&self) -> Result<(), String> {
        let ready = self.ready()?;
        let book = self
            .library()?
            .open_book(&ready.path)
            .map_err(|error| error.user_message())?;
        *ready
            .book
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(book);
        Ok(())
    }
}
impl CloseGuard {
    pub fn step(&mut self, now: Instant) -> CloseStep {
        if self.allowed {
            return CloseStep::Close;
        }
        if !self.editing {
            if self.flushing {
                return CloseStep::Close;
            }
            self.flushing = true;
            return CloseStep::FlushPlace;
        }
        let unanswered = self
            .asked_at
            .is_some_and(|asked| now.duration_since(asked).as_secs() >= UI_ANSWER_SECONDS);
        if unanswered {
            return CloseStep::Close;
        }
        self.asked_at.get_or_insert(now);
        CloseStep::AskPage
    }
}

const UI_ANSWER_SECONDS: u64 = 5;

#[cfg(test)]
mod tests {
    use super::{CloseGuard, CloseStep};
    use std::time::{Duration, Instant};

    fn guard(editing: bool) -> CloseGuard {
        CloseGuard {
            editing,
            asked_at: None,
            allowed: false,
            flushing: false,
        }
    }

    #[test]
    fn reading_flushes_the_place_once_and_editing_asks_the_page() {
        let now = Instant::now();
        let mut reading = guard(false);
        assert_eq!(reading.step(now), CloseStep::FlushPlace);
        assert_eq!(reading.step(now), CloseStep::Close);
        assert_eq!(guard(true).step(now), CloseStep::AskPage);
    }

    #[test]
    fn a_page_that_never_answers_cannot_trap_the_window() {
        let now = Instant::now();
        let mut hung = guard(true);
        assert_eq!(hung.step(now), CloseStep::AskPage);
        assert_eq!(hung.step(now + Duration::from_secs(1)), CloseStep::AskPage);
        assert_eq!(hung.step(now + Duration::from_secs(6)), CloseStep::Close);
    }

    #[test]
    fn a_page_that_answered_keeps_asking_however_long_the_dialog_stays_open() {
        let now = Instant::now();
        let mut open = guard(true);
        assert_eq!(open.step(now), CloseStep::AskPage);
        open.asked_at = None;
        assert_eq!(
            open.step(now + Duration::from_secs(600)),
            CloseStep::AskPage
        );
    }
}
