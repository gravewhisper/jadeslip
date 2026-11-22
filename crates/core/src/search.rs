use crate::book::ChapterRow;
use crate::clean::{Change, Chapter, CleanRule, find};
use crate::replace::Replacer;
use crate::worker;
use crate::zip::ReadAt;
use crate::{ChapterIx, ChapterRev, Error, Reader, VisRange, VisibleText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const CONTEXT_CHARS: usize = 60;
const MATCHED_CHARS: usize = 200;
pub(crate) const MAX_QUERY_BYTES: usize = 16 << 10;
const REORDER_WINDOW: usize = 512;
const REORDER_BYTES: usize = 64 << 20;
pub(crate) const MODEL_BYTES_PER_SOURCE_BYTE: u64 = 6;
const HIT_OVERHEAD_BYTES: usize = 128;
const DEADLINE_NANOS_PER_BYTE: u64 = 100;
pub(crate) const POLL: Duration = Duration::from_millis(2);
pub(crate) const REPLACE_CHAPTER_CAP: u32 = 1_000_000;

pub const CHAPTER_BYTE_BUDGET: u64 = 512 << 20;
pub const DEFAULT_MEMORY_CAP: u64 = 1 << 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Case {
    Sensitive,
    Insensitive,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SearchScope {
    Whole,
    UpTo { here: ChapterIx },
    From { here: ChapterIx },
    Only { here: ChapterIx },
}

impl SearchScope {
    fn span(self, len: usize) -> Range<usize> {
        match self {
            SearchScope::Whole => 0..len,
            SearchScope::UpTo { here } => 0..(here.0 as usize + 1).min(len),
            SearchScope::From { here } => (here.0 as usize).min(len)..len,
            SearchScope::Only { here } => {
                (here.0 as usize).min(len)..(here.0 as usize + 1).min(len)
            }
        }
    }

    pub(crate) fn rows<R: ReadAt>(self, reader: &Reader<R>) -> impl Iterator<Item = &ChapterRow> {
        let rows = reader.rows();
        rows[self.span(rows.len())].iter().filter(|row| !row.is_nav)
    }
}

#[derive(Clone, Debug)]
pub enum Query {
    Literal {
        text: String,
        case: Case,
        whole_word: bool,
    },
    Regex {
        pattern: String,
        case: Case,
    },
    Clean {
        rules: Vec<CleanRule>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hit {
    pub range: VisRange,
    pub before: String,
    pub matched: String,
    pub after: String,
    pub replaced: Option<String>,
    pub spans_tags: bool,
    pub crosses_block: bool,
    pub same: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<CleanRule>,
}

impl Hit {
    fn bytes(&self) -> usize {
        let replaced = self.replaced.as_ref().map_or(0, String::len);
        HIT_OVERHEAD_BYTES + self.before.len() + self.matched.len() + self.after.len() + replaced
    }
}

#[derive(Debug)]
pub enum SearchEvent {
    Hits {
        chapter: ChapterIx,
        rev: ChapterRev,
        hits: Vec<Hit>,
    },
    Progress {
        done: u32,
        total: u32,
    },
    ChapterLimited {
        chapter: ChapterIx,
    },
    ChapterUnreadable {
        chapter: ChapterIx,
        message: String,
    },
    CapReached,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchEnd {
    Done,
    Cancelled,
    BadPattern(String),
    TooSlow { limited: u32 },
    Failed(String),
}

pub type ChapterDelay = Arc<dyn Fn(ChapterIx) -> Duration + Send + Sync>;

#[derive(Clone)]
pub struct SearchOptions {
    pub max_hits: u32,
    pub threads: usize,
    pub worker_exe: Option<PathBuf>,
    pub chapter_deadline: Duration,
    pub book_deadline: Duration,
    pub memory_cap: u64,
    pub delay: Option<ChapterDelay>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            max_hits: 100_000,
            threads: std::thread::available_parallelism()
                .map_or(1, |count| count.get().saturating_sub(2).max(1)),
            worker_exe: None,
            chapter_deadline: Duration::from_millis(250),
            book_deadline: Duration::from_secs(5),
            memory_cap: DEFAULT_MEMORY_CAP,
            delay: None,
        }
    }
}

#[derive(Default)]
struct BudgetUse {
    used: u64,
    high_water: u64,
}

pub struct ByteBudget {
    usage: Mutex<BudgetUse>,
    freed: Condvar,
    cap: u64,
}

impl ByteBudget {
    pub fn new(cap: u64) -> Arc<ByteBudget> {
        Arc::new(ByteBudget {
            usage: Mutex::default(),
            freed: Condvar::new(),
            cap,
        })
    }

    pub fn high_water(&self) -> u64 {
        lock(&self.usage).high_water
    }

    pub(crate) fn acquire(self: &Arc<Self>, cost: u64, stopped: &AtomicBool) -> Option<Permit> {
        let cost = cost.min(self.cap);
        let mut usage = lock(&self.usage);
        while usage.used + cost > self.cap {
            if stopped.load(Ordering::Relaxed) {
                return None;
            }
            usage = self
                .freed
                .wait_timeout(usage, POLL)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        usage.used += cost;
        usage.high_water = usage.high_water.max(usage.used);
        Some(Permit {
            budget: self.clone(),
            cost,
        })
    }
}

pub(crate) struct Permit {
    budget: Arc<ByteBudget>,
    cost: u64,
}

impl Drop for Permit {
    fn drop(&mut self) {
        lock(&self.budget.usage).used -= self.cost;
        self.budget.freed.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) enum Outcome {
    Hits(Vec<Hit>),
    Limited(Vec<Hit>),
    Unreadable(String),
}

impl Outcome {
    fn bytes(&self) -> usize {
        match self {
            Outcome::Hits(hits) | Outcome::Limited(hits) => hits.iter().map(Hit::bytes).sum(),
            Outcome::Unreadable(message) => message.len(),
        }
    }
}

struct Schedule {
    next: usize,
    head: usize,
    finished: BTreeMap<usize, Outcome>,
    buffered: usize,
    early_end: Option<SearchEnd>,
}

pub(crate) struct ReplaceScan {
    pub(crate) replacer: Replacer,
    pub(crate) row_limit: u32,
}

pub(crate) struct RunPlan {
    pub(crate) chapters: Vec<ChapterIx>,
    pub(crate) scan: Option<Arc<ReplaceScan>>,
}

pub(crate) struct Run {
    pub(crate) reader: Arc<Reader<File>>,
    pub(crate) options: SearchOptions,
    pub(crate) scan: Option<Arc<ReplaceScan>>,
    chapters: Vec<ChapterIx>,
    budget: Arc<ByteBudget>,
    stopped: AtomicBool,
    schedule: Mutex<Schedule>,
    work: Condvar,
    ready: Condvar,
}

impl Run {
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _guard = lock(&self.schedule);
        self.work.notify_all();
        self.ready.notify_all();
    }

    pub(crate) fn take(&self) -> Option<(usize, ChapterIx)> {
        let mut schedule = lock(&self.schedule);
        loop {
            if self.is_stopped() || schedule.next >= self.chapters.len() {
                return None;
            }
            let in_window = schedule.next < schedule.head + REORDER_WINDOW;
            let has_room = schedule.buffered < REORDER_BYTES || schedule.next == schedule.head;
            if in_window && has_room {
                let position = schedule.next;
                schedule.next += 1;
                return Some((position, self.chapters[position]));
            }
            schedule = self
                .work
                .wait(schedule)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn inflated(&self, chapter: ChapterIx) -> u64 {
        self.reader.rows()[chapter.0 as usize]
            .rev
            .map_or(0, |rev| u64::from(rev.inflated_len))
    }

    pub(crate) fn permit_for(&self, chapter: ChapterIx) -> Option<Permit> {
        self.budget.acquire(
            self.inflated(chapter) * MODEL_BYTES_PER_SOURCE_BYTE,
            &self.stopped,
        )
    }

    pub(crate) fn deadline_for(&self, chapter: ChapterIx) -> Duration {
        self.options.chapter_deadline
            + Duration::from_nanos(self.inflated(chapter) * DEADLINE_NANOS_PER_BYTE)
    }

    pub(crate) fn hits_wanted(&self) -> u32 {
        if self.scan.is_some() {
            REPLACE_CHAPTER_CAP + 1
        } else {
            self.options.max_hits
        }
    }

    pub(crate) fn put(&self, position: usize, outcome: Outcome) {
        let over_cap = self.scan.is_some()
            && matches!(&outcome, Outcome::Hits(hits) if hits.len() > REPLACE_CHAPTER_CAP as usize);
        let outcome = if over_cap {
            Outcome::Limited(Vec::new())
        } else {
            outcome
        };
        let mut schedule = lock(&self.schedule);
        schedule.buffered += outcome.bytes();
        schedule.finished.insert(position, outcome);
        self.ready.notify_all();
    }

    pub(crate) fn end_early(&self, end: SearchEnd) {
        let mut schedule = lock(&self.schedule);
        schedule.early_end.get_or_insert(end);
        self.ready.notify_all();
    }

    fn drain(&self) -> (Vec<(usize, Outcome)>, Option<SearchEnd>) {
        let mut schedule = lock(&self.schedule);
        let at_head = |schedule: &Schedule| schedule.finished.contains_key(&schedule.head);
        if !at_head(&schedule) && schedule.early_end.is_none() {
            schedule = self
                .ready
                .wait_timeout(schedule, POLL)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        let mut drained = Vec::new();
        while let Some(outcome) = {
            let head = schedule.head;
            schedule.finished.remove(&head)
        } {
            schedule.buffered -= outcome.bytes();
            drained.push((schedule.head, outcome));
            schedule.head += 1;
        }
        if !drained.is_empty() {
            self.work.notify_all();
        }
        (drained, schedule.early_end.clone())
    }

    fn take_remaining(&self) -> Vec<(usize, Option<Outcome>)> {
        let mut schedule = lock(&self.schedule);
        let head = schedule.head;
        schedule.head = self.chapters.len();
        (head..self.chapters.len())
            .map(|position| (position, schedule.finished.remove(&position)))
            .collect()
    }
}

struct Stop(Arc<Run>);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

fn snippet_before(text: &str, end: usize) -> &str {
    let start = text[..end]
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS - 1)
        .map_or(0, |(at, _)| at);
    &text[start..end]
}

fn snippet_after(text: &str, start: usize) -> &str {
    let rest = &text[start..];
    let end = rest
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(rest.len(), |(at, _)| at);
    &rest[..end]
}

pub(crate) fn snippet_matched(text: &str) -> &str {
    let end = text
        .char_indices()
        .nth(MATCHED_CHARS)
        .map_or(text.len(), |(at, _)| at);
    &text[..end]
}

pub(crate) fn hit_at(visible: &VisibleText, start: usize, end: usize) -> Hit {
    let text = visible.as_str();
    Hit {
        range: VisRange {
            start: start as u32,
            end: end as u32,
        },
        before: snippet_before(text, start).to_owned(),
        matched: snippet_matched(&text[start..end]).to_owned(),
        after: snippet_after(text, end).to_owned(),
        replaced: None,
        spans_tags: visible.spans_tags(start, end),
        crosses_block: visible.crosses_block(start, end),
        same: false,
        rule: None,
    }
}

pub(crate) fn counted_hit(visible: &VisibleText, start: usize, end: usize) -> Hit {
    Hit {
        range: VisRange {
            start: start as u32,
            end: end as u32,
        },
        before: String::new(),
        matched: String::new(),
        after: String::new(),
        replaced: None,
        spans_tags: visible.spans_tags(start, end),
        crosses_block: visible.crosses_block(start, end),
        same: false,
        rule: None,
    }
}

fn compile_literal(text: &str, case: Case, whole_word: bool) -> Result<regex::Regex, String> {
    let escaped = regex::escape(text);
    let pattern = if whole_word {
        format!(r"\b{{start-half}}(?:{escaped})\b{{end-half}}")
    } else {
        escaped
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(case == Case::Insensitive)
        .build()
        .map_err(|_| "That search text is too long.".to_owned())
}

fn literal_chapter(run: &Run, regex: &regex::Regex, chapter: ChapterIx) -> Result<Vec<Hit>, Error> {
    let visible = run.reader.visible_text(chapter)?;
    let wanted = run.hits_wanted() as usize;
    let found = regex.find_iter(visible.as_str());
    let Some(scan) = &run.scan else {
        return Ok(found
            .take(wanted)
            .map(|found| hit_at(&visible, found.start(), found.end()))
            .collect());
    };
    let rows = scan.row_limit as usize;
    let mut hits = Vec::new();
    for found in found {
        let Some(text) = scan.replacer.literal_for(found.as_str()) else {
            continue;
        };
        let mut hit = if hits.len() < rows {
            let mut hit = hit_at(&visible, found.start(), found.end());
            hit.replaced = Some(text.to_owned());
            hit
        } else {
            counted_hit(&visible, found.start(), found.end())
        };
        hit.same = text == found.as_str();
        hits.push(hit);
        if hits.len() >= wanted {
            break;
        }
    }
    Ok(hits)
}

fn clean_chapter(run: &Run, rules: &[CleanRule], chapter: ChapterIx) -> Result<Vec<Hit>, Error> {
    let Some(model) = run.reader.model(chapter)? else {
        return Ok(Vec::new());
    };
    let found = Chapter::of(&model, &run.reader.limits())?;
    let rows = run
        .scan
        .as_ref()
        .map_or(usize::MAX, |scan| scan.row_limit as usize);
    let text = model.text.as_str();
    Ok(find(&found, rules)
        .into_iter()
        .enumerate()
        .map(|(row, change)| {
            let range = found.paras[change.para()].vis;
            let unit = &text[range.start as usize..range.end as usize];
            let replaced = match &change {
                Change::Retitle(fix) => fix.title.clone(),
                Change::Remove { .. } => String::new(),
            };
            Hit {
                range,
                before: String::new(),
                matched: if row < rows {
                    snippet_matched(unit).to_owned()
                } else {
                    String::new()
                },
                after: String::new(),
                replaced: Some(replaced),
                spans_tags: false,
                crosses_block: false,
                same: false,
                rule: Some(change.rule()),
            }
        })
        .collect())
}

fn chapter_thread(run: &Run, search: impl Fn(ChapterIx) -> Result<Vec<Hit>, Error>) {
    while let Some((position, chapter)) = run.take() {
        if let Some(delay) = &run.options.delay {
            std::thread::sleep(delay(chapter));
        }
        let Some(permit) = run.permit_for(chapter) else {
            return;
        };
        let attempt = catch_unwind(AssertUnwindSafe(|| search(chapter)));
        drop(permit);
        run.put(
            position,
            match attempt {
                Ok(Ok(hits)) => Outcome::Hits(hits),
                Ok(Err(error)) => Outcome::Unreadable(error.user_message()),
                Err(_) => Outcome::Unreadable("This chapter could not be searched.".to_owned()),
            },
        );
    }
}

fn start_literal(run: &Arc<Run>, regex: regex::Regex) {
    for _ in 0..run.options.threads.max(1) {
        let run = run.clone();
        let regex = regex.clone();
        std::thread::spawn(move || {
            chapter_thread(&run, |chapter| literal_chapter(&run, &regex, chapter));
        });
    }
}

fn start_clean(run: &Arc<Run>, rules: &Arc<Vec<CleanRule>>) {
    for _ in 0..run.options.threads.max(1) {
        let run = run.clone();
        let rules = rules.clone();
        std::thread::spawn(move || {
            chapter_thread(&run, |chapter| clean_chapter(&run, &rules, chapter));
        });
    }
}

struct Emitter<'a> {
    run: &'a Run,
    sink: &'a mut dyn FnMut(SearchEvent),
    total: u32,
    emitted_hits: u64,
    limited: u32,
    capped: bool,
    deadline: Duration,
}

impl Emitter<'_> {
    fn emit_hits(&mut self, chapter: ChapterIx, mut hits: Vec<Hit>) {
        let room = u64::from(self.run.options.max_hits) - self.emitted_hits;
        if hits.len() as u64 >= room {
            hits.truncate(room as usize);
            self.capped = true;
        }
        self.emitted_hits += hits.len() as u64;
        if hits.is_empty() {
            return;
        }
        if let Some(rev) = self.run.reader.rows()[chapter.0 as usize].rev {
            (self.sink)(SearchEvent::Hits { chapter, rev, hits });
        }
    }

    fn emit_outcome(&mut self, position: usize, outcome: Outcome) {
        let chapter = self.run.chapters[position];
        match outcome {
            Outcome::Hits(hits) => self.emit_hits(chapter, hits),
            Outcome::Limited(hits) => {
                self.emit_hits(chapter, hits);
                self.limited += 1;
                (self.sink)(SearchEvent::ChapterLimited { chapter });
            }
            Outcome::Unreadable(message) => {
                (self.sink)(SearchEvent::ChapterUnreadable { chapter, message });
            }
        }
    }

    fn finish_too_slow(&mut self) -> SearchEnd {
        self.run.stop();
        for (position, outcome) in self.run.take_remaining() {
            self.emit_outcome(position, outcome.unwrap_or(Outcome::Limited(Vec::new())));
            if self.capped {
                break;
            }
        }
        (self.sink)(SearchEvent::Progress {
            done: self.total,
            total: self.total,
        });
        SearchEnd::TooSlow {
            limited: self.limited,
        }
    }

    fn end(&self) -> SearchEnd {
        if self.limited > 0 {
            SearchEnd::TooSlow {
                limited: self.limited,
            }
        } else {
            SearchEnd::Done
        }
    }

    fn drive(&mut self, cancel: &crate::CancelToken) -> SearchEnd {
        let started = Instant::now();
        let mut done = 0u32;
        loop {
            if cancel.is_cancelled() {
                return SearchEnd::Cancelled;
            }
            let (ready, early) = self.run.drain();
            if let Some(end) = early {
                return end;
            }
            let moved = !ready.is_empty();
            for (position, outcome) in ready {
                if cancel.is_cancelled() {
                    return SearchEnd::Cancelled;
                }
                self.emit_outcome(position, outcome);
                done += 1;
                if self.capped {
                    (self.sink)(SearchEvent::CapReached);
                    (self.sink)(SearchEvent::Progress {
                        done,
                        total: self.total,
                    });
                    return self.end();
                }
            }
            if moved {
                (self.sink)(SearchEvent::Progress {
                    done,
                    total: self.total,
                });
            }
            if done == self.total {
                return self.end();
            }
            if started.elapsed() >= self.deadline {
                return self.finish_too_slow();
            }
        }
    }
}

pub(crate) fn run_search(
    reader: Arc<Reader<File>>,
    budget: Arc<ByteBudget>,
    query: &Query,
    options: &SearchOptions,
    plan: RunPlan,
    cancel: &crate::CancelToken,
    sink: &mut dyn FnMut(SearchEvent),
) -> SearchEnd {
    let chapters = plan.chapters;
    let total = chapters.len() as u32;
    let run = Arc::new(Run {
        reader,
        options: options.clone(),
        scan: plan.scan,
        chapters,
        budget,
        stopped: AtomicBool::new(false),
        schedule: Mutex::new(Schedule {
            next: 0,
            head: 0,
            finished: BTreeMap::new(),
            buffered: 0,
            early_end: None,
        }),
        work: Condvar::new(),
        ready: Condvar::new(),
    });
    let _stop = Stop(run.clone());
    match query {
        Query::Literal { text, .. } | Query::Regex { pattern: text, .. }
            if text.len() > MAX_QUERY_BYTES =>
        {
            return SearchEnd::BadPattern("That search is too long.".to_owned());
        }
        Query::Literal { text, .. } | Query::Regex { pattern: text, .. } if text.is_empty() => {
            return SearchEnd::Done;
        }
        Query::Literal {
            text,
            case,
            whole_word,
        } => match compile_literal(text, *case, *whole_word) {
            Ok(regex) => start_literal(&run, regex),
            Err(message) => return SearchEnd::BadPattern(message),
        },
        Query::Regex { pattern, case } => worker::start_pool(&run, pattern.clone(), *case),
        Query::Clean { rules } if rules.is_empty() => return SearchEnd::Done,
        Query::Clean { rules } => start_clean(&run, &Arc::new(rules.clone())),
    }
    let mut emitter = Emitter {
        run: &run,
        sink,
        total,
        emitted_hits: 0,
        limited: 0,
        capped: false,
        deadline: match query {
            Query::Literal { .. } | Query::Clean { .. } => Duration::MAX,
            Query::Regex { .. } => options.book_deadline,
        },
    };
    emitter.drive(cancel)
}
