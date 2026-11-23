use crate::book::Reader;
use crate::chapter::{ChapterModel, decode_and_model};
use crate::clean::{self, CleanRule, RULES};
use crate::commit::{CommitReport, FileStamp};
use crate::edit::finish_bytes;
use crate::manage::{check_plan, encode_text};
use crate::markup::{Event, scan};
use crate::search::{
    ByteBudget, Case, Hit, MAX_QUERY_BYTES, MODEL_BYTES_PER_SOURCE_BYTE, POLL, Query, ReplaceScan,
    RunPlan, SearchEnd, SearchEvent, SearchOptions, SearchScope, run_search, snippet_matched,
};
use crate::zip::{Changes, EntryFate, EntryName, NewEntry, ZipEntry};
use crate::{Book, CancelToken, ChapterIx, Error, Limits};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

const MAX_OFFER_WORD_CHARS: usize = 64;
const MIN_OFFER_WORD_CHARS: usize = 2;
const MAX_OFFER_CHAPTER_BYTES: u64 = 4 << 20;
const WINDOW_CHAPTERS: usize = 256;
const WINDOW_BYTES: usize = 128 << 20;
const REBUILD_COST_FACTOR: u64 = 2;

#[derive(Clone, Debug)]
pub struct CaseVariant {
    pub matched: String,
    pub replacement: String,
}

#[derive(Clone, Debug)]
pub struct ReplaceRequest {
    pub query: Query,
    pub replacement: String,
    pub variants: Vec<CaseVariant>,
}

impl ReplaceRequest {
    pub fn new(query: Query, replacement: String) -> ReplaceRequest {
        ReplaceRequest {
            query,
            replacement,
            variants: Vec::new(),
        }
    }

    pub fn clean(rules: Vec<CleanRule>) -> ReplaceRequest {
        ReplaceRequest::new(Query::Clean { rules }, String::new())
    }

    fn replacer(&self) -> Replacer {
        match &self.query {
            Query::Literal { .. } => Replacer::Literal {
                text: self.replacement.clone(),
                variants: self.variants.clone(),
            },
            Query::Regex { .. } => Replacer::Template(self.replacement.clone()),
            Query::Clean { .. } => Replacer::Remove,
        }
    }

    fn check(&self) -> Result<(), Error> {
        if self.replacement.len() > MAX_QUERY_BYTES {
            return Err(Error::Book("That replacement is too long.".to_owned()));
        }
        let storable = |text: &str| text.chars().all(is_xml_char);
        let all_storable = storable(&self.replacement)
            && self
                .variants
                .iter()
                .all(|variant| storable(&variant.replacement));
        if !all_storable {
            return Err(Error::Book(
                "That replacement holds a character a book cannot store.".to_owned(),
            ));
        }
        Ok(())
    }
}

fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

pub(crate) enum Replacer {
    Literal {
        text: String,
        variants: Vec<CaseVariant>,
    },
    Template(String),
    Remove,
}

impl Replacer {
    pub(crate) fn literal_for(&self, matched: &str) -> Option<&str> {
        match self {
            Replacer::Literal { text, variants } if variants.is_empty() => Some(text),
            Replacer::Literal { variants, .. } => variants
                .iter()
                .find(|variant| variant.matched == matched)
                .map(|variant| variant.replacement.as_str()),
            Replacer::Template(_) | Replacer::Remove => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TouchedChapter {
    pub chapter: ChapterIx,
    pub matches: u32,
    pub changes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RuleTally {
    pub rule: CleanRule,
    pub matches: u64,
    pub chapters: u32,
}

pub struct ReplacePreview {
    request: ReplaceRequest,
    snapshot: FileStamp,
    pub matches: u64,
    pub changes: u64,
    pub chapters: u32,
    pub skipped: u64,
    pub spanning: u64,
    pub same: u64,
    pub limited: u32,
    pub unreadable: u32,
    pub touched: Vec<TouchedChapter>,
    pub rules: Vec<RuleTally>,
    titles: HashMap<ChapterIx, (u32, String)>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReplaceExclusion {
    Hit { chapter: ChapterIx, ordinal: u32 },
    Chapter { chapter: ChapterIx },
    Rule { rule: CleanRule },
}

#[derive(Debug)]
pub struct AppliedReplace {
    pub report: CommitReport,
    pub changes: u64,
    pub titles: u64,
    pub chapters: u32,
}

fn unique_chapters(reader: &Reader<File>, scope: SearchScope) -> Vec<ChapterIx> {
    let mut seen = HashSet::new();
    scope
        .rows(reader)
        .filter(|row| row.href.as_ref().is_none_or(|href| seen.insert(href)))
        .map(|row| row.ix)
        .collect()
}

fn too_slow() -> Error {
    Error::Limit(
        "This pattern was too slow or matched too often on some chapters, so nothing was saved."
            .to_owned(),
    )
}

fn unsafe_change(chapter: ChapterIx) -> Error {
    Error::WouldBreak(format!(
        "Chapter {} could not be changed safely, so nothing was saved.",
        u64::from(chapter.0) + 1
    ))
}

fn changed_since_preview() -> Error {
    Error::WouldBreak(
        "The matches came out different from the preview, so nothing was saved.".to_owned(),
    )
}

pub(crate) fn preview_on(
    (snapshot, reader, budget): (FileStamp, Arc<Reader<File>>, Arc<ByteBudget>),
    request: &ReplaceRequest,
    scope: SearchScope,
    max_rows: u32,
    options: &SearchOptions,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(SearchEvent),
) -> Result<ReplacePreview, Error> {
    let options = SearchOptions {
        max_hits: u32::MAX,
        ..options.clone()
    };
    let plan = RunPlan {
        chapters: unique_chapters(&reader, scope),
        scan: Some(Arc::new(ReplaceScan {
            replacer: request.replacer(),
            row_limit: max_rows,
        })),
    };
    let mut preview = ReplacePreview {
        request: request.clone(),
        snapshot,
        matches: 0,
        changes: 0,
        chapters: 0,
        skipped: 0,
        spanning: 0,
        same: 0,
        limited: 0,
        unreadable: 0,
        touched: Vec::new(),
        rules: tallies_for(&request.query),
        titles: HashMap::new(),
    };
    let mut rows_left = max_rows as usize;
    let end = run_search(
        reader,
        budget,
        &request.query,
        &options,
        plan,
        cancel,
        &mut |event| match event {
            SearchEvent::Hits {
                chapter,
                rev,
                mut hits,
            } => {
                preview.count(chapter, &hits);
                hits.truncate(rows_left);
                rows_left -= hits.len();
                for hit in &mut hits {
                    hit.replaced = hit
                        .replaced
                        .take()
                        .map(|text| snippet_matched(&text).to_owned());
                }
                if !hits.is_empty() {
                    sink(SearchEvent::Hits { chapter, rev, hits });
                }
            }
            SearchEvent::ChapterUnreadable { .. } => {
                preview.unreadable += 1;
                sink(event);
            }
            other => sink(other),
        },
    );
    match end {
        SearchEnd::Done => Ok(preview),
        SearchEnd::TooSlow { limited } => {
            preview.limited = limited;
            Ok(preview)
        }
        SearchEnd::Cancelled => Err(Error::Cancelled),
        SearchEnd::BadPattern(message) | SearchEnd::Failed(message) => Err(Error::Book(message)),
    }
}

fn tallies_for(query: &Query) -> Vec<RuleTally> {
    let Query::Clean { rules } = query else {
        return Vec::new();
    };
    RULES
        .iter()
        .filter(|spec| rules.contains(&spec.rule))
        .map(|spec| RuleTally {
            rule: spec.rule,
            matches: 0,
            chapters: 0,
        })
        .collect()
}

impl Book {
    pub fn preview_replace(
        &self,
        request: &ReplaceRequest,
        scope: SearchScope,
        max_rows: u32,
        options: &SearchOptions,
        cancel: &CancelToken,
        sink: &mut dyn FnMut(SearchEvent),
    ) -> Result<ReplacePreview, Error> {
        request.check()?;
        let (snapshot, reader) = self.snapshot();
        preview_on(
            (snapshot, reader, self.budget()),
            request,
            scope,
            max_rows,
            options,
            cancel,
            sink,
        )
    }

    pub fn apply_replace(
        &self,
        preview: &ReplacePreview,
        exclude: &[ReplaceExclusion],
        options: &SearchOptions,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(u32, u32),
    ) -> Result<AppliedReplace, Error> {
        if preview.limited > 0 {
            return Err(too_slow());
        }
        let (stamp, reader) = self.snapshot();
        if stamp != preview.snapshot {
            return Err(Error::PlanStale);
        }
        let plan = ApplyPlan::new(reader.clone(), preview, exclude);
        if plan.worklist.is_empty() {
            return Ok(AppliedReplace {
                report: CommitReport::nothing_to_do(),
                changes: 0,
                titles: 0,
                chapters: 0,
            });
        }
        let line = Pipeline::default();
        let threads = options.threads.max(1);
        let options = SearchOptions {
            max_hits: u32::MAX,
            book_deadline: Duration::MAX,
            ..options.clone()
        };
        let run_plan = RunPlan {
            chapters: plan.worklist.clone(),
            scan: Some(Arc::new(ReplaceScan {
                replacer: preview.request.replacer(),
                row_limit: 0,
            })),
        };
        let budget = self.budget();
        let labels = plan.labels()?;
        let mut changes = ApplyChanges {
            line: &line,
            labels,
            positions: plan.positions(),
            last: plan.worklist.len() - 1,
            total: plan.worklist.len() as u32,
            done: 0,
            written: 0,
            cancel,
            progress,
        };
        let outcome = std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| line.work(&plan, &budget));
            }
            scope.spawn(|| {
                line.scan(&plan, &preview.request.query, &options, run_plan, &budget);
            });
            let outcome = self.save_if_current(preview.snapshot, &mut changes);
            line.stop();
            outcome
        });
        if line.nothing.load(Ordering::Relaxed) {
            return Ok(AppliedReplace {
                report: CommitReport::nothing_to_do(),
                changes: 0,
                titles: 0,
                chapters: 0,
            });
        }
        let mut report = outcome?;
        let (changed, titles, chapters, warnings) = line.totals();
        report.warnings.extend(warnings);
        Ok(AppliedReplace {
            report,
            changes: changed,
            titles,
            chapters,
        })
    }
}

impl ReplacePreview {
    fn count(&mut self, chapter: ChapterIx, hits: &[Hit]) {
        let mut changes = 0u32;
        let mut seen: Vec<CleanRule> = Vec::new();
        for (ordinal, hit) in hits.iter().enumerate() {
            if hit.rule == Some(CleanRule::Titles)
                && let Some(title) = &hit.replaced
            {
                self.titles.insert(chapter, (ordinal as u32, title.clone()));
            }
            if let Some(rule) = hit.rule
                && let Some(tally) = self.rules.iter_mut().find(|tally| tally.rule == rule)
            {
                tally.matches += 1;
                if !seen.contains(&rule) {
                    seen.push(rule);
                    tally.chapters += 1;
                }
            }
            if hit.crosses_block {
                self.skipped += 1;
            } else if hit.same {
                self.same += 1;
            } else {
                changes += 1;
                if hit.spans_tags {
                    self.spanning += 1;
                }
            }
        }
        self.matches += hits.len() as u64;
        self.changes += u64::from(changes);
        self.chapters += u32::from(changes > 0);
        self.touched.push(TouchedChapter {
            chapter,
            matches: hits.len() as u32,
            changes,
        });
    }
}

struct ApplyPlan {
    reader: Arc<Reader<File>>,
    replacer: Replacer,
    worklist: Vec<ChapterIx>,
    excluded: HashMap<ChapterIx, HashSet<u32>>,
    rules_out: HashSet<CleanRule>,
    expected: HashMap<ChapterIx, u32>,
    titles: HashMap<ChapterIx, String>,
}

impl ApplyPlan {
    fn new(
        reader: Arc<Reader<File>>,
        preview: &ReplacePreview,
        exclude: &[ReplaceExclusion],
    ) -> ApplyPlan {
        let mut chapters_out = HashSet::new();
        let mut excluded: HashMap<ChapterIx, HashSet<u32>> = HashMap::new();
        let mut rules_out = HashSet::new();
        for exclusion in exclude {
            match *exclusion {
                ReplaceExclusion::Chapter { chapter } => {
                    chapters_out.insert(chapter);
                }
                ReplaceExclusion::Hit { chapter, ordinal } => {
                    excluded.entry(chapter).or_default().insert(ordinal);
                }
                ReplaceExclusion::Rule { rule } => {
                    rules_out.insert(rule);
                }
            }
        }
        let zip_order: HashMap<&EntryName, usize> = reader
            .zip()
            .entries()
            .iter()
            .enumerate()
            .map(|(ix, entry)| (&entry.name, ix))
            .collect();
        let place = |chapter: ChapterIx| {
            reader
                .entry_of(chapter)
                .ok()
                .and_then(|entry| zip_order.get(&entry.name).copied())
                .unwrap_or(usize::MAX)
        };
        let mut worklist: Vec<ChapterIx> = preview
            .touched
            .iter()
            .filter(|touched| touched.changes > 0 && !chapters_out.contains(&touched.chapter))
            .map(|touched| touched.chapter)
            .collect();
        worklist.sort_by_cached_key(|chapter| place(*chapter));
        let expected = preview
            .touched
            .iter()
            .map(|touched| (touched.chapter, touched.matches))
            .collect();
        let titles = preview
            .titles
            .iter()
            .filter(|(chapter, (ordinal, _))| {
                !chapters_out.contains(*chapter)
                    && !rules_out.contains(&CleanRule::Titles)
                    && !excluded
                        .get(*chapter)
                        .is_some_and(|out| out.contains(ordinal))
            })
            .map(|(chapter, (_, title))| (*chapter, title.clone()))
            .collect();
        ApplyPlan {
            replacer: preview.request.replacer(),
            reader,
            worklist,
            excluded,
            rules_out,
            expected,
            titles,
        }
    }

    fn positions(&self) -> HashMap<EntryName, usize> {
        self.worklist
            .iter()
            .enumerate()
            .filter_map(|(position, chapter)| {
                let entry = self.reader.entry_of(*chapter).ok()?;
                Some((entry.name.clone(), position))
            })
            .collect()
    }

    fn labels(&self) -> Result<HashMap<EntryName, NewEntry>, Error> {
        let mut writes = HashMap::new();
        if self.titles.is_empty() {
            return Ok(writes);
        }
        let limits = self.reader.limits();
        let names = self.reader.entry_names();
        let docs: Vec<_> = self.reader.toc_docs()?.into_iter().flatten().collect();
        let package = self.reader.package(&docs, &names, Vec::new());
        if package.ensure_editable().is_err() {
            return Ok(writes);
        }
        let plan = package.relabel(&self.titles)?;
        check_plan(&self.reader, &package, &plan, &limits)?;
        for (doc, bytes) in package.docs.iter().zip(&plan.docs) {
            if *bytes == encode_text(doc.toc.text_form, doc.toc.source.clone()) {
                continue;
            }
            let in_worklist = self.worklist.iter().any(|chapter| {
                self.reader
                    .entry_of(*chapter)
                    .is_ok_and(|entry| entry.name == *doc.name)
            });
            if in_worklist {
                return Err(Error::WouldBreak(
                    "The contents page is also a chapter being cleaned, so nothing was saved."
                        .to_owned(),
                ));
            }
            writes.insert(doc.name.clone(), NewEntry::deflate(doc.name.clone(), bytes));
        }
        Ok(writes)
    }

    fn rebuild(&self, job: &Job) -> Result<Built, Error> {
        if self.expected.get(&job.chapter) != Some(&(job.hits.len() as u32)) {
            return Err(changed_since_preview());
        }
        let limits = self.reader.limits();
        let Some(model) = self.reader.model(job.chapter)? else {
            return Err(changed_since_preview());
        };
        if matches!(self.replacer, Replacer::Remove) {
            return self.rebuild_removal(job, &model, &limits);
        }
        let excluded = self.excluded.get(&job.chapter);
        let visible = model.text.as_str();
        let mut edits = Vec::new();
        let mut expected = String::with_capacity(visible.len());
        let mut cursor = 0;
        let mut changes = 0u32;
        let mut grown = model.source.len() as u64;
        for (ordinal, hit) in job.hits.iter().enumerate() {
            let is_out = excluded.is_some_and(|out| out.contains(&(ordinal as u32)));
            if hit.crosses_block || hit.same || is_out {
                continue;
            }
            let (start, end) = (hit.range.start as usize, hit.range.end as usize);
            let text = match &self.replacer {
                Replacer::Literal { .. } => self.replacer.literal_for(&visible[start..end]),
                Replacer::Template(_) => hit.replaced.as_deref(),
                Replacer::Remove => None,
            }
            .ok_or_else(changed_since_preview)?;
            let new_edits = model.text.source_edits(hit.range, text);
            grown += new_edits
                .iter()
                .map(|edit| edit.text.len() as u64)
                .sum::<u64>();
            if grown > limits.max_entry_inflated {
                return Err(Error::Limit(
                    "This replacement would make a chapter too large, so nothing was saved."
                        .to_owned(),
                ));
            }
            edits.extend(new_edits);
            expected.push_str(&visible[cursor..start]);
            expected.push_str(text);
            cursor = end;
            changes += 1;
        }
        if changes == 0 {
            return Ok(Built::Unchanged);
        }
        expected.push_str(&visible[cursor..]);
        let (bytes, warnings) = finish_bytes(&model, edits);
        check_rebuilt(&model, &bytes, &expected, &limits)
            .map_err(|()| unsafe_change(job.chapter))?;
        self.built(job.chapter, &bytes, changes, warnings)
    }

    fn rebuild_removal(
        &self,
        job: &Job,
        model: &ChapterModel,
        limits: &Limits,
    ) -> Result<Built, Error> {
        let excluded = self.excluded.get(&job.chapter);
        let chapter = clean::Chapter::of(model, limits).map_err(|_| changed_since_preview())?;
        let all: Vec<CleanRule> = RULES.iter().map(|spec| spec.rule).collect();
        let mut found = clean::find(&chapter, &all);
        let mut chosen = Vec::new();
        for (ordinal, hit) in job.hits.iter().enumerate() {
            let hit_out = excluded.is_some_and(|out| out.contains(&(ordinal as u32)));
            let rule_out = hit.rule.is_some_and(|rule| self.rules_out.contains(&rule));
            if hit_out || rule_out {
                continue;
            }
            let change = if hit.rule == Some(CleanRule::Titles) {
                let wanted = self.titles.get(&job.chapter);
                let at = found
                    .iter()
                    .position(|change| match change {
                        clean::Change::Retitle(fix) => {
                            chapter.paras[fix.heading].vis == hit.range
                                && hit.replaced.as_ref() == Some(&fix.title)
                                && wanted == Some(&fix.title)
                        }
                        clean::Change::Remove { .. } => false,
                    })
                    .ok_or_else(changed_since_preview)?;
                found.swap_remove(at)
            } else {
                let para = chapter
                    .paras
                    .iter()
                    .position(|para| para.vis == hit.range && para.cut.is_some())
                    .ok_or_else(changed_since_preview)?;
                clean::Change::Remove {
                    rule: hit.rule.unwrap_or(CleanRule::Watermark),
                    para,
                }
            };
            chosen.push(change);
        }
        if chosen.is_empty() {
            return Ok(Built::Unchanged);
        }
        let titles = chosen
            .iter()
            .filter(|change| matches!(change, clean::Change::Retitle(_)))
            .count() as u32;
        let (bytes, warnings) = clean::rewrite(model, &chapter, &chosen, limits)
            .map_err(|()| unsafe_change(job.chapter))?;
        let name = self.reader.entry_of(job.chapter)?.name.clone();
        Ok(Built::Entry {
            size: bytes.len(),
            entry: NewEntry::deflate(name, &bytes),
            changes: chosen.len() as u32 - titles,
            titles,
            warnings,
        })
    }

    fn built(
        &self,
        chapter: ChapterIx,
        bytes: &[u8],
        changes: u32,
        warnings: Vec<String>,
    ) -> Result<Built, Error> {
        let name = self.reader.entry_of(chapter)?.name.clone();
        Ok(Built::Entry {
            size: bytes.len(),
            entry: NewEntry::deflate(name, bytes),
            changes,
            titles: 0,
            warnings,
        })
    }
}

fn without_space(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter(|ch| !ch.is_whitespace())
}

fn tags<'a>(source: &'a str, events: &'a [Event<'a>]) -> impl Iterator<Item = &'a str> + 'a {
    events.iter().filter_map(move |event| match event {
        Event::Open { span, .. } | Event::Close { span, .. } => Some(&source[span.range()]),
        Event::Text { .. } | Event::CData { .. } | Event::Other { .. } => None,
    })
}

fn check_rebuilt(
    old: &ChapterModel,
    bytes: &[u8],
    expected: &str,
    limits: &Limits,
) -> Result<(), ()> {
    let new = decode_and_model(bytes.to_vec(), limits).map_err(|_| ())?;
    if !without_space(new.text.as_str()).eq(without_space(expected)) {
        return Err(());
    }
    let old_scan = scan(&old.source, limits).map_err(|_| ())?;
    let new_scan = scan(&new.source, limits).map_err(|_| ())?;
    if tags(&old.source, &old_scan.events).eq(tags(&new.source, &new_scan.events)) {
        Ok(())
    } else {
        Err(())
    }
}

struct Job {
    position: usize,
    chapter: ChapterIx,
    hits: Vec<Hit>,
}

enum Built {
    Unchanged,
    Entry {
        entry: NewEntry,
        size: usize,
        changes: u32,
        titles: u32,
        warnings: Vec<String>,
    },
}

#[derive(Default)]
struct Line {
    jobs: VecDeque<Job>,
    ready: HashMap<usize, Built>,
    head: usize,
    ready_bytes: usize,
    pushed: usize,
    scan_done: bool,
    failed: Option<Error>,
    changes: u64,
    titles: u64,
    chapters: u32,
    warnings: Vec<String>,
}

#[derive(Default)]
struct Pipeline {
    line: Mutex<Line>,
    moved: Condvar,
    stopped: AtomicBool,
    nothing: AtomicBool,
    scan_cancel: CancelToken,
}

impl Pipeline {
    fn lock(&self) -> MutexGuard<'_, Line> {
        self.line
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'a>(&self, line: MutexGuard<'a, Line>) -> MutexGuard<'a, Line> {
        self.moved
            .wait_timeout(line, POLL)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.scan_cancel.cancel();
        self.moved.notify_all();
    }

    fn fail(&self, error: Error) {
        self.lock().failed.get_or_insert(error);
        self.stop();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn push(&self, job: Job, room_for: usize) {
        let mut line = self.lock();
        loop {
            if self.is_stopped() {
                return;
            }
            let at_head = job.position == line.head;
            let has_room = job.position < line.head + WINDOW_CHAPTERS
                && line.ready_bytes < WINDOW_BYTES
                && line.jobs.len() < room_for;
            if at_head || has_room {
                break;
            }
            line = self.wait(line);
        }
        line.pushed += 1;
        line.jobs.push_back(job);
        self.moved.notify_all();
    }

    fn scan(
        &self,
        plan: &ApplyPlan,
        query: &Query,
        options: &SearchOptions,
        run_plan: RunPlan,
        budget: &Arc<ByteBudget>,
    ) {
        let positions: HashMap<ChapterIx, usize> = plan
            .worklist
            .iter()
            .enumerate()
            .map(|(position, chapter)| (*chapter, position))
            .collect();
        let room_for = options.threads.max(1) * 2;
        let end = run_search(
            plan.reader.clone(),
            budget.clone(),
            query,
            options,
            run_plan,
            &self.scan_cancel,
            &mut |event| match event {
                SearchEvent::Hits { chapter, hits, .. } => match positions.get(&chapter) {
                    Some(&position) => self.push(
                        Job {
                            position,
                            chapter,
                            hits,
                        },
                        room_for,
                    ),
                    None => self.fail(changed_since_preview()),
                },
                SearchEvent::ChapterLimited { .. } | SearchEvent::CapReached => {
                    self.fail(too_slow());
                }
                SearchEvent::ChapterUnreadable { message, .. } => self.fail(Error::Book(message)),
                SearchEvent::Progress { .. } => {}
            },
        );
        match end {
            SearchEnd::Done => {
                let mut line = self.lock();
                let complete = line.pushed == plan.worklist.len();
                line.scan_done = true;
                drop(line);
                if complete {
                    self.moved.notify_all();
                } else {
                    self.fail(changed_since_preview());
                }
            }
            SearchEnd::Cancelled => self.moved.notify_all(),
            SearchEnd::TooSlow { .. } => self.fail(too_slow()),
            SearchEnd::BadPattern(message) | SearchEnd::Failed(message) => {
                self.fail(Error::Book(message));
            }
        }
    }

    fn next_job(&self) -> Option<Job> {
        let mut line = self.lock();
        loop {
            if self.is_stopped() {
                return None;
            }
            if let Some(job) = line.jobs.pop_front() {
                self.moved.notify_all();
                return Some(job);
            }
            if line.scan_done {
                return None;
            }
            line = self.wait(line);
        }
    }

    fn work(&self, plan: &ApplyPlan, budget: &Arc<ByteBudget>) {
        while let Some(job) = self.next_job() {
            let inflated = plan
                .reader
                .rev_of(job.chapter)
                .map_or(0, |rev| u64::from(rev.inflated_len));
            let cost = inflated * MODEL_BYTES_PER_SOURCE_BYTE * REBUILD_COST_FACTOR;
            let Some(permit) = budget.acquire(cost, &self.stopped) else {
                return;
            };
            let attempt = catch_unwind(AssertUnwindSafe(|| plan.rebuild(&job)));
            drop(permit);
            match attempt {
                Ok(Ok(built)) => {
                    let mut line = self.lock();
                    if let Built::Entry { size, .. } = &built {
                        line.ready_bytes += size;
                    }
                    line.ready.insert(job.position, built);
                    self.moved.notify_all();
                }
                Ok(Err(error)) => self.fail(error),
                Err(_) => self.fail(Error::WouldBreak(
                    "Something went wrong while changing a chapter, so nothing was saved."
                        .to_owned(),
                )),
            }
        }
    }

    fn totals(&self) -> (u64, u64, u32, Vec<String>) {
        let mut line = self.lock();
        let warnings = std::mem::take(&mut line.warnings);
        (line.changes, line.titles, line.chapters, warnings)
    }
}

struct ApplyChanges<'a> {
    line: &'a Pipeline,
    labels: HashMap<EntryName, NewEntry>,
    positions: HashMap<EntryName, usize>,
    last: usize,
    total: u32,
    done: u32,
    written: u32,
    cancel: &'a CancelToken,
    progress: &'a mut dyn FnMut(u32, u32),
}

impl ApplyChanges<'_> {
    fn check_stop<'g>(
        &self,
        mut line: MutexGuard<'g, Line>,
    ) -> Result<MutexGuard<'g, Line>, Error> {
        if let Some(error) = line.failed.take() {
            return Err(error);
        }
        if self.cancel.is_cancelled() {
            drop(line);
            self.line.stop();
            return Err(Error::Cancelled);
        }
        Ok(self.line.wait(line))
    }

    fn built_at(&self, position: usize) -> Result<Built, Error> {
        let mut line = self.line.lock();
        loop {
            if let Some(error) = line.failed.take() {
                return Err(error);
            }
            if let Some(built) = line.ready.remove(&position) {
                line.head = position + 1;
                if let Built::Entry { size, .. } = &built {
                    line.ready_bytes -= size;
                }
                self.line.moved.notify_all();
                return Ok(built);
            }
            line = self.check_stop(line)?;
        }
    }

    fn wait_for_scan_end(&self) -> Result<(), Error> {
        let mut line = self.line.lock();
        while !line.scan_done {
            line = self.check_stop(line)?;
        }
        match line.failed.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Changes for ApplyChanges<'_> {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error> {
        if self.cancel.is_cancelled() {
            self.line.stop();
            return Err(Error::Cancelled);
        }
        if let Some(new) = self.labels.remove(&entry.name) {
            return Ok(EntryFate::Write(new));
        }
        let Some(&position) = self.positions.get(&entry.name) else {
            return Ok(EntryFate::RawCopy);
        };
        if self.done == 0 {
            (self.progress)(0, self.total);
        }
        let built = self.built_at(position)?;
        self.done += 1;
        (self.progress)(self.done, self.total);
        let fate = match built {
            Built::Unchanged => EntryFate::RawCopy,
            Built::Entry {
                entry,
                changes,
                titles,
                warnings,
                ..
            } => {
                let mut line = self.line.lock();
                line.changes += u64::from(changes);
                line.titles += u64::from(titles);
                line.chapters += 1;
                for warning in warnings {
                    if !line.warnings.contains(&warning) {
                        line.warnings.push(warning);
                    }
                }
                self.written += 1;
                EntryFate::Write(entry)
            }
        };
        if position == self.last {
            self.wait_for_scan_end()?;
            if self.written == 0 {
                self.line.nothing.store(true, Ordering::Relaxed);
                return Err(Error::Cancelled);
            }
        }
        Ok(fate)
    }

    fn added(&mut self) -> Vec<NewEntry> {
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixOffer {
    pub from: String,
    pub to: String,
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || matches!(ch, '\u{300}'..='\u{36F}')
}

fn is_connector(ch: char) -> bool {
    matches!(ch, '\'' | '\u{2019}' | '-')
}

fn is_in_word(text: &str, at: usize) -> bool {
    let Some(ch) = text[at..].chars().next() else {
        return false;
    };
    if is_word_char(ch) {
        return true;
    }
    let before = text[..at].chars().next_back().is_some_and(is_word_char);
    let after = text[at + ch.len_utf8()..]
        .chars()
        .next()
        .is_some_and(is_word_char);
    is_connector(ch) && before && after
}

fn word_start(text: &str, mut at: usize) -> usize {
    while let Some(ch) = text[..at].chars().next_back() {
        let previous = at - ch.len_utf8();
        if !is_in_word(text, previous) {
            break;
        }
        at = previous;
    }
    at
}

fn word_end(text: &str, mut at: usize) -> usize {
    while let Some(ch) = text[at..].chars().next() {
        if !is_in_word(text, at) {
            break;
        }
        at += ch.len_utf8();
    }
    at
}

fn is_one_word(text: &str) -> bool {
    word_start(text, text.len()) == 0
        && text.chars().any(char::is_alphabetic)
        && (MIN_OFFER_WORD_CHARS..=MAX_OFFER_WORD_CHARS).contains(&text.chars().count())
}

pub(crate) fn fix_offer(old: &str, new: &str) -> Option<FixOffer> {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let room = old.len().min(new.len()) - prefix;
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .scan(0, |total, width| {
            *total += width;
            (*total <= room).then_some(width)
        })
        .sum();
    if prefix == old.len() && prefix == new.len() {
        return None;
    }
    let from = &old[word_start(old, prefix)..word_end(old, old.len() - suffix)];
    let to = &new[word_start(new, prefix)..word_end(new, new.len() - suffix)];
    let clear = is_one_word(from) && is_one_word(to);
    (clear && from.to_lowercase() != to.to_lowercase()).then(|| FixOffer {
        from: from.to_owned(),
        to: to.to_owned(),
    })
}

pub(crate) fn offers_between(
    old: &Reader<File>,
    new: &Reader<File>,
    ix: ChapterIx,
) -> Vec<FixOffer> {
    let small = |reader: &Reader<File>| {
        reader
            .rev_of(ix)
            .is_ok_and(|rev| u64::from(rev.inflated_len) <= MAX_OFFER_CHAPTER_BYTES)
    };
    if !small(old) || !small(new) {
        return Vec::new();
    }
    let (Ok(before), Ok(after)) = (old.visible_text(ix), new.visible_text(ix)) else {
        return Vec::new();
    };
    fix_offer(before.as_str(), after.as_str())
        .into_iter()
        .collect()
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl FixOffer {
    fn variants(&self) -> Vec<CaseVariant> {
        let mut pairs = vec![(self.from.clone(), self.to.clone())];
        let mut letters = self.from.chars();
        let first_upper = letters.next().is_some_and(char::is_uppercase);
        let rest_lower = !letters.any(char::is_uppercase);
        if !first_upper && rest_lower {
            pairs.push((capitalised(&self.from), capitalised(&self.to)));
        }
        if rest_lower {
            pairs.push((self.from.to_uppercase(), self.to.to_uppercase()));
        }
        let mut seen = HashSet::new();
        pairs
            .into_iter()
            .filter(|(matched, _)| seen.insert(matched.clone()))
            .map(|(matched, replacement)| CaseVariant {
                matched,
                replacement,
            })
            .collect()
    }

    pub fn request(&self) -> ReplaceRequest {
        let variants = self.variants();
        ReplaceRequest {
            query: Query::Literal {
                text: self.from.clone(),
                case: if variants.len() > 1 {
                    Case::Insensitive
                } else {
                    Case::Sensitive
                },
                whole_word: true,
            },
            replacement: self.to.clone(),
            variants,
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
