use super::links::LinkWindow;
use super::toc_edit::{DocEdits, Removal, entry_markup, subtree_end};
use super::{
    ChapterOp, DeletedChapter, InverseEdit, NewBookMeta, RowRef, Spot, UNTITLED, apply_inverse,
    clean_title, encode_text, invert, with_ws, ws_start, xml_escape,
};
use crate::book::ChapterRow;
use crate::chapter::ChapterModel;
use crate::clean::chapter_title;
use crate::edit::finish_bytes;
use crate::markup::{Event, SourceEdit, Span, attr_value, decode_text, local_name, scan, splice};
use crate::package::{
    OpfDoc, TextForm, TocDoc, TocSource, has_url_scheme, percent_encode_path, resolve_href,
};
use crate::zip::{EntryName, NewEntry};
use crate::{ChapterIx, Error, Limits};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

const MAX_STYLESHEETS: usize = 16;
const MIME_XHTML: &str = "application/xhtml+xml";
const NEW_BOOK_DIR: &str = "OEBPS";

pub(crate) struct Doc<'a> {
    pub(crate) name: &'a EntryName,
    pub(crate) toc: &'a TocDoc,
    files: Vec<Option<EntryName>>,
    rows: Vec<Option<u32>>,
    fragments: Vec<Option<String>>,
    of_row: HashMap<u32, Vec<u32>>,
}

pub(crate) struct Package<'a> {
    pub(crate) opf_name: &'a EntryName,
    pub(crate) opf: &'a OpfDoc,
    pub(crate) rows: &'a [ChapterRow],
    pub(crate) spine_items: &'a [Option<u32>],
    pub(crate) item_files: Vec<Option<EntryName>>,
    pub(crate) docs: Vec<Doc<'a>>,
    pub(crate) has_primary: bool,
    pub(crate) names: &'a BTreeSet<EntryName>,
    pub(crate) reserved_names: Vec<EntryName>,
    pub(crate) limits: Limits,
}

#[derive(Default)]
pub(crate) struct Gathered {
    pub(crate) chapter: Option<ChapterModel>,
    pub(crate) stylesheets: Vec<String>,
    pub(crate) record: Option<DeletedRecord>,
}

pub(crate) struct PackagePlan {
    pub(crate) opf: Vec<u8>,
    pub(crate) docs: Vec<Vec<u8>>,
    pub(crate) chapter: Option<(EntryName, Vec<u8>)>,
    pub(crate) added: Option<(EntryName, Vec<u8>)>,
    pub(crate) removed: Option<EntryName>,
    pub(crate) delta: i64,
    pub(crate) landing: Option<EntryName>,
    pub(crate) record: Option<DeletedRecord>,
    pub(crate) restores_prior: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct SavedDocEntry {
    pub(crate) fragment: Option<String>,
    pub(crate) label: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct RestoreDoc {
    pub(crate) name: String,
    pub(crate) after_crc: u32,
    pub(crate) edits: Vec<InverseEdit>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct DeletedRecord {
    pub(crate) version: u32,
    pub(crate) chapter: DeletedChapter,
    pub(crate) entry: String,
    pub(crate) bytes: String,
    pub(crate) crc32: u32,
    pub(crate) item_id: String,
    pub(crate) item_text: String,
    pub(crate) row_text: String,
    pub(crate) spine_at: u32,
    pub(crate) prev_idref: Option<String>,
    pub(crate) restore: Vec<RestoreDoc>,
    pub(crate) toc: Vec<(String, Vec<SavedDocEntry>)>,
    #[serde(default)]
    pub(crate) linking: Vec<LinkWindow>,
}

pub(crate) struct HeadingSpans {
    pub(crate) title: Option<(Span, String)>,
    pub(crate) heading: Option<(Span, String)>,
    pub(crate) body_start: Option<u32>,
}

fn is_heading(local: &str) -> bool {
    matches!(local, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

pub(crate) fn heading_of(source: &str, limits: &Limits) -> Result<HeadingSpans, Error> {
    let scanned = scan(source, limits)?;
    let mut spans = HeadingSpans {
        title: None,
        heading: None,
        body_start: None,
    };
    let mut open: Option<(bool, String, u32, u32, String)> = None;
    for event in &scanned.events {
        match event {
            Event::Open {
                name,
                span,
                self_closed,
                ..
            } => {
                let local = local_name(name).to_ascii_lowercase();
                if let Some((_, wanted, _, depth, _)) = &mut open {
                    if !self_closed && local == *wanted {
                        *depth += 1;
                    }
                    continue;
                }
                if local == "body" && !self_closed && spans.body_start.is_none() {
                    spans.body_start = Some(span.end);
                } else if local == "title"
                    && !self_closed
                    && spans.title.is_none()
                    && spans.body_start.is_none()
                {
                    open = Some((true, local, span.end, 0, String::new()));
                } else if is_heading(&local)
                    && !self_closed
                    && spans.heading.is_none()
                    && spans.body_start.is_some()
                {
                    open = Some((false, local, span.end, 0, String::new()));
                }
            }
            Event::Close { name, span } => {
                let local = local_name(name).to_ascii_lowercase();
                if let Some((is_title, wanted, inner, depth, text)) = &mut open
                    && local == *wanted
                {
                    if *depth > 0 {
                        *depth -= 1;
                        continue;
                    }
                    let found = (Span::new(*inner as usize, span.start as usize), tidy(text));
                    if *is_title {
                        spans.title = Some(found);
                    } else {
                        spans.heading = Some(found);
                    }
                    open = None;
                }
            }
            Event::Text { raw, .. } => {
                if let Some((.., text)) = &mut open {
                    text.push_str(&decode_text(raw));
                }
            }
            Event::CData { raw, .. } => {
                if let Some((.., text)) = &mut open {
                    text.push_str(raw);
                }
            }
            Event::Other { .. } => {}
        }
    }
    Ok(spans)
}

fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn stylesheets_of(source: &str, limits: &Limits) -> Vec<String> {
    let Ok(scanned) = scan(source, limits) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for event in &scanned.events {
        if let Event::Open { name, attrs, .. } = event {
            let local = local_name(name).to_ascii_lowercase();
            if local == "body" {
                break;
            }
            if local == "link"
                && attr_value(attrs, "rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|kind| kind.eq_ignore_ascii_case("stylesheet"))
                })
                && let Some(href) = attr_value(attrs, "href")
                && !has_url_scheme(&href)
                && found.len() < MAX_STYLESHEETS
            {
                found.push(href);
            }
        }
    }
    found
}

pub(crate) fn relative_href(from: &EntryName, to: &EntryName) -> String {
    let base: Vec<&str> = from
        .as_str()
        .rsplit_once('/')
        .map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
    let target: Vec<&str> = to.as_str().split('/').collect();
    let shared = base
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count()
        .min(target.len().saturating_sub(1));
    let mut parts: Vec<String> = vec!["..".to_owned(); base.len() - shared];
    parts.extend(
        target[shared..]
            .iter()
            .map(|part| percent_encode_path(part)),
    );
    parts.join("/")
}

impl<'a> Doc<'a> {
    pub(crate) fn new(
        name: &'a EntryName,
        toc: &'a TocDoc,
        row_of_file: &HashMap<EntryName, u32>,
    ) -> Doc<'a> {
        let count = toc.entries.len();
        let mut doc = Doc {
            name,
            toc,
            files: Vec::with_capacity(count),
            rows: Vec::with_capacity(count),
            fragments: Vec::with_capacity(count),
            of_row: HashMap::new(),
        };
        for (at, entry) in toc.entries.iter().enumerate() {
            let (path, fragment) = match entry.href.as_deref().map(|href| href.split_once('#')) {
                Some(Some((path, fragment))) => (Some(path), Some(fragment.to_owned())),
                Some(None) => (entry.href.as_deref(), None),
                None => (None, None),
            };
            let file = path
                .filter(|path| !path.is_empty() && !has_url_scheme(path))
                .and_then(|path| resolve_href(name, path).ok().flatten());
            let row = file
                .as_ref()
                .and_then(|file| row_of_file.get(file).copied());
            if let Some(row) = row {
                doc.of_row.entry(row).or_default().push(at as u32);
            }
            doc.files.push(file);
            doc.rows.push(row);
            doc.fragments.push(fragment.filter(|text| !text.is_empty()));
        }
        doc
    }

    pub(crate) fn has_rows(&self) -> bool {
        !self.of_row.is_empty()
    }

    fn subtree_rows(&self, at: u32) -> Vec<u32> {
        (at..subtree_end(self.toc, at))
            .filter_map(|ix| self.rows[ix as usize])
            .collect()
    }

    fn first_entry_of(&self, row: u32) -> Option<u32> {
        self.of_row
            .get(&row)
            .and_then(|entries| entries.first().copied())
    }

    fn top_entries_of(&self, row: u32) -> Vec<u32> {
        let Some(entries) = self.of_row.get(&row) else {
            return Vec::new();
        };
        entries
            .iter()
            .copied()
            .filter(|&at| {
                let mut parent = self.toc.entries[at as usize].parent;
                while let Some(above) = parent {
                    if self.rows[above as usize] == Some(row) {
                        return false;
                    }
                    parent = self.toc.entries[above as usize].parent;
                }
                true
            })
            .collect()
    }

    fn entry_for_rename(&self, row: u32) -> Option<u32> {
        let entries = self.of_row.get(&row)?;
        let plain = |at: &&u32| self.fragments[**at as usize].is_none();
        let leaf = |at: &&u32| self.toc.entries[**at as usize].kids.is_empty();
        entries
            .iter()
            .filter(plain)
            .find(leaf)
            .or_else(|| entries.iter().find(plain))
            .or_else(|| entries.first())
            .copied()
    }

    fn fresh_nav_point_id(&self, used: &mut HashSet<String>) -> String {
        let mut number = self.toc.entries.len() + 1;
        loop {
            let id = format!("ss-nav-{number}");
            if !self.toc.ids.contains(&id) && used.insert(id.clone()) {
                return id;
            }
            number += 1;
        }
    }

    fn new_entry(&self, used: &mut HashSet<String>, href: &str, label: &str) -> String {
        match self.toc.kind {
            TocSource::Ncx => {
                let has_order = self
                    .toc
                    .entries
                    .iter()
                    .any(|entry| entry.play_order.is_some());
                let id = self.fresh_nav_point_id(used);
                let order = has_order.then(|| next_play_order(self.toc));
                entry_markup(self.toc, href, label, Some((&id, order)))
            }
            _ => entry_markup(self.toc, href, label, None),
        }
    }

    fn at_before(&self, at: u32) -> (u32, String) {
        let source = &self.toc.source;
        let start = self.toc.entries[at as usize].whole.start as usize;
        let from = ws_start(source, start);
        (from as u32, source[from..start].to_owned())
    }

    fn at_after(&self, at: u32) -> (u32, String) {
        let source = &self.toc.source;
        let whole = self.toc.entries[at as usize].whole;
        let from = ws_start(source, whole.start as usize);
        (whole.end, source[from..whole.start as usize].to_owned())
    }

    fn at_end(&self) -> Option<(u32, String)> {
        let last_top = self
            .toc
            .entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, entry)| entry.parent.is_none())
            .map(|(at, _)| at as u32);
        match last_top {
            Some(at) => Some(self.at_after(at)),
            None => self.toc.list.map(|list| (list.start, String::new())),
        }
    }

    fn insertion_after_row(&self, row: Option<u32>, rows: u32) -> Option<(u32, String)> {
        if let Some(row) = row {
            let next = (row + 1..rows).find_map(|later| self.first_entry_of(later));
            let tops = self.top_entries_of(row);
            if let Some(&anchor) = tops.last() {
                let after_anchor = self.at_after(anchor);
                return Some(match next {
                    Some(next) if self.toc.entries[next as usize].whole.start < after_anchor.0 => {
                        self.at_before(next)
                    }
                    _ => after_anchor,
                });
            }
            if let Some(next) = next {
                return Some(self.at_before(next));
            }
        }
        self.at_end()
    }
}

fn next_play_order(toc: &TocDoc) -> u64 {
    if play_orders_ok(toc) {
        return 0;
    }
    toc.entries
        .iter()
        .filter_map(|entry| entry.play_order)
        .filter_map(|span| toc.source[span.range()].trim().parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn play_orders_ok(toc: &TocDoc) -> bool {
    let mut last = 0u64;
    for entry in &toc.entries {
        let Some(span) = entry.play_order else {
            continue;
        };
        let Ok(value) = toc.source[span.range()].trim().parse::<u64>() else {
            return false;
        };
        if value <= last {
            return false;
        }
        last = value;
    }
    true
}

fn nav_depth(toc: &TocDoc) -> u16 {
    toc.entries
        .iter()
        .map(|entry| entry.depth.saturating_add(1))
        .max()
        .unwrap_or(0)
}

fn depth_meta_matches(toc: &TocDoc) -> bool {
    toc.depth_meta.is_some_and(|span| {
        toc.source[span.range()].trim().parse::<u16>().ok() == Some(nav_depth(toc))
    })
}

fn depth_fix(
    toc: &TocDoc,
    edits: &[SourceEdit],
    limits: &Limits,
) -> Result<Option<SourceEdit>, Error> {
    let Some(span) = toc.depth_meta.filter(|_| depth_meta_matches(toc)) else {
        return Ok(None);
    };
    let after = TocDoc::parse_ncx(splice(&toc.source, edits.to_vec()).into_bytes(), limits)?;
    let depth = nav_depth(&after);
    Ok((depth != nav_depth(toc)).then(|| SourceEdit {
        span,
        text: depth.to_string(),
    }))
}

fn finish_toc(toc: &TocDoc, edits: DocEdits, limits: &Limits) -> Result<Vec<u8>, Error> {
    let text = splice(&toc.source, edits.finish());
    let orders_tracked = play_orders_ok(toc);
    let depth_tracked = depth_meta_matches(toc);
    if toc.kind != TocSource::Ncx || !(orders_tracked || depth_tracked) {
        return Ok(encode_text(toc.text_form, text));
    }
    let after = TocDoc::parse_ncx(text.into_bytes(), limits)?;
    let mut fixes = Vec::new();
    if orders_tracked && !play_orders_ok(&after) {
        let mut number = 0u64;
        fixes.extend(
            after
                .entries
                .iter()
                .filter_map(|entry| entry.play_order)
                .map(|span| {
                    number += 1;
                    SourceEdit {
                        span,
                        text: number.to_string(),
                    }
                }),
        );
    }
    if depth_tracked
        && !depth_meta_matches(&after)
        && let Some(span) = after.depth_meta
    {
        fixes.push(SourceEdit {
            span,
            text: nav_depth(&after).to_string(),
        });
    }
    Ok(encode_text(toc.text_form, splice(&after.source, fixes)))
}

fn new_chapter_text(
    title: &str,
    language: Option<&str>,
    epub2: bool,
    stylesheets: &[String],
) -> String {
    let title = xml_escape(title);
    let doctype = if epub2 {
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">"
    } else {
        "<!DOCTYPE html>"
    };
    let lang = language.map_or(String::new(), |language| {
        let language = xml_escape(language);
        if epub2 {
            format!(" xml:lang=\"{language}\"")
        } else {
            format!(" lang=\"{language}\" xml:lang=\"{language}\"")
        }
    });
    let links: String = stylesheets
        .iter()
        .map(|href| {
            format!(
                "\n<link rel=\"stylesheet\" type=\"text/css\" href=\"{}\"/>",
                xml_escape(href)
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{doctype}\n<html xmlns=\"http://www.w3.org/1999/xhtml\"{lang}>\n<head>\n<title>{title}</title>{links}\n</head>\n<body>\n<h1>{title}</h1>\n<p><br/></p>\n</body>\n</html>\n"
    )
}

fn refuse(why: &str) -> Error {
    Error::NotEditable(why.to_owned())
}

struct MovePlan {
    moved: BTreeSet<u32>,
    order: Vec<u32>,
    before_block: Option<u32>,
}

impl<'a> Package<'a> {
    pub(crate) fn ensure_editable(&self) -> Result<(), Error> {
        let other = |form: TextForm| form == TextForm::Other;
        if other(self.opf.text_form) || self.docs.iter().any(|doc| other(doc.toc.text_form)) {
            return Err(refuse(
                "This book's package files are not stored as UTF-8, so its chapters cannot be managed yet. The book was not changed.",
            ));
        }
        if self.opf.repaired || self.docs.iter().any(|doc| doc.toc.repaired) {
            return Err(refuse(
                "This book's package files are damaged, so its chapters cannot be managed safely. The book was not changed.",
            ));
        }
        Ok(())
    }

    fn row(&self, ix: ChapterIx) -> Result<u32, Error> {
        if (ix.0 as usize) < self.rows.len() {
            Ok(ix.0)
        } else {
            Err(refuse("That chapter is not in this book."))
        }
    }

    fn primary(&self) -> Option<&Doc<'a>> {
        self.has_primary.then(|| &self.docs[0])
    }

    fn entry(&self, entry: u32) -> Result<(&Doc<'a>, u32), Error> {
        let doc = self.primary().ok_or_else(|| {
            refuse("This book has no table of contents, so that row cannot be used.")
        })?;
        if (entry as usize) < doc.toc.entries.len() {
            Ok((doc, entry))
        } else {
            Err(refuse("That row is no longer in the table of contents."))
        }
    }

    fn is_listed(&self, row: u32) -> bool {
        self.primary()
            .is_some_and(|doc| doc.of_row.contains_key(&row))
    }

    fn carried_after(&self, row: u32) -> u32 {
        let mut next = row + 1;
        if !self.is_listed(row) {
            return next;
        }
        while (next as usize) < self.rows.len()
            && !self.is_listed(next)
            && !self.rows[next as usize].is_nav
        {
            next += 1;
        }
        next
    }

    fn name_of(&self, row: u32) -> Option<EntryName> {
        self.rows[row as usize].href.clone()
    }

    pub(crate) fn rename_target(&self, row: RowRef) -> Result<(Option<u32>, Option<u32>), Error> {
        match row {
            RowRef::Entry(entry) => {
                let (doc, at) = self.entry(entry)?;
                Ok((Some(at), doc.rows[at as usize]))
            }
            RowRef::Chapter(ix) => {
                let row = self.row(ix)?;
                let entry = self.primary().and_then(|doc| doc.entry_for_rename(row));
                Ok((entry, Some(row)))
            }
        }
    }

    pub(crate) fn add_neighbour(&self, after: Option<ChapterIx>) -> Option<u32> {
        let wanted = after.map_or(self.rows.len().saturating_sub(1), |ix| ix.0 as usize);
        (0..=wanted.min(self.rows.len().saturating_sub(1)))
            .rev()
            .find(|&at| !self.rows[at].is_nav && self.rows[at].href.is_some())
            .map(|at| at as u32)
    }

    pub(crate) fn plan(&self, op: &ChapterOp, gathered: Gathered) -> Result<PackagePlan, Error> {
        self.ensure_editable()?;
        match op {
            ChapterOp::Add { after, title } => {
                let title = clean_title(title);
                let title = if title.is_empty() {
                    UNTITLED.to_owned()
                } else {
                    title
                };
                self.plan_add(*after, &title, &gathered.stylesheets)
            }
            ChapterOp::Rename { row, title } => {
                let title = clean_title(title);
                if title.is_empty() {
                    return Err(refuse("A chapter needs a title."));
                }
                self.plan_rename(*row, &title, gathered.chapter)
            }
            ChapterOp::Move { row, to } => self.plan_move(*row, *to),
            ChapterOp::Delete { chapter } => self.plan_delete(*chapter),
            ChapterOp::UndoDelete { .. } => {
                let record = gathered
                    .record
                    .ok_or_else(|| refuse("That deleted chapter can no longer be restored."))?;
                self.plan_undo(record)
            }
        }
    }

    fn unchanged_docs(&self) -> Vec<Vec<u8>> {
        self.docs
            .iter()
            .map(|doc| encode_text(doc.toc.text_form, doc.toc.source.clone()))
            .collect()
    }

    fn plan_of_opf(&self, edits: Vec<SourceEdit>) -> Vec<u8> {
        encode_text(self.opf.text_form, splice(&self.opf.source, edits))
    }

    fn fresh_file(&self, near: Option<u32>) -> (EntryName, String) {
        let dir = near
            .and_then(|row| self.name_of(row))
            .unwrap_or_else(|| self.opf_name.clone());
        let dir = dir
            .as_str()
            .rsplit_once('/')
            .map_or(String::new(), |(dir, _)| format!("{dir}/"));
        let taken: HashSet<String> = self
            .names
            .iter()
            .chain(&self.reserved_names)
            .map(|name| name.as_str().to_lowercase())
            .collect();
        let ids: HashSet<&str> = self.opf.ids.iter().map(String::as_str).collect();
        let mut number = self.rows.len() + 1;
        loop {
            let stem = format!("chapter-{number:03}");
            let path = format!("{dir}{stem}.xhtml");
            if !taken.contains(&path.to_lowercase())
                && !ids.contains(stem.as_str())
                && let Ok(name) = EntryName::parse(&path)
            {
                return (name, stem);
            }
            number += 1;
        }
    }

    fn manifest_insert(&self, text: &str) -> SourceEdit {
        let source = &self.opf.source;
        let last = self.opf.manifest.last().map(|item| item.span);
        let (at, ws) = match last {
            Some(span) => (
                span.end as usize,
                &source[ws_start(source, span.start as usize)..span.start as usize],
            ),
            None => (self.opf.manifest_end as usize, ""),
        };
        SourceEdit {
            span: Span::new(at, at),
            text: format!("{ws}{text}"),
        }
    }

    fn spine_insert_after(&self, row: Option<u32>, text: &str) -> SourceEdit {
        let source = &self.opf.source;
        let anchor = row.or_else(|| (self.rows.len() as u32).checked_sub(1));
        let span = anchor.map_or(
            Span::new(self.opf.spine_end as usize, self.opf.spine_end as usize),
            |at| self.opf.spine[at as usize].span,
        );
        let ws = &source[ws_start(source, span.start as usize)..span.start as usize];
        SourceEdit {
            span: Span::new(span.end as usize, span.end as usize),
            text: format!("{ws}{text}"),
        }
    }

    fn insert_new_rows(
        &self,
        file: &EntryName,
        after: Option<u32>,
        rows: impl Fn(&Doc<'a>) -> Vec<SavedDocEntry>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let mut out = Vec::with_capacity(self.docs.len());
        for doc in &self.docs {
            let mut edits = DocEdits::default();
            let mut used = HashSet::new();
            if let Some((at, ws)) = doc.insertion_after_row(after, self.rows.len() as u32) {
                let base = relative_href(doc.name, file);
                let mut text = String::new();
                for row in rows(doc) {
                    let href = match &row.fragment {
                        Some(fragment) => format!("{base}#{fragment}"),
                        None => base.clone(),
                    };
                    let markup =
                        doc.new_entry(&mut used, &xml_escape(&href), &xml_escape(&row.label));
                    text.push_str(&ws);
                    text.push_str(&markup);
                }
                edits.insert(at, text);
            }
            out.push(finish_toc(doc.toc, edits, &self.limits)?);
        }
        Ok(out)
    }

    fn plan_add(
        &self,
        after: Option<ChapterIx>,
        title: &str,
        stylesheets: &[String],
    ) -> Result<PackagePlan, Error> {
        let after = after.map(|ix| self.row(ix)).transpose()?;
        let anchor = after.map(|row| self.carried_after(row) - 1);
        let (file, id) = self.fresh_file(self.add_neighbour(after.map(ChapterIx)));
        let href = relative_href(self.opf_name, &file);
        let item = format!(
            "<{p}item id=\"{id}\" href=\"{href}\" media-type=\"{MIME_XHTML}\"/>",
            p = self.opf.prefix,
            href = xml_escape(&href)
        );
        let row = format!("<{p}itemref idref=\"{id}\"/>", p = self.opf.prefix);
        let opf = self.plan_of_opf(vec![
            self.manifest_insert(&item),
            self.spine_insert_after(anchor, &row),
        ]);
        let docs = self.insert_new_rows(&file, after, |_| {
            vec![SavedDocEntry {
                fragment: None,
                label: title.to_owned(),
            }]
        })?;
        let epub2 = self.opf.version.starts_with('2');
        let text = new_chapter_text(title, self.opf.language.as_deref(), epub2, stylesheets);
        Ok(PackagePlan {
            opf,
            docs,
            chapter: None,
            added: Some((file.clone(), text.into_bytes())),
            removed: None,
            delta: 1,
            landing: Some(file),
            record: None,
            restores_prior: false,
        })
    }

    fn plan_rename(
        &self,
        target: RowRef,
        title: &str,
        chapter: Option<ChapterModel>,
    ) -> Result<PackagePlan, Error> {
        let (entry, row) = self.rename_target(target)?;
        if row.is_some_and(|row| self.rows[row as usize].is_nav) {
            return Err(refuse("The contents page cannot be renamed."));
        }
        let label = xml_escape(title);
        let mut docs = Vec::with_capacity(self.docs.len());
        let old = entry.map(|at| self.docs[0].toc.entries[at as usize].label.clone());
        let fragment = entry.and_then(|at| self.docs[0].fragments[at as usize].clone());
        for (position, doc) in self.docs.iter().enumerate() {
            let at = if position == 0 && self.has_primary {
                entry
            } else {
                self.matching_entry(doc, row, fragment.as_deref(), old.as_deref())
            };
            let mut edits = DocEdits::default();
            if let Some(span) = at.and_then(|at| doc.toc.entries[at as usize].label_span) {
                edits.swap(span, label.clone());
            }
            docs.push(finish_toc(doc.toc, edits, &self.limits)?);
        }
        let opf = encode_text(self.opf.text_form, self.opf.source.clone());
        let keeps_file = fragment.is_some()
            && entry
                .is_some_and(|at| row.and_then(|row| self.docs[0].first_entry_of(row)) != Some(at));
        let written = if keeps_file {
            None
        } else {
            match (row, chapter) {
                (Some(row), Some(model)) => self
                    .renamed_chapter(row, &model, title, old.as_deref())?
                    .map(|bytes| (self.name_of(row), bytes)),
                _ => None,
            }
        };
        let changed_toc = docs
            .iter()
            .zip(self.unchanged_docs())
            .any(|(new, old)| *new != old);
        if written.is_none() && !changed_toc {
            return Err(refuse("This row has nothing to rename."));
        }
        Ok(PackagePlan {
            opf,
            docs,
            chapter: written.and_then(|(name, bytes)| Some((name?, bytes))),
            added: None,
            removed: None,
            delta: 0,
            landing: row.and_then(|row| self.name_of(row)),
            record: None,
            restores_prior: false,
        })
    }

    pub(crate) fn relabel(
        &self,
        titles: &HashMap<ChapterIx, String>,
    ) -> Result<PackagePlan, Error> {
        let number_of = |text: &str| chapter_title(text).map(|found| found.number);
        let mut docs = Vec::with_capacity(self.docs.len());
        for (position, doc) in self.docs.iter().enumerate() {
            let mut edits = DocEdits::default();
            let mut swapped = Vec::new();
            for (ix, title) in titles {
                let Some(row) = self
                    .row(*ix)
                    .ok()
                    .filter(|&row| !self.rows[row as usize].is_nav)
                else {
                    continue;
                };
                let at = if position == 0 && self.has_primary {
                    doc.entry_for_rename(row)
                } else {
                    self.matching_entry(doc, Some(row), None, None)
                };
                let Some(at) = at.filter(|&at| doc.fragments[at as usize].is_none()) else {
                    continue;
                };
                let entry = &doc.toc.entries[at as usize];
                if let Some(span) = entry.label_span
                    && entry.label != *title
                    && number_of(&entry.label)
                        .is_some_and(|number| Some(number) == number_of(title))
                {
                    edits.swap(span, xml_escape(title));
                    swapped.push((at, title));
                }
            }
            let bytes = finish_toc(doc.toc, edits, &self.limits)?;
            if !swapped.is_empty() {
                let parse = match doc.toc.kind {
                    TocSource::Ncx => TocDoc::parse_ncx,
                    _ => TocDoc::parse_nav,
                };
                let after = parse(bytes.clone(), &self.limits)?;
                let lost = after.entries.len() != doc.toc.entries.len()
                    || swapped
                        .iter()
                        .any(|(at, title)| after.entries[*at as usize].label != **title);
                if lost {
                    return Err(Error::WouldBreak(
                        "The contents could not be changed safely, so nothing was saved."
                            .to_owned(),
                    ));
                }
            }
            docs.push(bytes);
        }
        Ok(PackagePlan {
            opf: encode_text(self.opf.text_form, self.opf.source.clone()),
            docs,
            chapter: None,
            added: None,
            removed: None,
            delta: 0,
            landing: None,
            record: None,
            restores_prior: false,
        })
    }

    fn matching_entry(
        &self,
        doc: &Doc<'a>,
        row: Option<u32>,
        fragment: Option<&str>,
        old: Option<&str>,
    ) -> Option<u32> {
        let same_label = |at: &&u32| Some(doc.toc.entries[**at as usize].label.as_str()) == old;
        match row {
            Some(row) => {
                let entries = doc.of_row.get(&row)?;
                let same = |at: &&u32| doc.fragments[**at as usize].as_deref() == fragment;
                entries
                    .iter()
                    .filter(same)
                    .find(same_label)
                    .or_else(|| entries.iter().find(same))
                    .or_else(|| entries.iter().find(same_label))
                    .copied()
            }
            None => {
                let label = old?;
                (0..doc.toc.entries.len() as u32)
                    .find(|&at| doc.toc.entries[at as usize].label == label)
            }
        }
    }

    fn renamed_chapter(
        &self,
        row: u32,
        model: &ChapterModel,
        title: &str,
        old: Option<&str>,
    ) -> Result<Option<Vec<u8>>, Error> {
        let spans = heading_of(&model.source, &self.limits)?;
        let text = xml_escape(title);
        let mut edits = Vec::new();
        let swap = |edits: &mut Vec<SourceEdit>, span: Span| {
            edits.push(SourceEdit {
                span,
                text: text.clone(),
            });
        };
        let listed = self.is_listed(row);
        let heading_text = spans.heading.as_ref().map(|(_, found)| found.as_str());
        if let Some((span, found)) = &spans.heading
            && (!listed || old == Some(found.as_str()))
        {
            swap(&mut edits, *span);
        }
        if let Some((span, found)) = &spans.title
            && (old == Some(found.as_str()) || (!listed && heading_text == Some(found.as_str())))
        {
            swap(&mut edits, *span);
        }
        if !listed
            && spans.heading.is_none()
            && let Some(at) = spans.body_start
        {
            edits.push(SourceEdit {
                span: Span::new(at as usize, at as usize),
                text: format!("<h1>{text}</h1>"),
            });
        }
        if edits.is_empty() {
            return Ok(None);
        }
        Ok(Some(finish_bytes(model, edits).0))
    }

    fn closure(&self, seed: Vec<u32>) -> BTreeSet<u32> {
        let mut moved: BTreeSet<u32> = seed.iter().copied().collect();
        let mut queue = seed;
        while let Some(row) = queue.pop() {
            let Some(doc) = self.primary() else { break };
            let Some(entries) = doc.of_row.get(&row) else {
                continue;
            };
            for &at in entries {
                for found in doc.subtree_rows(at) {
                    if moved.insert(found) {
                        queue.push(found);
                    }
                }
            }
        }
        if self.primary().is_some() {
            let listed: Vec<u32> = moved
                .iter()
                .copied()
                .filter(|&row| self.is_listed(row))
                .collect();
            for row in listed {
                moved.extend(row + 1..self.carried_after(row));
            }
        }
        moved
    }

    fn move_plan(&self, row: RowRef, to: Spot) -> Result<MovePlan, Error> {
        let moved = match row {
            RowRef::Chapter(ix) => {
                let row = self.row(ix)?;
                self.closure(vec![row])
            }
            RowRef::Entry(entry) => {
                let (doc, at) = self.entry(entry)?;
                if let Some(row) = doc.rows[at as usize]
                    && doc.first_entry_of(row) != Some(at)
                {
                    return Err(refuse(
                        "This row is a section inside a chapter. Move its chapter instead.",
                    ));
                }
                let seed = doc.subtree_rows(at);
                if seed.is_empty() {
                    return Err(refuse("This row has no chapters to move."));
                }
                self.closure(seed)
            }
        };
        let rows = self.rows.len() as u32;
        let inside = || refuse("A chapter cannot move inside itself.");
        let free = |row: &u32| !moved.contains(row);
        let before_row = match to.row {
            RowRef::Chapter(ix) => {
                let row = self.row(ix)?;
                if moved.contains(&row) {
                    return Err(inside());
                }
                if to.after {
                    self.carried_after(row)
                } else {
                    row
                }
            }
            RowRef::Entry(entry) => {
                let (doc, at) = self.entry(entry)?;
                let end = subtree_end(doc.toc, at);
                let own = doc.subtree_rows(at);
                let kept: Vec<u32> = own.iter().copied().filter(free).collect();
                if !own.is_empty() && kept.is_empty() {
                    return Err(inside());
                }
                let edge = if to.after {
                    kept.iter().max().map(|&row| self.carried_after(row))
                } else {
                    kept.iter().min().copied()
                };
                edge.unwrap_or_else(|| {
                    (end..doc.toc.entries.len() as u32)
                        .find_map(|later| doc.rows[later as usize].filter(free))
                        .unwrap_or(rows)
                })
            }
        };
        let mut order: Vec<u32> = (0..rows).filter(|row| !moved.contains(row)).collect();
        let at = order.partition_point(|&row| row < before_row);
        let before_block = order.get(at).copied();
        let block: Vec<u32> = moved.iter().copied().collect();
        order.splice(at..at, block);
        Ok(MovePlan {
            moved,
            order,
            before_block,
        })
    }

    fn doc_place(
        &self,
        doc: &Doc<'a>,
        is_primary: bool,
        to: Spot,
        plan: &MovePlan,
        gone: &[bool],
    ) -> Option<(u32, String)> {
        let kept = |at: &u32| !gone[*at as usize];
        if is_primary && let RowRef::Entry(entry) = to.row {
            let row = doc.rows[entry as usize];
            let place = match (row, to.after) {
                (None, false) => Some(doc.at_before(entry)),
                (None, true) => Some(doc.at_after(entry)),
                (Some(row), false) => doc
                    .of_row
                    .get(&row)
                    .and_then(|entries| entries.iter().find(|at| kept(at)))
                    .map(|&first| doc.at_before(first)),
                (Some(row), true) => doc
                    .top_entries_of(row)
                    .iter()
                    .rfind(|at| kept(at))
                    .map(|&last| doc.at_after(last)),
            };
            if place.is_some() {
                return place;
            }
        }
        let spot_row = match to.row {
            RowRef::Chapter(ix) => Some(ix.0),
            RowRef::Entry(entry) => {
                let primary = &self.docs[0];
                let rows = primary.subtree_rows(entry);
                let kept_rows = rows.iter().copied().filter(|row| !plan.moved.contains(row));
                if to.after {
                    kept_rows.max()
                } else {
                    kept_rows.min()
                }
            }
        };
        let later = plan.before_block.and_then(|row| {
            let at = plan.order.iter().position(|&found| found == row)?;
            plan.order[at..]
                .iter()
                .filter(|row| !plan.moved.contains(row))
                .find_map(|&row| doc.of_row.get(&row)?.iter().find(|at| kept(at)).copied())
        });
        if let Some(row) = spot_row {
            if to.after {
                if let Some(&last) = doc.top_entries_of(row).iter().rfind(|at| kept(at)) {
                    let after = doc.at_after(last);
                    return Some(match later {
                        Some(next) if doc.toc.entries[next as usize].whole.start < after.0 => {
                            doc.at_before(next)
                        }
                        _ => after,
                    });
                }
            } else if let Some(first) = doc
                .of_row
                .get(&row)
                .and_then(|entries| entries.iter().find(|at| kept(at)))
            {
                return Some(doc.at_before(*first));
            }
        }
        match later {
            Some(first) => Some(doc.at_before(first)),
            None => doc.at_end(),
        }
    }

    fn plan_move(&self, row: RowRef, to: Spot) -> Result<PackagePlan, Error> {
        let plan = self.move_plan(row, to)?;
        let source = &self.opf.source;
        let mut spine = DocEdits::default();
        let mut block = String::new();
        for &moved in &plan.moved {
            let span = with_ws(source, self.opf.spine[moved as usize].span);
            spine.cut(span);
            block.push_str(&source[span.range()]);
        }
        let at = match plan.before_block {
            Some(row) => ws_start(source, self.opf.spine[row as usize].span.start as usize) as u32,
            None => {
                let last = (0..self.rows.len() as u32)
                    .rev()
                    .find(|row| !plan.moved.contains(row));
                last.map_or(self.opf.spine_end, |row| {
                    self.opf.spine[row as usize].span.end
                })
            }
        };
        spine.insert(at, block);
        let opf = encode_text(self.opf.text_form, splice(source, spine.finish()));
        let mut docs = Vec::with_capacity(self.docs.len());
        for (position, doc) in self.docs.iter().enumerate() {
            let is_primary = position == 0 && self.has_primary;
            let count = doc.toc.entries.len();
            let mut gone: Vec<bool> = (0..count)
                .map(|at| doc.rows[at].is_some_and(|row| plan.moved.contains(&row)))
                .collect();
            if is_primary && let RowRef::Entry(entry) = row {
                gone[entry as usize] = true;
            }
            if is_primary {
                for at in 0..count {
                    if let Some(parent) = doc.toc.entries[at].parent
                        && gone[parent as usize]
                    {
                        gone[at] = true;
                    }
                }
            }
            for at in (0..count).rev() {
                let entry = &doc.toc.entries[at];
                if entry.href.is_none()
                    && !entry.kids.is_empty()
                    && entry.kids.iter().all(|&kid| gone[kid as usize])
                {
                    gone[at] = true;
                }
            }
            let removal = Removal { gone };
            let mut edits = DocEdits::default();
            let mut text = String::new();
            for root in removal.roots(doc.toc) {
                removal.moved_text(doc.toc, root, &mut text);
            }
            if !text.is_empty() {
                removal.apply(doc.toc, &mut edits);
                if let Some((at, _)) = self.doc_place(doc, is_primary, to, &plan, &removal.gone) {
                    edits.insert(at, text);
                }
            }
            docs.push(finish_toc(doc.toc, edits, &self.limits)?);
        }
        Ok(PackagePlan {
            opf,
            docs,
            chapter: None,
            added: None,
            removed: None,
            delta: 0,
            landing: plan.moved.first().and_then(|&row| self.name_of(row)),
            record: None,
            restores_prior: false,
        })
    }

    fn refs_to_item(&self, item: u32, except_row: u32) -> bool {
        self.spine_items
            .iter()
            .enumerate()
            .any(|(row, found)| row as u32 != except_row && *found == Some(item))
    }

    fn landing_after_delete(&self, row: u32) -> Option<EntryName> {
        let linear = |at: &u32| {
            *at != row
                && self.rows[*at as usize].linear_ordinal.is_some()
                && self.rows[*at as usize].href.is_some()
        };
        (row + 1..self.rows.len() as u32)
            .find(linear)
            .or_else(|| (0..row).rev().find(linear))
            .and_then(|at| self.name_of(at))
    }

    fn plan_delete(&self, chapter: ChapterIx) -> Result<PackagePlan, Error> {
        let row = self.row(chapter)?;
        let info = &self.rows[row as usize];
        if info.is_nav {
            return Err(refuse(
                "This page is the book's table of contents, so it cannot be deleted.",
            ));
        }
        let others_left = self
            .rows
            .iter()
            .enumerate()
            .any(|(at, other)| at as u32 != row && !other.is_nav);
        if !others_left {
            return Err(refuse(
                "A book needs at least one chapter, so the last one cannot be deleted.",
            ));
        }
        let source = &self.opf.source;
        let mut opf_edits = DocEdits::default();
        let row_span = with_ws(source, self.opf.spine[row as usize].span);
        opf_edits.cut(row_span);
        let item = self.spine_items[row as usize];
        let file = info.href.clone();
        let mut removed = None;
        let mut item_id = String::new();
        let mut item_text = String::new();
        if let Some(item) = item
            && !self.refs_to_item(item, row)
        {
            let found = &self.opf.manifest[item as usize];
            let span = with_ws(source, found.span);
            opf_edits.cut(span);
            item_id = found.id.clone();
            item_text = source[span.range()].to_owned();
            let shared = self.item_files.iter().enumerate().any(|(at, other)| {
                at as u32 != item && other.is_some() && *other == self.item_files[item as usize]
            });
            if !shared {
                removed = file.clone().filter(|name| self.names.contains(name));
            }
        }
        if let Some(name) = &removed {
            let cut_all = self.opf.guide.iter().all(|guide| {
                resolve_href(
                    self.opf_name,
                    guide.href.split('#').next().unwrap_or_default(),
                )
                .ok()
                .flatten()
                .as_ref()
                    == Some(name)
            });
            for guide in &self.opf.guide {
                let path = guide.href.split('#').next().unwrap_or_default();
                if resolve_href(self.opf_name, path).ok().flatten().as_ref() == Some(name) {
                    opf_edits.cut(with_ws(source, guide.span));
                }
            }
            if cut_all && let Some(whole) = self.opf.guide_span {
                opf_edits.cut(with_ws(source, whole));
            }
        }
        let opf_list = opf_edits.finish();
        let mut restore = vec![(self.opf_name.as_str().to_owned(), invert(source, &opf_list))];
        let opf_text = splice(source, opf_list);
        let mut docs = Vec::with_capacity(self.docs.len());
        let mut saved = Vec::with_capacity(self.docs.len());
        for doc in &self.docs {
            let count = doc.toc.entries.len();
            let gone: Vec<bool> = (0..count)
                .map(|at| {
                    doc.rows[at] == Some(row)
                        || (doc.files[at].is_some() && doc.files[at] == removed)
                })
                .collect();
            let rows: Vec<SavedDocEntry> = (0..count)
                .filter(|&at| gone[at])
                .map(|at| SavedDocEntry {
                    fragment: doc.fragments[at].clone(),
                    label: doc.toc.entries[at].label.clone(),
                })
                .collect();
            saved.push((doc.name.as_str().to_owned(), rows));
            let removal = Removal { gone };
            let mut edits = DocEdits::default();
            removal.apply(doc.toc, &mut edits);
            if let Some(name) = &removed {
                self.cut_other_links(doc, name, &mut edits);
            }
            let mut list = edits.finish();
            list.extend(depth_fix(doc.toc, &list, &self.limits)?);
            restore.push((doc.name.as_str().to_owned(), invert(&doc.toc.source, &list)));
            docs.push(encode_text(
                doc.toc.text_form,
                splice(&doc.toc.source, list),
            ));
        }
        let opf = encode_text(self.opf.text_form, opf_text);
        let title = self
            .primary()
            .and_then(|doc| doc.entry_for_rename(row))
            .map(|at| self.docs[0].toc.entries[at as usize].label.clone())
            .unwrap_or_else(|| format!("Chapter {}", row + 1));
        let record = removed.as_ref().map(|name| {
            let mut after = vec![&opf];
            after.extend(docs.iter());
            DeletedRecord {
                version: 2,
                chapter: DeletedChapter {
                    id: super::DeletedId(0),
                    title,
                    deleted_at: 0,
                },
                entry: name.as_str().to_owned(),
                bytes: String::new(),
                crc32: 0,
                item_id,
                item_text,
                row_text: source[row_span.range()].to_owned(),
                spine_at: row,
                prev_idref: row
                    .checked_sub(1)
                    .map(|at| self.opf.spine[at as usize].idref.clone()),
                restore: restore
                    .into_iter()
                    .zip(after)
                    .map(|((name, edits), bytes)| RestoreDoc {
                        name,
                        after_crc: crc32fast::hash(bytes),
                        edits,
                    })
                    .collect(),
                toc: saved,
                linking: Vec::new(),
            }
        });
        Ok(PackagePlan {
            opf,
            docs,
            chapter: None,
            added: None,
            removed,
            delta: -1,
            landing: self.landing_after_delete(row),
            record,
            restores_prior: false,
        })
    }

    fn cut_other_links(&self, doc: &Doc<'a>, file: &EntryName, edits: &mut DocEdits) {
        let source = &doc.toc.source;
        let points_there = |href: &str| {
            let path = href.split('#').next().unwrap_or_default();
            !has_url_scheme(path)
                && resolve_href(doc.name, path).ok().flatten().as_ref() == Some(file)
        };
        for list in &doc.toc.others {
            let hits: Vec<_> = list
                .links
                .iter()
                .filter(|link| points_there(&link.href))
                .collect();
            if hits.is_empty() {
                continue;
            }
            if hits.len() == list.links.len() {
                edits.cut(with_ws(source, list.whole));
            } else {
                for link in hits {
                    edits.cut(with_ws(source, link.whole));
                }
            }
        }
    }

    pub(crate) fn documents_except(&self, removed: &EntryName) -> Vec<EntryName> {
        let is_toc = |name: &EntryName| self.docs.iter().any(|doc| doc.name == name);
        let mut names: Vec<EntryName> = self
            .opf
            .manifest
            .iter()
            .zip(&self.item_files)
            .filter(|(item, _)| matches!(item.media_type.as_str(), MIME_XHTML | "text/html"))
            .filter_map(|(_, file)| file.clone())
            .filter(|name| name != removed && !is_toc(name) && self.names.contains(name))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn plan_undo(&self, record: DeletedRecord) -> Result<PackagePlan, Error> {
        let file = EntryName::parse(&record.entry)
            .map_err(|_| refuse("That deleted chapter can no longer be restored."))?;
        if self.names.contains(&file) {
            return Err(refuse(
                "A file with this chapter's name is back in the book, so it cannot be restored.",
            ));
        }
        let bytes = hex_decode(&record.bytes)
            .filter(|bytes| crc32fast::hash(bytes) == record.crc32)
            .ok_or_else(|| refuse("The saved copy of this chapter failed its check."))?;
        let exact = |name: &str, current: &[u8]| -> Option<String> {
            let saved = record.restore.iter().find(|doc| doc.name == name)?;
            if saved.after_crc != crc32fast::hash(current) {
                return None;
            }
            let text = std::str::from_utf8(current).ok()?;
            let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
            Some(apply_inverse(text, &saved.edits))
        };
        let opf_now = encode_text(self.opf.text_form, self.opf.source.clone());
        let mut restores_prior = true;
        let opf = match exact(self.opf_name.as_str(), &opf_now) {
            Some(text) => encode_text(self.opf.text_form, text),
            None => {
                restores_prior = false;
                self.reinsert_in_opf(&record)?
            }
        };
        let after_row = self.row_after_restore(&record);
        let mut docs = Vec::with_capacity(self.docs.len());
        for doc in &self.docs {
            let now = encode_text(doc.toc.text_form, doc.toc.source.clone());
            match exact(doc.name.as_str(), &now) {
                Some(text) => docs.push(encode_text(doc.toc.text_form, text)),
                None => {
                    restores_prior = false;
                    let rows: Vec<SavedDocEntry> = record
                        .toc
                        .iter()
                        .find(|(name, _)| name == doc.name.as_str())
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default();
                    docs.push(self.reinsert_in_doc(doc, &file, after_row, rows)?);
                }
            }
        }
        Ok(PackagePlan {
            opf,
            docs,
            chapter: None,
            added: Some((file.clone(), bytes)),
            removed: None,
            delta: 1,
            landing: Some(file),
            record: Some(record),
            restores_prior,
        })
    }

    fn row_after_restore(&self, record: &DeletedRecord) -> Option<u32> {
        let by_name = record.prev_idref.as_deref().and_then(|idref| {
            self.opf
                .spine
                .iter()
                .position(|row| row.idref == idref)
                .map(|at| at as u32)
        });
        by_name.or_else(|| {
            record
                .spine_at
                .checked_sub(1)
                .map(|at| at.min(self.rows.len() as u32 - 1))
        })
    }

    fn reinsert_in_opf(&self, record: &DeletedRecord) -> Result<Vec<u8>, Error> {
        if self.opf.ids.contains(&record.item_id) {
            return Err(refuse(
                "Another entry now uses this chapter's id, so it cannot be restored.",
            ));
        }
        let source = &self.opf.source;
        let last = self.opf.manifest.last().map(|item| item.span);
        let item_at = last.map_or(self.opf.manifest_end, |span| span.end) as usize;
        let row_at = match self.row_after_restore(record) {
            Some(row) => self.opf.spine[row as usize].span.end as usize,
            None => ws_start(source, self.opf.spine[0].span.start as usize),
        };
        Ok(self.plan_of_opf(vec![
            SourceEdit {
                span: Span::new(item_at, item_at),
                text: record.item_text.clone(),
            },
            SourceEdit {
                span: Span::new(row_at, row_at),
                text: record.row_text.clone(),
            },
        ]))
    }

    fn reinsert_in_doc(
        &self,
        doc: &Doc<'a>,
        file: &EntryName,
        after: Option<u32>,
        rows: Vec<SavedDocEntry>,
    ) -> Result<Vec<u8>, Error> {
        let mut edits = DocEdits::default();
        let place = match after {
            Some(row) => doc.insertion_after_row(Some(row), self.rows.len() as u32),
            None => doc
                .toc
                .entries
                .iter()
                .position(|entry| entry.parent.is_none())
                .map(|at| doc.at_before(at as u32))
                .or_else(|| doc.at_end()),
        };
        if let Some((at, ws)) = place {
            let base = relative_href(doc.name, file);
            let mut used = HashSet::new();
            let mut text = String::new();
            for row in rows {
                let href = match &row.fragment {
                    Some(fragment) => format!("{base}#{fragment}"),
                    None => base.clone(),
                };
                text.push_str(&ws);
                text.push_str(&doc.new_entry(
                    &mut used,
                    &xml_escape(&href),
                    &xml_escape(&row.label),
                ));
            }
            edits.insert(at, text);
        }
        finish_toc(doc.toc, edits, &self.limits)
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

pub(crate) fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len() / 2)
        .map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok())
        .collect()
}

fn clean_language(language: &str) -> Result<String, Error> {
    let language = language.trim();
    let valid = !language.is_empty()
        && language.split('-').all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    if valid {
        Ok(language.to_owned())
    } else {
        Err(refuse(
            "That language code is not valid. Use a code such as en or pt-BR.",
        ))
    }
}

fn utc_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs()) as i64;
    let days = seconds.div_euclid(86_400);
    let left = seconds.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_shift = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_shift + 2) / 5 + 1;
    let month = if month_shift < 10 {
        month_shift + 3
    } else {
        month_shift - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        left / 3_600,
        left / 60 % 60,
        left % 60
    )
}

fn fresh_uuid(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
            .to_le_bytes(),
    );
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    let hex = hex_encode(&digest[..16]);
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

pub(crate) fn new_book_entries(meta: &NewBookMeta) -> Result<Vec<NewEntry>, Error> {
    let title = clean_title(&meta.title);
    if title.is_empty() {
        return Err(refuse("A new book needs a title."));
    }
    let author = clean_title(&meta.author);
    let language = clean_language(&meta.language)?;
    let first = clean_title(&meta.first_chapter);
    let first = if first.is_empty() {
        UNTITLED.to_owned()
    } else {
        first
    };
    let creator = if author.is_empty() {
        String::new()
    } else {
        format!("\n<dc:creator>{}</dc:creator>", xml_escape(&author))
    };
    let (title_text, first_text, lang) = (
        xml_escape(&title),
        xml_escape(&first),
        xml_escape(&language),
    );
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"book-id\">urn:uuid:{uuid}</dc:identifier>\n<dc:title>{title_text}</dc:title>{creator}\n<dc:language>{lang}</dc:language>\n<meta property=\"dcterms:modified\">{modified}</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"{MIME_XHTML}\" properties=\"nav\"/>\n<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n<item id=\"chapter-001\" href=\"chapter-001.xhtml\" media-type=\"{MIME_XHTML}\"/>\n</manifest>\n<spine toc=\"ncx\">\n<itemref idref=\"chapter-001\"/>\n</spine>\n</package>\n",
        uuid = fresh_uuid(&title),
        modified = utc_now(),
    );
    let nav = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head>\n<title>{title_text}</title>\n</head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n<ol>\n<li><a href=\"chapter-001.xhtml\">{first_text}</a></li>\n</ol>\n</nav>\n</body>\n</html>\n"
    );
    let ncx = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n<head>\n<meta name=\"dtb:depth\" content=\"1\"/>\n<meta name=\"dtb:totalPageCount\" content=\"0\"/>\n<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n</head>\n<docTitle><text>{title_text}</text></docTitle>\n<navMap>\n<navPoint id=\"nav-1\" playOrder=\"1\"><navLabel><text>{first_text}</text></navLabel><content src=\"chapter-001.xhtml\"/></navPoint>\n</navMap>\n</ncx>\n"
    );
    let chapter = new_chapter_text(&first, Some(&language), false, &[]);
    let container = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n".to_owned();
    let named = |path: String, text: String| -> Result<NewEntry, Error> {
        Ok(NewEntry::deflate(EntryName::parse(&path)?, text.as_bytes()))
    };
    Ok(vec![
        named("META-INF/container.xml".to_owned(), container)?,
        named(format!("{NEW_BOOK_DIR}/content.opf"), opf)?,
        named(format!("{NEW_BOOK_DIR}/nav.xhtml"), nav)?,
        named(format!("{NEW_BOOK_DIR}/toc.ncx"), ncx)?,
        named(format!("{NEW_BOOK_DIR}/chapter-001.xhtml"), chapter)?,
    ])
}
