use crate::headings::HeadingBatch;
use crate::search::{ApplyBatch, ReplaceCommand, SearchBatch, SearchRequest, Searches};
use crate::library::OpenFailure;
use crate::session::{CloseStep, Opening, Ready, Session, guarded};
use serde::{Deserialize, Serialize};
use jadeslip_core::check_new_book;
use jadeslip_core::{
    AfterBlock, BackupStatus, BlockEdits, ChapterIx, ChapterKind, ChapterOp, ChapterRev,
    CleanRule, DeletedChapter, DomPoint, DomRange, DraftOffer, Error, FixOffer, LinkTarget,
    NewBookMeta, Prefs, RULES, Reader, ReplaceExclusion, SaveOutcome, SearchMemory, SearchScope,
    TocEntry, TocSource, VisRange,
};
use std::path::PathBuf;
use std::time::Instant;
use tauri::{AppHandle, DragDropEvent, Manager, State, WebviewWindow, Window, WindowEvent};

const MAX_DISPLAY_BYTES: usize = 16 << 20;

#[derive(Serialize)]
pub struct Chapter {
    ix: u32,
    rev: ChapterRev,
    number: Option<u32>,
    prev: Option<u32>,
    next: Option<u32>,
    title: Option<String>,
    kind: &'static str,
    xhtml: String,
    css: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct BookInfo {
    title: String,
    author: Option<String>,
    chapters: u32,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum FirstText {
    NoBook {
        shelf: Vec<crate::library::ShelfRow>,
    },
    Opened(Box<Opened>),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    book: BookInfo,
    on_shelf: bool,
    start: u32,
    place: Option<DomPoint>,
    chapter: Result<Chapter, String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkView {
    Chapter { ix: u32, fragment: Option<String> },
    External,
    Blocked,
}

fn neighbour(reader: &Reader, ix: ChapterIx, forward: bool) -> Option<u32> {
    let rows = reader.rows();
    let at = ix.0 as usize;
    let linear = |row: &&jadeslip_core::ChapterRow| row.linear_ordinal.is_some();
    let found = if forward {
        rows.get(at + 1..)?.iter().find(linear)
    } else {
        rows.get(..at)?.iter().rev().find(linear)
    };
    found.map(|row| row.ix.0)
}

pub fn chapter_view(reader: &Reader, ix: ChapterIx) -> Result<Chapter, Error> {
    let rendered = reader.render(ix)?;
    if rendered.xhtml.len() > MAX_DISPLAY_BYTES {
        return Err(Error::Limit(format!(
            "This chapter is too large to display: {} MB, and the reader's limit is {} MB.",
            rendered.xhtml.len() >> 20,
            MAX_DISPLAY_BYTES >> 20
        )));
    }
    #[cfg(feature = "e2e")]
    let rendered = crate::e2e::unsanitized_if_switched(reader, ix, rendered);
    Ok(Chapter {
        ix: ix.0,
        rev: rendered.rev,
        number: reader
            .rows()
            .get(ix.0 as usize)
            .and_then(|row| row.linear_ordinal),
        prev: neighbour(reader, ix, false),
        next: neighbour(reader, ix, true),
        title: rendered.title,
        kind: match rendered.kind {
            ChapterKind::Text => "text",
            ChapterKind::ImagePage => "image",
            ChapterKind::Empty => "empty",
        },
        xhtml: rendered.xhtml,
        css: rendered.css,
        warnings: rendered.warnings,
    })
}

pub fn book_info(reader: &Reader) -> BookInfo {
    let summary = reader.summary();
    BookInfo {
        title: summary.title.clone(),
        author: summary.author.clone(),
        chapters: summary.linear_chapters,
    }
}

pub fn opened_view(session: &Session, ready: &Ready) -> Result<Opened, String> {
    let pending = ready
        .first
        .lock()
        .map_err(|_| "The reader stopped unexpectedly.".to_owned())?
        .take();
    let chapter = pending.unwrap_or_else(|| {
        guarded(|| chapter_view(&ready.reader(), ready.start).map_err(|error| error.user_message()))
    });
    Ok(Opened {
        on_shelf: session.on_shelf(&ready.book()),
        book: book_info(&ready.reader()),
        start: ready.start.0,
        place: ready.place,
        chapter,
    })
}

#[tauri::command(async)]
pub fn first_text(session: State<'_, Session>) -> Result<FirstText, OpenFailure> {
    crate::mark!("first_text_called");
    if let Opening::Failed(failure) = session.wait()
        && session.current().is_none()
    {
        return Err(failure.clone());
    }
    match session.current() {
        Some(ready) => Ok(FirstText::Opened(Box::new(opened_view(&session, &ready)?))),
        None => Ok(FirstText::NoBook {
            shelf: crate::library::shelf_rows(&session),
        }),
    }
}

#[derive(Serialize)]
pub struct ManagedView {
    pub landing: Option<u32>,
    pub shown: Option<u32>,
    pub deleted: Option<u64>,
    pub book: BookInfo,
    pub warnings: Vec<String>,
}

#[tauri::command(async)]
pub fn manage(
    session: State<'_, Session>,
    op: ChapterOp,
    shown: Option<u32>,
) -> Result<ManagedView, String> {
    guarded(|| {
        let book = session.book()?;
        let managed = book
            .manage(op, shown.map(ChapterIx))
            .map_err(|error| error.user_message())?;
        session.library()?.refresh(&book);
        session.count_words_later(&book);
        Ok(ManagedView {
            landing: managed.landing.map(|ix| ix.0),
            shown: managed.shown.map(|ix| ix.0),
            deleted: managed.deleted.map(|id| id.0),
            book: book_info(&book.read()),
            warnings: managed.report.warnings,
        })
    })
}

#[derive(Serialize)]
pub struct DeletedRow {
    pub id: u64,
    title: String,
    deleted_at: u64,
}

impl From<DeletedChapter> for DeletedRow {
    fn from(chapter: DeletedChapter) -> DeletedRow {
        DeletedRow {
            id: chapter.id.0,
            title: chapter.title.chars().take(MAX_LABEL_CHARS).collect(),
            deleted_at: chapter.deleted_at,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum BackupView {
    None,
    Valid { deleted: Vec<DeletedRow> },
    OtherFileOnly,
}

#[tauri::command(async)]
pub fn backup_status(session: State<'_, Session>) -> Result<BackupView, String> {
    guarded(|| {
        let status = session
            .book()?
            .backup_status()
            .map_err(|error| error.user_message())?;
        Ok(match status {
            BackupStatus::None => BackupView::None,
            BackupStatus::OtherFileOnly => BackupView::OtherFileOnly,
            BackupStatus::Valid { deleted, .. } => BackupView::Valid {
                deleted: deleted.into_iter().map(DeletedRow::from).collect(),
            },
        })
    })
}

pub fn with_epub_extension(mut path: PathBuf) -> PathBuf {
    let has_it = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"));
    if !has_it {
        let mut name = path.file_name().unwrap_or_default().to_owned();
        name.push(".epub");
        path.set_file_name(name);
    }
    path
}

#[cfg(not(feature = "e2e"))]
pub fn pick_save_path(app: &AppHandle, heading: &str, title: &str) -> Option<PathBuf> {
    native_save_dialog(app, heading, title)
}

#[cfg(feature = "e2e")]
pub fn pick_save_path(app: &AppHandle, heading: &str, title: &str) -> Option<PathBuf> {
    match crate::e2e::take_save_answer() {
        crate::e2e::SaveAnswer::Path(path) => Some(path),
        crate::e2e::SaveAnswer::Cancel => None,
        crate::e2e::SaveAnswer::RealDialog => native_save_dialog(app, heading, title),
    }
}

fn native_save_dialog(app: &AppHandle, heading: &str, title: &str) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let suggested: String = title
        .chars()
        .filter(|c| !matches!(c, '/' | '\\') && !c.is_control())
        .take(80)
        .collect();
    app.dialog()
        .file()
        .set_title(heading)
        .add_filter("EPUB book", &["epub"])
        .set_file_name(format!("{suggested}.epub"))
        .blocking_save_file()
        .and_then(|picked| picked.into_path().ok())
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum NewBookView {
    Cancelled,
    Opened(Box<Opened>),
}

#[tauri::command(async)]
pub fn new_book(session: State<'_, Session>, meta: NewBookMeta) -> Result<NewBookView, String> {
    guarded(|| {
        check_new_book(&meta).map_err(|error| error.user_message())?;
        let ready = session
            .create_book(&meta)
            .map_err(|error| error.user_message())?;
        Ok(NewBookView::Opened(Box::new(opened_view(&session, &ready)?)))
    })
}

#[tauri::command(async)]
pub fn render_chapter(session: State<'_, Session>, ix: u32) -> Result<Chapter, String> {
    guarded(|| {
        let reader = session.reader()?;
        chapter_view(&reader, ChapterIx(ix)).map_err(|error| error.user_message())
    })
}

#[tauri::command(async)]
pub fn follow_link(
    session: State<'_, Session>,
    from: u32,
    href: String,
) -> Result<LinkView, String> {
    guarded(|| {
        let reader = session.reader()?;
        Ok(match reader.link(ChapterIx(from), &href) {
            LinkTarget::Chapter { ix, fragment } => LinkView::Chapter { ix: ix.0, fragment },
            LinkTarget::External { url } => {
                crate::external::open(&url);
                LinkView::External
            }
            LinkTarget::Blocked => LinkView::Blocked,
        })
    })
}

const MAX_LABEL_CHARS: usize = 300;

#[derive(Serialize)]
pub struct TocRow {
    label: String,
    ix: Option<u32>,
    href: Option<String>,
    fragment: Option<String>,
    depth: u16,
    number: Option<u32>,
}

#[derive(Serialize)]
pub struct TocView {
    source: &'static str,
    entries: Vec<TocRow>,
    linear: Vec<u32>,
}

fn toc_row(reader: &Reader, entry: &TocEntry) -> TocRow {
    let row = entry
        .target
        .and_then(|ix| reader.rows().get(ix.0 as usize));
    TocRow {
        label: entry.label.chars().take(MAX_LABEL_CHARS).collect(),
        ix: entry.target.map(|ix| ix.0),
        href: row
            .and_then(|row| row.href.as_ref())
            .map(|href| href.as_str().to_owned()),
        fragment: entry.fragment.clone(),
        depth: entry.depth,
        number: row.and_then(|row| row.linear_ordinal),
    }
}

#[tauri::command(async)]
pub fn toc(session: State<'_, Session>) -> Result<TocView, String> {
    crate::mark!("toc_called");
    guarded(|| {
        let reader = session.reader()?;
        let toc = reader.toc().map_err(|error| error.user_message())?;
        Ok(TocView {
            source: match toc.source {
                TocSource::Nav => "nav",
                TocSource::Ncx => "ncx",
                TocSource::HeadingsPending => "headings",
            },
            entries: toc
                .entries
                .iter()
                .map(|entry| toc_row(&reader, entry))
                .collect(),
            linear: reader
                .rows()
                .iter()
                .filter(|row| row.linear_ordinal.is_some())
                .map(|row| row.ix.0)
                .collect(),
        })
    })
}

#[tauri::command(async)]
pub fn headings_start(session: State<'_, Session>, id: u64) -> Result<(), String> {
    guarded(|| {
        session.headings.start(session.reader()?, id);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn headings_next(session: State<'_, Session>, id: u64) -> HeadingBatch {
    session.headings.next(id)
}

#[tauri::command(async)]
pub fn headings_cancel(session: State<'_, Session>, id: u64) {
    session.headings.cancel(id);
}

#[tauri::command]
pub fn prefs_get(session: State<'_, Session>) -> Prefs {
    session.saved.prefs()
}

#[tauri::command(async)]
pub fn prefs_set(session: State<'_, Session>, prefs: Prefs) -> Result<(), String> {
    session.saved.set_prefs(prefs)
}

#[tauri::command(async)]
pub fn searches_get(session: State<'_, Session>) -> SearchMemory {
    session.saved.searches()
}

#[tauri::command(async)]
pub fn searches_set(session: State<'_, Session>, memory: SearchMemory) -> Result<(), String> {
    session.saved.set_searches(&memory)
}

#[tauri::command(async)]
pub fn save_place(
    session: State<'_, Session>,
    ix: u32,
    point: Option<DomPoint>,
    writing: bool,
) -> Result<(), String> {
    guarded(|| session.save_place(ChapterIx(ix), point, writing))
}

#[derive(Serialize)]
pub struct SavedView {
    rev: ChapterRev,
    wrote: bool,
    after: Option<Vec<AfterBlock>>,
    warnings: Vec<String>,
    #[serde(rename = "fixOffers")]
    fix_offers: Vec<FixOffer>,
    contents: bool,
}

#[derive(Serialize)]
pub struct SaveFailure {
    kind: &'static str,
    message: String,
}

impl From<Error> for SaveFailure {
    fn from(error: Error) -> SaveFailure {
        let kind = match &error {
            Error::ChangedOnDisk => "changedOnDisk",
            Error::PlanStale => "planStale",
            Error::NotEditable(_) => "notEditable",
            Error::WouldBreak(_) => "wouldBreak",
            Error::SaveFailed(_) => "saveFailed",
            Error::Cancelled => "cancelled",
            Error::Book(_) | Error::Limit(_) | Error::Io(_) => "other",
        };
        SaveFailure {
            kind,
            message: format!("{} Your text is still on screen.", error.user_message()),
        }
    }
}

impl From<String> for SaveFailure {
    fn from(message: String) -> SaveFailure {
        SaveFailure {
            kind: "other",
            message: format!("{message} Your text is still on screen."),
        }
    }
}

#[tauri::command(async)]
pub fn save_chapter(
    session: State<'_, Session>,
    ix: u32,
    edits: BlockEdits,
) -> Result<SavedView, SaveFailure> {
    crate::mark!("save_chapter");
    #[cfg(feature = "e2e")]
    crate::e2e::injected_save_failure()?;
    let book = session.book()?;
    let attempt = std::panic::AssertUnwindSafe(|| book.save_chapter(ChapterIx(ix), &edits));
    let saved = std::panic::catch_unwind(attempt)
        .map_err(|_| "Something went wrong while saving. The book was not changed.".to_owned())??;
    if matches!(saved.outcome, SaveOutcome::Written(_)) {
        session.count_words_later(&book);
    }
    Ok(SavedView {
        rev: saved.rev,
        wrote: matches!(saved.outcome, SaveOutcome::Written(_)),
        after: saved.after,
        warnings: saved.warnings,
        fix_offers: saved.fix_offers,
        contents: book
            .read()
            .rows()
            .get(ix as usize)
            .is_some_and(|row| row.is_nav),
    })
}

#[derive(Serialize)]
pub struct RecoveredDraft {
    ix: u32,
    edits: BlockEdits,
}

#[tauri::command(async)]
pub fn draft_write(session: State<'_, Session>, ix: u32, edits: BlockEdits) -> Result<(), String> {
    crate::mark!("draft_write");
    guarded(|| {
        session
            .book()?
            .write_draft(ChapterIx(ix), &edits)
            .map_err(|error| error.user_message())
    })
}

#[tauri::command(async)]
pub fn draft_offer(session: State<'_, Session>) -> Result<Option<DraftOffer>, String> {
    guarded(|| Ok(session.book()?.draft_offer()))
}

#[tauri::command(async)]
pub fn draft_recover(session: State<'_, Session>) -> Result<RecoveredDraft, String> {
    guarded(|| {
        let (ix, edits) = session
            .book()?
            .recover_draft()
            .map_err(|error| error.user_message())?;
        Ok(RecoveredDraft { ix: ix.0, edits })
    })
}

#[tauri::command(async)]
pub fn draft_discard(session: State<'_, Session>) -> Result<(), String> {
    guarded(|| {
        session
            .book()?
            .discard_draft()
            .map_err(|error| error.user_message())
    })
}

#[tauri::command(async)]
pub fn draft_drop(session: State<'_, Session>) -> Result<(), String> {
    guarded(|| {
        session
            .book()?
            .drop_draft()
            .map_err(|error| error.user_message())
    })
}

#[tauri::command(async)]
pub fn search_start(
    session: State<'_, Session>,
    id: u64,
    request: SearchRequest,
    scope: SearchScope,
) -> Result<(), String> {
    guarded(|| {
        session.searches.start(session.book()?, id, request, scope);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn search_next(session: State<'_, Session>, id: u64) -> SearchBatch {
    session.searches.next(id)
}

#[tauri::command(async)]
pub fn search_cancel(session: State<'_, Session>, id: u64) {
    session.searches.cancel(id);
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    Pane,
    Offer,
    Clean,
}

fn searches_of(session: &Session, slot: Slot) -> &Searches {
    match slot {
        Slot::Pane => &session.searches,
        Slot::Offer => &session.offers,
        Slot::Clean => &session.cleans,
    }
}

#[derive(Serialize)]
pub struct CleanupRuleView {
    rule: CleanRule,
    label: &'static str,
    detail: &'static str,
}

#[tauri::command]
pub fn cleanup_rules() -> Vec<CleanupRuleView> {
    RULES
        .iter()
        .map(|spec| CleanupRuleView {
            rule: spec.rule,
            label: spec.label,
            detail: spec.detail,
        })
        .collect()
}

#[tauri::command(async)]
pub fn replace_start(
    session: State<'_, Session>,
    slot: Slot,
    id: u64,
    command: ReplaceCommand,
    scope: SearchScope,
) -> Result<(), String> {
    guarded(|| {
        searches_of(&session, slot).start_preview(session.book()?, id, command, scope);
        Ok(())
    })
}

#[tauri::command(async)]
pub fn replace_next(session: State<'_, Session>, slot: Slot, id: u64) -> SearchBatch {
    searches_of(&session, slot).next(id)
}

#[tauri::command(async)]
pub fn replace_cancel(session: State<'_, Session>, slot: Slot, id: u64) {
    searches_of(&session, slot).cancel(id);
}

#[tauri::command(async)]
pub fn apply_start(
    session: State<'_, Session>,
    slot: Slot,
    preview: u64,
    id: u64,
    exclude: Vec<ReplaceExclusion>,
) -> Result<(), String> {
    guarded(|| {
        session.applies.start(
            session.book()?,
            searches_of(&session, slot),
            preview,
            id,
            exclude,
        );
        Ok(())
    })
}

#[tauri::command(async)]
pub fn apply_next(session: State<'_, Session>, id: u64) -> ApplyBatch {
    session.applies.next(id)
}

#[tauri::command(async)]
pub fn apply_cancel(session: State<'_, Session>, id: u64) {
    session.applies.cancel(id);
}

const MAX_LOCATED_RANGES: usize = 5_000;

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Located {
    Ranges { ranges: Vec<DomRange> },
    Stale,
}

#[tauri::command(async)]
pub fn locate_hits(
    session: State<'_, Session>,
    ix: u32,
    rev: ChapterRev,
    mut ranges: Vec<VisRange>,
) -> Result<Located, String> {
    guarded(|| {
        let reader = session.reader()?;
        let chapter = ChapterIx(ix);
        if reader.rows().get(ix as usize).and_then(|row| row.rev) != Some(rev) {
            return Ok(Located::Stale);
        }
        ranges.truncate(MAX_LOCATED_RANGES);
        let ranges = reader
            .locate_many(chapter, &ranges)
            .map_err(|error| error.user_message())?;
        Ok(Located::Ranges { ranges })
    })
}

#[tauri::command(async)]
pub fn reload_book(session: State<'_, Session>) -> Result<(), String> {
    guarded(|| session.reload())
}

#[tauri::command(async)]
pub fn changed_on_disk(session: State<'_, Session>) -> Option<String> {
    let ready = session.current()?;
    ready
        .book()
        .changed_on_disk()
        .then(|| Error::ChangedOnDisk.user_message())
}

#[tauri::command]
pub fn set_close_guard(session: State<'_, Session>, editing: bool) {
    if let Ok(mut guard) = session.close.lock() {
        guard.editing = editing;
        guard.asked_at = None;
    }
}

#[tauri::command]
pub fn close_asked(session: State<'_, Session>) {
    if let Ok(mut guard) = session.close.lock() {
        guard.asked_at = None;
    }
}

#[tauri::command]
pub fn close_window(session: State<'_, Session>, window: WebviewWindow) -> Result<(), String> {
    if let Ok(mut guard) = session.close.lock() {
        guard.allowed = true;
    }
    window.close().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_fullscreen(window: WebviewWindow, on: bool) -> Result<(), String> {
    window.set_fullscreen(on).map_err(|error| error.to_string())
}

const FLUSH_WAIT: std::time::Duration = std::time::Duration::from_millis(400);

fn tell_page(window: &Window, event: &str) {
    if let Some(webview) = window.get_webview_window(window.label()) {
        let _ = webview.eval(format!("window.dispatchEvent(new Event('{event}'))"));
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
            crate::library::receive_drop(&window.state::<Session>(), paths.clone());
            tell_page(window, "ss-book-dropped");
        }
        WindowEvent::CloseRequested { api, .. } => {
            let session = window.state::<Session>();
            let Ok(mut guard) = session.close.lock() else {
                return;
            };
            match guard.step(Instant::now()) {
                CloseStep::Close => {}
                CloseStep::AskPage => {
                    api.prevent_close();
                    tell_page(window, "ss-close-request");
                }
                CloseStep::FlushPlace => {
                    api.prevent_close();
                    tell_page(window, "ss-close-flush");
                    let late = window.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(FLUSH_WAIT);
                        let _ = late.close();
                    });
                }
            }
        }
        _ => {}
    }
}
