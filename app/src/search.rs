use serde::{Deserialize, Serialize};
use jadeslip_core::{
    AppliedReplace, Book, CancelToken, Case, ChapterRev, CleanRule, Error, FixOffer, Hit, Query,
    Reader, ReplaceExclusion, ReplacePreview, ReplaceRequest, RuleTally, SearchEnd, SearchEvent,
    SearchOptions, SearchScope, TouchedChapter,
};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const MAX_HITS: u32 = 100_000;
const MAX_PREVIEW_ROWS: u32 = 5_000;
const MAX_CLEAN_ROWS: u32 = 20_000;
const MAX_BATCH_HITS: usize = 4096;
const MAX_TITLE_CHARS: usize = 200;
const LONG_POLL: Duration = Duration::from_millis(40);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    text: String,
    regex: bool,
    case_sensitive: bool,
    whole_word: bool,
}

impl SearchRequest {
    fn query(self) -> Query {
        let case = if self.case_sensitive {
            Case::Sensitive
        } else {
            Case::Insensitive
        };
        if self.regex {
            Query::Regex {
                pattern: self.text,
                case,
            }
        } else {
            Query::Literal {
                text: self.text,
                case,
                whole_word: self.whole_word,
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReplaceCommand {
    #[serde(rename_all = "camelCase")]
    Text {
        text: String,
        regex: bool,
        case_sensitive: bool,
        replacement: String,
    },
    Fix {
        from: String,
        to: String,
    },
    Clean {
        rules: Vec<CleanRule>,
    },
}

impl ReplaceCommand {
    fn max_rows(&self) -> u32 {
        match self {
            ReplaceCommand::Clean { .. } => MAX_CLEAN_ROWS,
            ReplaceCommand::Text { .. } | ReplaceCommand::Fix { .. } => MAX_PREVIEW_ROWS,
        }
    }

    fn request(self) -> ReplaceRequest {
        match self {
            ReplaceCommand::Fix { from, to } => FixOffer { from, to }.request(),
            ReplaceCommand::Clean { rules } => ReplaceRequest::clean(rules),
            ReplaceCommand::Text {
                text,
                regex,
                case_sensitive,
                replacement,
            } => {
                let query = SearchRequest {
                    text,
                    regex,
                    case_sensitive,
                    whole_word: false,
                }
                .query();
                ReplaceRequest::new(query, replacement)
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct PreviewView {
    matches: u64,
    changes: u64,
    chapters: u32,
    skipped: u64,
    spanning: u64,
    limited: u32,
    unreadable: u32,
    touched: Vec<TouchedChapter>,
    rules: Vec<RuleTally>,
}

impl From<&ReplacePreview> for PreviewView {
    fn from(preview: &ReplacePreview) -> PreviewView {
        PreviewView {
            matches: preview.matches,
            changes: preview.changes,
            chapters: preview.chapters,
            skipped: preview.skipped,
            spanning: preview.spanning,
            limited: preview.limited,
            unreadable: preview.unreadable,
            touched: preview.touched.clone(),
            rules: preview.rules.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct GroupView {
    ix: u32,
    rev: ChapterRev,
    number: Option<u32>,
    title: Option<String>,
    hits: Vec<Hit>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EndView {
    Done,
    Cancelled,
    BadPattern { message: String },
    TooSlow { limited: u32 },
    Failed { message: String },
}

impl From<SearchEnd> for EndView {
    fn from(end: SearchEnd) -> EndView {
        match end {
            SearchEnd::Done => EndView::Done,
            SearchEnd::Cancelled => EndView::Cancelled,
            SearchEnd::BadPattern(message) => EndView::BadPattern { message },
            SearchEnd::TooSlow { limited } => EndView::TooSlow { limited },
            SearchEnd::Failed(message) => EndView::Failed { message },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchBatch {
    groups: Vec<GroupView>,
    done: u32,
    total: u32,
    limited: u32,
    unreadable: u32,
    capped: bool,
    preview: Option<PreviewView>,
    end: Option<EndView>,
}

#[derive(Default)]
struct Inbox {
    groups: VecDeque<GroupView>,
    done: u32,
    total: u32,
    limited: u32,
    unreadable: u32,
    capped: bool,
    preview: Option<PreviewView>,
    end: Option<EndView>,
}

impl Inbox {
    fn take_batch(&mut self) -> Vec<GroupView> {
        let mut batch = Vec::new();
        let mut room = MAX_BATCH_HITS;
        while room > 0 {
            let Some(group) = self.groups.front_mut() else {
                break;
            };
            if group.hits.len() <= room {
                room -= group.hits.len();
                batch.extend(self.groups.pop_front());
            } else {
                let rest = group.hits.split_off(room);
                let front = std::mem::replace(&mut group.hits, rest);
                batch.push(GroupView {
                    ix: group.ix,
                    rev: group.rev,
                    number: group.number,
                    title: group.title.clone(),
                    hits: front,
                });
                room = 0;
            }
        }
        batch
    }

    fn ended_and_empty(&self) -> Option<EndView> {
        if self.groups.is_empty() {
            self.end.clone()
        } else {
            None
        }
    }
}

struct SearchRun {
    id: u64,
    cancel: CancelToken,
    inbox: Mutex<Inbox>,
    arrived: Condvar,
    finished_preview: Mutex<Option<FinishedPreview>>,
}

#[derive(Clone)]
struct FinishedPreview {
    book: Arc<Book>,
    preview: Arc<ReplacePreview>,
}

impl SearchRun {
    fn inbox(&self) -> MutexGuard<'_, Inbox> {
        self.inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut Inbox)) {
        change(&mut self.inbox());
        self.arrived.notify_all();
    }
}

#[derive(Default)]
pub struct Searches {
    current: Mutex<Option<Arc<SearchRun>>>,
}

fn titles_of(reader: &Reader<File>) -> HashMap<u32, String> {
    let mut titles = HashMap::new();
    if let Ok(toc) = reader.toc() {
        for entry in &toc.entries {
            if let Some(target) = entry.target {
                titles
                    .entry(target.0)
                    .or_insert_with(|| entry.label.chars().take(MAX_TITLE_CHARS).collect());
            }
        }
    }
    titles
}

fn options() -> SearchOptions {
    let worker_exe = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(format!("jadeslip-worker{}", std::env::consts::EXE_SUFFIX)));
    SearchOptions {
        max_hits: MAX_HITS,
        worker_exe,
        #[cfg(feature = "e2e")]
        delay: crate::e2e::search_delay(),
        ..SearchOptions::default()
    }
}

fn listen<'a>(
    run: &'a SearchRun,
    reader: &'a Reader<File>,
    titles: &'a HashMap<u32, String>,
) -> impl FnMut(SearchEvent) + 'a {
    let rows = reader.rows();
    let number_of = |ix: u32| rows.get(ix as usize).and_then(|row| row.linear_ordinal);
    move |event| match event {
        SearchEvent::Hits { chapter, rev, hits } => run.update(|inbox| {
            inbox.groups.push_back(GroupView {
                ix: chapter.0,
                rev,
                number: number_of(chapter.0),
                title: titles.get(&chapter.0).cloned(),
                hits,
            });
        }),
        SearchEvent::Progress { done, total } => run.update(|inbox| {
            inbox.done = done;
            inbox.total = total;
        }),
        SearchEvent::ChapterLimited { .. } => run.update(|inbox| inbox.limited += 1),
        SearchEvent::ChapterUnreadable { .. } => run.update(|inbox| inbox.unreadable += 1),
        SearchEvent::CapReached => run.update(|inbox| inbox.capped = true),
    }
}

fn run_search(book: &Book, query: &Query, scope: SearchScope, run: &SearchRun) -> SearchEnd {
    let reader = book.read();
    let titles = titles_of(&reader);
    book.search(
        query,
        scope,
        &options(),
        &run.cancel,
        &mut listen(run, &reader, &titles),
    )
}

fn run_preview(
    book: &Arc<Book>,
    request: &ReplaceRequest,
    scope: SearchScope,
    max_rows: u32,
    run: &SearchRun,
) -> SearchEnd {
    let reader = book.read();
    let titles = titles_of(&reader);
    let shown = book.preview_replace(
        request,
        scope,
        max_rows,
        &options(),
        &run.cancel,
        &mut listen(run, &reader, &titles),
    );
    match shown {
        Ok(preview) => {
            let view = PreviewView::from(&preview);
            let limited = preview.limited;
            *lock_preview(run) = Some(FinishedPreview {
                book: book.clone(),
                preview: Arc::new(preview),
            });
            run.update(|inbox| inbox.preview = Some(view));
            if limited > 0 {
                SearchEnd::TooSlow { limited }
            } else {
                SearchEnd::Done
            }
        }
        Err(Error::Cancelled) => SearchEnd::Cancelled,
        Err(error) => SearchEnd::BadPattern(error.user_message()),
    }
}

fn lock_preview(run: &SearchRun) -> MutexGuard<'_, Option<FinishedPreview>> {
    run.finished_preview
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Searches {
    fn current(&self) -> MutexGuard<'_, Option<Arc<SearchRun>>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn start(&self, book: Arc<Book>, id: u64, request: SearchRequest, scope: SearchScope) {
        self.launch(id, move |run| {
            let query = request.query();
            run_search(&book, &query, scope, run)
        });
    }

    pub fn start_preview(
        &self,
        book: Arc<Book>,
        id: u64,
        command: ReplaceCommand,
        scope: SearchScope,
    ) {
        self.launch(id, move |run| {
            let max_rows = command.max_rows();
            let request = command.request();
            run_preview(&book, &request, scope, max_rows, run)
        });
    }

    fn launch(&self, id: u64, work: impl FnOnce(&SearchRun) -> SearchEnd + Send + 'static) {
        let run = Arc::new(SearchRun {
            id,
            cancel: CancelToken::default(),
            inbox: Mutex::new(Inbox::default()),
            arrived: Condvar::new(),
            finished_preview: Mutex::new(None),
        });
        {
            let mut current = self.current();
            if current.as_ref().is_some_and(|running| running.id > id) {
                return;
            }
            if let Some(previous) = current.replace(run.clone()) {
                previous.cancel.cancel();
            }
        }
        std::thread::spawn(move || {
            let attempt = std::panic::AssertUnwindSafe(|| work(&run));
            let end = std::panic::catch_unwind(attempt).unwrap_or_else(|_| {
                SearchEnd::Failed("Something went wrong during this search.".to_owned())
            });
            run.update(|inbox| inbox.end = Some(end.into()));
        });
    }

    pub fn cancel(&self, id: u64) {
        if let Some(run) = self.current().as_ref().filter(|run| run.id == id) {
            run.cancel.cancel();
        }
    }

    pub fn next(&self, id: u64) -> SearchBatch {
        let run = self.current().as_ref().filter(|run| run.id == id).cloned();
        let Some(run) = run else {
            return SearchBatch {
                groups: Vec::new(),
                done: 0,
                total: 0,
                limited: 0,
                unreadable: 0,
                capped: false,
                preview: None,
                end: Some(EndView::Cancelled),
            };
        };
        let until = Instant::now() + LONG_POLL;
        let mut inbox = run.inbox();
        while inbox.groups.is_empty() && inbox.end.is_none() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            inbox = run
                .arrived
                .wait_timeout(inbox, until - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        let groups = inbox.take_batch();
        SearchBatch {
            groups,
            done: inbox.done,
            total: inbox.total,
            limited: inbox.limited,
            unreadable: inbox.unreadable,
            capped: inbox.capped,
            preview: inbox.ended_and_empty().and(inbox.preview.clone()),
            end: inbox.ended_and_empty(),
        }
    }

    fn preview_of(&self, id: u64) -> Option<FinishedPreview> {
        let run = self
            .current()
            .as_ref()
            .filter(|run| run.id == id)
            .cloned()?;
        lock_preview(&run).clone()
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApplyEnd {
    Done {
        changes: u64,
        titles: u64,
        chapters: u32,
        warnings: Vec<String>,
    },
    Cancelled,
    Failed {
        reason: &'static str,
        message: String,
    },
}

const NO_PREVIEW: &str =
    "This preview is no longer available, so nothing was changed. Run it again.";
const STALE_PREVIEW: &str = "The book was saved after this preview, so nothing was changed. Check the new preview and apply again.";

fn end_of(result: Result<AppliedReplace, Error>) -> ApplyEnd {
    match result {
        Ok(applied) => ApplyEnd::Done {
            changes: applied.changes,
            titles: applied.titles,
            chapters: applied.chapters,
            warnings: applied.report.warnings,
        },
        Err(Error::Cancelled) => ApplyEnd::Cancelled,
        Err(Error::PlanStale) => ApplyEnd::Failed {
            reason: "stale",
            message: STALE_PREVIEW.to_owned(),
        },
        Err(Error::ChangedOnDisk) => ApplyEnd::Failed {
            reason: "changedOnDisk",
            message: Error::ChangedOnDisk.user_message(),
        },
        Err(error) => ApplyEnd::Failed {
            reason: "other",
            message: error.user_message(),
        },
    }
}

#[derive(Default)]
struct ApplyStatus {
    done: u32,
    total: u32,
    end: Option<ApplyEnd>,
}

struct ApplyRun {
    id: u64,
    cancel: CancelToken,
    status: Mutex<ApplyStatus>,
    arrived: Condvar,
}

impl ApplyRun {
    fn status(&self) -> MutexGuard<'_, ApplyStatus> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Serialize)]
pub struct ApplyBatch {
    done: u32,
    total: u32,
    end: Option<ApplyEnd>,
}

#[derive(Default)]
pub struct Applies {
    current: Mutex<Option<Arc<ApplyRun>>>,
}

impl Applies {
    fn current(&self) -> MutexGuard<'_, Option<Arc<ApplyRun>>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn start(
        &self,
        book: Arc<Book>,
        searches: &Searches,
        preview_id: u64,
        id: u64,
        exclude: Vec<ReplaceExclusion>,
    ) {
        let run = Arc::new(ApplyRun {
            id,
            cancel: CancelToken::default(),
            status: Mutex::new(ApplyStatus::default()),
            arrived: Condvar::new(),
        });
        *self.current() = Some(run.clone());
        let finished = searches.preview_of(preview_id);
        std::thread::spawn(move || {
            let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                apply_now(&book, finished, &exclude, &run)
            }))
            .unwrap_or_else(|_| ApplyEnd::Failed {
                reason: "other",
                message: "Something went wrong during this save. The book was not changed."
                    .to_owned(),
            });
            run.status().end = Some(end);
            run.arrived.notify_all();
        });
    }

    pub fn cancel(&self, id: u64) {
        if let Some(run) = self.current().as_ref().filter(|run| run.id == id) {
            run.cancel.cancel();
        }
    }

    pub fn next(&self, id: u64) -> ApplyBatch {
        let run = self.current().as_ref().filter(|run| run.id == id).cloned();
        let Some(run) = run else {
            return ApplyBatch {
                done: 0,
                total: 0,
                end: Some(ApplyEnd::Failed {
                    reason: "other",
                    message: NO_PREVIEW.to_owned(),
                }),
            };
        };
        let until = Instant::now() + LONG_POLL;
        let mut status = run.status();
        let seen = status.done;
        while status.done == seen && status.end.is_none() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            status = run
                .arrived
                .wait_timeout(status, until - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        ApplyBatch {
            done: status.done,
            total: status.total,
            end: status.end.clone(),
        }
    }
}

fn apply_now(
    book: &Arc<Book>,
    finished: Option<FinishedPreview>,
    exclude: &[ReplaceExclusion],
    run: &ApplyRun,
) -> ApplyEnd {
    let Some(finished) = finished else {
        return ApplyEnd::Failed {
            reason: "stale",
            message: NO_PREVIEW.to_owned(),
        };
    };
    if !Arc::ptr_eq(book, &finished.book) {
        return end_of(Err(Error::PlanStale));
    }
    #[cfg(feature = "e2e")]
    if let Some(message) = crate::e2e::injected_apply_failure() {
        return end_of(Err(Error::WouldBreak(message)));
    }
    end_of(book.apply_replace(
        &finished.preview,
        exclude,
        &options(),
        &run.cancel,
        &mut |done, total| {
            let mut status = run.status();
            status.done = done;
            status.total = total;
            run.arrived.notify_all();
        },
    ))
}
