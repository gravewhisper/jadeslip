use crate::commands::{
    BookInfo, DeletedRow, ManagedView, NewBookView, book_info, opened_view, pick_save_path,
    with_epub_extension,
};
use crate::session::{Session, guarded};
use serde::{Deserialize, Serialize};
use jadeslip_core::{
    AddOutcome, AnchorResolution, BOOK_URL_PREFIX, BackupStatus, Book, BookKey, ChapterIx,
    ChapterRev, CommitReport, CoverFound, CoverStamp, DomRange, EntryName, Mark, Reader,
    ResumePoint, RevertKind, RevertTarget, ShelfAdd, VisRange, key_hex, parse_key,
};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

const MAX_LABEL_CHARS: usize = 300;

fn short(text: &str) -> String {
    text.chars().take(MAX_LABEL_CHARS).collect()
}

fn no_message(error: jadeslip_core::Error) -> String {
    error.user_message()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointRow {
    label: String,
    ordinal: Option<u32>,
    progress: f32,
    passage: String,
}

impl From<ResumePoint> for PointRow {
    fn from(point: ResumePoint) -> PointRow {
        PointRow {
            label: short(&point.chapter_label),
            ordinal: point.ordinal,
            progress: point.progress,
            passage: point.passage,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfRow {
    key: String,
    title: String,
    author: Option<String>,
    chapters: u32,
    bytes: u64,
    own: bool,
    read: Option<PointRow>,
    write: Option<PointRow>,
    touched_at: u64,
    backed_up: bool,
    cover: Option<String>,
    words: Option<u64>,
}

fn cover_url(key: BookKey, cover: Option<CoverStamp>) -> Option<String> {
    cover
        .filter(|cover| cover.found == CoverFound::Thumbnail)
        .map(|cover| format!("{BOOK_URL_PREFIX}cover/{}?v={}", key_hex(key), cover.file))
}

pub fn shelf_rows(session: &Session) -> Vec<ShelfRow> {
    let Ok(library) = session.library() else {
        return Vec::new();
    };
    library
        .shelf()
        .into_iter()
        .map(|(key, entry)| ShelfRow {
            key: key_hex(key),
            title: short(&entry.title),
            author: entry.author.as_deref().map(short),
            chapters: entry.linear_chapters,
            bytes: entry.inflated_spine_bytes,
            own: entry.own,
            read: entry.read_point.map(PointRow::from),
            write: entry.write_point.map(PointRow::from),
            touched_at: entry.touched_at,
            backed_up: library.has_backup(key),
            cover: cover_url(key, entry.cover),
            words: entry.words.filter(|_| entry.own).map(|words| words.count),
        })
        .collect()
}

#[tauri::command(async)]
pub fn shelf(session: State<'_, Session>) -> Vec<ShelfRow> {
    shelf_rows(&session)
}

#[tauri::command(async)]
pub fn fill_covers(session: State<'_, Session>) -> Result<usize, String> {
    guarded(|| Ok(session.library()?.fill_covers()))
}

fn key_of(text: &str) -> Result<BookKey, String> {
    parse_key(text).ok_or_else(|| "That book is not on the shelf.".to_owned())
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum FailKind {
    Missing,
    Unreadable,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenFailure {
    kind: FailKind,
    pub message: String,
    key: Option<String>,
}

impl From<String> for OpenFailure {
    fn from(message: String) -> OpenFailure {
        OpenFailure {
            kind: FailKind::Unreadable,
            message,
            key: None,
        }
    }
}

impl OpenFailure {
    pub fn of(
        session: &Session,
        path: &Path,
        key: Option<BookKey>,
        error: &jadeslip_core::Error,
    ) -> OpenFailure {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        let key = key.or_else(|| session.library().ok()?.key_at(&path));
        let missing = matches!(error, jadeslip_core::Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound);
        let shown = path.display();
        let (kind, message) = if missing {
            (
                FailKind::Missing,
                format!("Jadeslip cannot find the book at {shown}. It may have been moved, renamed or deleted."),
            )
        } else {
            (
                FailKind::Unreadable,
                format!("Jadeslip could not open {shown}. {}", error.user_message()),
            )
        };
        OpenFailure {
            kind,
            message,
            key: key.map(key_hex),
        }
    }
}

fn open_at(
    session: &Session,
    path: &Path,
    key: Option<BookKey>,
    writing: bool,
) -> Result<NewBookView, OpenFailure> {
    let ready = session
        .open_path(path, writing)
        .map_err(|error| OpenFailure::of(session, path, key, &error))?;
    Ok(NewBookView::Opened(Box::new(opened_view(session, &ready)?)))
}

#[tauri::command(async)]
pub fn open_shelf_book(
    session: State<'_, Session>,
    key: String,
    writing: bool,
) -> Result<NewBookView, OpenFailure> {
    guarded(|| {
        let key = key_of(&key)?;
        let entry = session
            .shelf()?
            .entry(key)
            .ok_or_else(|| "That book is not on the shelf.".to_owned())?;
        open_at(&session, &entry.path, Some(key), writing)
    })
}

#[tauri::command(async)]
pub fn relink_book(
    app: AppHandle,
    session: State<'_, Session>,
    key: Option<String>,
) -> Result<NewBookView, OpenFailure> {
    guarded(|| {
        let Some(path) = pick_open_path(&app) else {
            return Ok(NewBookView::Cancelled);
        };
        let Some(key) = key.as_deref().map(key_of).transpose()? else {
            return open_at(&session, &path, None, false);
        };
        let library = session.shelf()?;
        let found = library
            .key_of_file(&path)
            .map_err(|error| OpenFailure::of(&session, &path, Some(key), &error))?;
        if found != key {
            let title = library.entry(key).map(|entry| entry.title).unwrap_or_default();
            return Err(format!(
                "{} is a different book. Pick the file for {title}.",
                path.display()
            )
            .into());
        }
        library.add_to_shelf(&path).map_err(no_message)?;
        let entry = library
            .entry(key)
            .ok_or_else(|| "That book is not on the shelf.".to_owned())?;
        open_at(&session, &entry.path, Some(key), false)
    })
}

#[cfg(not(feature = "e2e"))]
fn pick_open_path(app: &AppHandle) -> Option<PathBuf> {
    native_open_dialog(app)
}

#[cfg(feature = "e2e")]
fn pick_open_path(app: &AppHandle) -> Option<PathBuf> {
    match crate::e2e::take_save_answer() {
        crate::e2e::SaveAnswer::Path(path) => Some(path),
        crate::e2e::SaveAnswer::Cancel => None,
        crate::e2e::SaveAnswer::RealDialog => native_open_dialog(app),
    }
}

fn native_open_dialog(app: &AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("Open an EPUB")
        .add_filter("EPUB book", &["epub", "EPUB"])
        .blocking_pick_file()
        .and_then(|picked| picked.into_path().ok())
}

#[tauri::command(async)]
pub fn pick_and_open(app: AppHandle, session: State<'_, Session>) -> Result<NewBookView, String> {
    guarded(|| {
        let Some(path) = pick_open_path(&app) else {
            return Ok(NewBookView::Cancelled);
        };
        let ready = session.open_path(&path, false).map_err(no_message)?;
        Ok(NewBookView::Opened(Box::new(opened_view(&session, &ready)?)))
    })
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AddView {
    Cancelled,
    Added { key: String, title: String },
    AlreadyOnShelf { key: String, title: String },
}

impl From<ShelfAdd> for AddView {
    fn from(add: ShelfAdd) -> AddView {
        let key = key_hex(add.key);
        let title = short(&add.title);
        match add.outcome {
            AddOutcome::Added => AddView::Added { key, title },
            AddOutcome::AlreadyOnShelf => AddView::AlreadyOnShelf { key, title },
        }
    }
}

#[tauri::command(async)]
pub fn pick_and_add(app: AppHandle, session: State<'_, Session>) -> Result<AddView, String> {
    guarded(|| {
        let Some(path) = pick_open_path(&app) else {
            return Ok(AddView::Cancelled);
        };
        Ok(session
            .shelf()?
            .add_to_shelf(&path)
            .map_err(no_message)?
            .into())
    })
}

#[tauri::command(async)]
pub fn add_open_book(session: State<'_, Session>) -> Result<AddView, String> {
    guarded(|| {
        let book = session.book()?;
        Ok(session
            .shelf()?
            .add_to_shelf(book.path())
            .map_err(no_message)?
            .into())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovingView {
    finished: bool,
    total: usize,
    done: usize,
    copied: usize,
    failed: Vec<String>,
}

#[tauri::command(async)]
pub fn library_moving(session: State<'_, Session>) -> MovingView {
    let moving = session.mover.state();
    MovingView {
        finished: moving.finished,
        total: moving.migration.total,
        done: moving.migration.done,
        copied: moving.migration.copied,
        failed: moving.migration.failed,
    }
}

pub fn receive_drop(session: &Session, paths: Vec<PathBuf>) {
    if let Ok(mut held) = session.dropped.lock() {
        *held = paths;
    }
}

fn take_dropped(session: &Session) -> Vec<PathBuf> {
    session
        .dropped
        .lock()
        .map(|mut held| std::mem::take(&mut *held))
        .unwrap_or_default()
}

#[derive(Serialize)]
pub struct DroppedView {
    opened: NewBookView,
    others: usize,
}

#[tauri::command(async)]
pub fn open_dropped(session: State<'_, Session>) -> Result<DroppedView, String> {
    guarded(|| {
        let mut paths = take_dropped(&session).into_iter();
        let Some(first) = paths.next() else {
            return Ok(DroppedView {
                opened: NewBookView::Cancelled,
                others: 0,
            });
        };
        let ready = session.open_path(&first, false).map_err(no_message)?;
        Ok(DroppedView {
            opened: NewBookView::Opened(Box::new(opened_view(&session, &ready)?)),
            others: paths.count(),
        })
    })
}

#[derive(Serialize)]
pub struct AddedView {
    added: Vec<AddView>,
    failed: Vec<String>,
}

#[tauri::command(async)]
pub fn add_dropped(session: State<'_, Session>) -> Result<AddedView, String> {
    guarded(|| {
        let paths = take_dropped(&session);
        let library = session.shelf()?;
        let mut added = Vec::new();
        let mut failed = Vec::new();
        for path in paths {
            match library.add_to_shelf(&path) {
                Ok(add) => added.push(add.into()),
                Err(error) => failed.push(format!(
                    "{} was not added. {}",
                    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy(),
                    error.user_message()
                )),
            }
        }
        Ok(AddedView { added, failed })
    })
}

#[tauri::command(async)]
pub fn close_book(session: State<'_, Session>) -> Result<(), String> {
    guarded(|| {
        if let Ok(book) = session.book() {
            session.library()?.refresh(&book);
        }
        session.close_book();
        Ok(())
    })
}

#[tauri::command(async)]
pub fn set_own(session: State<'_, Session>, key: String, own: bool) -> Result<(), String> {
    guarded(|| {
        let key = key_of(&key)?;
        session.shelf()?.set_own(key, own).map_err(no_message)?;
        if own {
            session.count_words_of_file_later(key);
        }
        Ok(())
    })
}

#[tauri::command(async)]
pub fn reveal_shelf_book(session: State<'_, Session>, key: String) -> Result<(), String> {
    guarded(|| {
        let entry = session
            .shelf()?
            .entry(key_of(&key)?)
            .ok_or_else(|| "That book is not on the shelf.".to_owned())?;
        let folder = entry
            .path
            .parent()
            .filter(|folder| folder.is_dir())
            .ok_or_else(|| {
                format!(
                    "The folder for {} is not there any more.",
                    entry.path.display()
                )
            })?;
        crate::external::reveal(folder);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn export_shelf_copy(
    app: AppHandle,
    session: State<'_, Session>,
    key: String,
) -> Result<ExportView, String> {
    guarded(|| {
        let key = key_of(&key)?;
        let library = session.shelf()?;
        let title = library
            .entry(key)
            .map(|entry| format!("{} copy", entry.title))
            .ok_or_else(|| "That book is not on the shelf.".to_owned())?;
        let Some(path) = pick_save_path(&app, "Export a copy", &title) else {
            return Ok(ExportView::Cancelled);
        };
        library
            .export_copy_of(key, &with_epub_extension(path))
            .map_err(no_message)?;
        Ok(ExportView::Saved)
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalView {
    title: String,
    data_bytes: u64,
    file_bytes: Option<u64>,
}

#[tauri::command(async)]
pub fn removal_info(session: State<'_, Session>, key: String) -> Result<RemovalView, String> {
    guarded(|| {
        let removal = session
            .shelf()?
            .removal(key_of(&key)?)
            .ok_or_else(|| "That book is not on the shelf.".to_owned())?;
        Ok(RemovalView {
            title: short(&removal.title),
            data_bytes: removal.data_bytes,
            file_bytes: removal.file_bytes,
        })
    })
}

#[cfg(not(windows))]
#[tauri::command(async)]
pub fn remove_book(
    session: State<'_, Session>,
    key: String,
    data: bool,
    file: bool,
) -> Result<(), String> {
    guarded(|| {
        let trash = jadeslip_core::Trash::home()
            .ok_or_else(|| "The HOME folder is not set, so there is no trash to move files to.".to_owned())?;
        session
            .shelf()?
            .remove_with(key_of(&key)?, data, file, &trash)
            .map_err(no_message)
    })
}

#[cfg(windows)]
#[tauri::command(async)]
pub fn remove_book(
    session: State<'_, Session>,
    key: String,
    data: bool,
    file: bool,
) -> Result<(), String> {
    guarded(|| {
        if data || file {
            return Err("On Windows, Jadeslip cannot move a book's files to the Recycle Bin yet, so nothing was removed. Untick both boxes to take the book off the shelf and keep its files.".to_owned());
        }
        session.shelf()?.remove(key_of(&key)?).map_err(no_message)
    })
}

#[tauri::command(async)]
pub fn remove_from_shelf(session: State<'_, Session>, key: String) -> Result<(), String> {
    guarded(|| session.shelf()?.remove(key_of(&key)?).map_err(no_message))
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ExportView {
    Cancelled,
    Saved,
}

#[tauri::command(async)]
pub fn export_copy_dialog(
    app: AppHandle,
    session: State<'_, Session>,
) -> Result<ExportView, String> {
    guarded(|| {
        let book = session.book()?;
        let title = format!("{} copy", book.read().summary().title);
        let Some(path) = pick_save_path(&app, "Export a copy", &title) else {
            return Ok(ExportView::Cancelled);
        };
        book.export_copy(&with_epub_extension(path))
            .map_err(no_message)?;
        Ok(ExportView::Saved)
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertRowView {
    entry: String,
    kind: &'static str,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RevertPreview {
    None,
    OtherFileOnly,
    Valid {
        changed: Vec<RevertRowView>,
        deleted: Vec<DeletedRow>,
        chapter: &'static str,
    },
}

fn chapter_state(rows: &[RevertRowView], name: Option<&str>) -> &'static str {
    match rows.iter().find(|row| Some(row.entry.as_str()) == name) {
        Some(row) if row.kind == "changed" => "changed",
        Some(_) => "new",
        None if name.is_some() => "same",
        None => "new",
    }
}

#[tauri::command(async)]
pub fn revert_preview(
    session: State<'_, Session>,
    shown: Option<u32>,
) -> Result<RevertPreview, String> {
    guarded(|| {
        let book = session.book()?;
        let deleted = match book.backup_status().map_err(no_message)? {
            BackupStatus::None => return Ok(RevertPreview::None),
            BackupStatus::OtherFileOnly => return Ok(RevertPreview::OtherFileOnly),
            BackupStatus::Valid { deleted, .. } => deleted,
        };
        let changed: Vec<RevertRowView> = book
            .revert_list()
            .map_err(no_message)?
            .into_iter()
            .map(|row| RevertRowView {
                entry: row.entry.as_str().to_owned(),
                kind: match row.kind {
                    RevertKind::Changed => "changed",
                    RevertKind::Added => "added",
                    RevertKind::Removed => "removed",
                },
            })
            .collect();
        let reader = book.read();
        let name = shown
            .and_then(|ix| reader.rows().get(ix as usize))
            .filter(|row| !row.is_nav)
            .and_then(|row| row.href.as_ref())
            .map(|href| href.as_str().to_owned());
        Ok(RevertPreview::Valid {
            chapter: chapter_state(&changed, name.as_deref()),
            changed,
            deleted: deleted.into_iter().map(DeletedRow::from).collect(),
        })
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RevertChoice {
    Chapter { ix: u32 },
    WholeBook,
}

#[derive(Serialize)]
pub struct RevertedView {
    #[serde(flatten)]
    managed: ManagedView,
    wrote: bool,
}

fn first_place(reader: &Reader, shown: Option<u32>) -> Option<u32> {
    shown.or_else(|| reader.start_chapter().ok().map(|ix| ix.0))
}

#[tauri::command(async)]
pub fn revert_book(
    session: State<'_, Session>,
    choice: RevertChoice,
    shown: Option<u32>,
) -> Result<RevertedView, String> {
    guarded(|| {
        let book = session.book()?;
        let before = book.read();
        let shown_name = shown
            .and_then(|ix| before.rows().get(ix as usize))
            .and_then(|row| row.href.clone());
        let target = match choice {
            RevertChoice::WholeBook => RevertTarget::WholeBook,
            RevertChoice::Chapter { ix } => RevertTarget::Chapter(
                before
                    .rows()
                    .get(ix as usize)
                    .and_then(|row| row.href.clone())
                    .ok_or_else(|| "That chapter has no file in this book.".to_owned())?,
            ),
        };
        let report = book.revert(target).map_err(no_message)?;
        reshaped(&session, &book, shown_name, report)
    })
}

pub fn reshaped(
    session: &Session,
    book: &Book,
    shown_name: Option<EntryName>,
    report: CommitReport,
) -> Result<RevertedView, String> {
    session.library()?.refresh(book);
    session.count_words_later(book);
    let reader = book.read();
    let shown = shown_name
        .and_then(|name| reader.chapter_of_entry(name.as_str()))
        .map(|ix| ix.0);
    let book_view: BookInfo = book_info(&reader);
    Ok(RevertedView {
        wrote: report.bytes_written > 0,
        managed: ManagedView {
            landing: first_place(&reader, shown),
            shown,
            deleted: None,
            book: book_view,
            warnings: report.warnings,
        },
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkRow {
    id: u64,
    note: Option<String>,
    quote: String,
    created_at: u64,
    state: &'static str,
    ix: Option<u32>,
    number: Option<u32>,
    label: Option<String>,
    rev: Option<ChapterRev>,
    range: Option<VisRange>,
}

fn mark_row(reader: &Reader, mark: Mark, resolution: AnchorResolution) -> MarkRow {
    let (state, found) = match resolution {
        AnchorResolution::Exact { chapter, range } => ("exact", Some((chapter, range))),
        AnchorResolution::Moved { chapter, range } => ("moved", Some((chapter, range))),
        AnchorResolution::Detached => ("detached", None),
    };
    let row = found.and_then(|(chapter, _)| reader.rows().get(chapter.0 as usize));
    let number = row.and_then(|row| row.linear_ordinal);
    MarkRow {
        id: mark.id,
        note: mark.note,
        quote: short(&mark.anchor.exact),
        created_at: mark.created_at,
        state,
        ix: found.map(|(chapter, _)| chapter.0),
        number,
        label: found.map(|(chapter, _)| short(&reader.chapter_label(chapter, number))),
        rev: row.and_then(|row| row.rev),
        range: found.map(|(_, range)| range),
    }
}

fn this_key(book: &Book) -> BookKey {
    BookKey::of(&book.identity())
}

#[tauri::command(async)]
pub fn marks(session: State<'_, Session>) -> Result<Vec<MarkRow>, String> {
    guarded(|| {
        let book = session.book()?;
        let marks = session
            .library()?
            .marks(this_key(&book))
            .map_err(no_message)?;
        let reader = book.read();
        let anchors: Vec<_> = marks.iter().map(|mark| mark.anchor.clone()).collect();
        let resolved = reader.resolve_all(&anchors);
        Ok(marks
            .into_iter()
            .zip(resolved)
            .map(|(mark, resolution)| mark_row(&reader, mark, resolution))
            .rev()
            .collect())
    })
}

#[tauri::command(async)]
pub fn add_mark(
    session: State<'_, Session>,
    ix: u32,
    at: DomRange,
    note: Option<String>,
) -> Result<u64, String> {
    guarded(|| {
        let book = session.book()?;
        let anchor = book
            .read()
            .anchor_at(ChapterIx(ix), at)
            .map_err(no_message)?;
        let mark = session
            .library()?
            .add_mark(this_key(&book), anchor, note)
            .map_err(no_message)?;
        Ok(mark.id)
    })
}

#[tauri::command(async)]
pub fn delete_mark(session: State<'_, Session>, id: u64) -> Result<(), String> {
    guarded(|| {
        let book = session.book()?;
        session
            .library()?
            .delete_mark(this_key(&book), id)
            .map_err(no_message)
    })
}

#[tauri::command(async)]
pub fn resolve_mark(session: State<'_, Session>, id: u64) -> Result<MarkRow, String> {
    guarded(|| {
        let book = session.book()?;
        let mark = session
            .library()?
            .marks(this_key(&book))
            .map_err(no_message)?
            .into_iter()
            .find(|mark| mark.id == id)
            .ok_or_else(|| "That bookmark is gone.".to_owned())?;
        let reader = book.read();
        let resolution = reader.resolve(&mark.anchor);
        Ok(mark_row(&reader, mark, resolution))
    })
}
