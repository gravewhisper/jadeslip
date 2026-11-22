mod check;
mod links;
mod plan;
mod toc_edit;

pub(crate) use check::{Problem, check_package, refusal_for};
pub(crate) use links::{sweep, unwrap_links};
pub(crate) use plan::{
    DeletedRecord, Doc, Gathered, Package, PackagePlan, hex_decode, hex_encode, stylesheets_of,
};

use crate::book::Reader;
use crate::commit::{Book, CommitReport};
use crate::markup::{SourceEdit, Span};
use crate::package::TextForm;
use crate::zip::{Changes, EntryFate, EntryName, NewEntry, ReadAt, ZipEntry};
use crate::{ChapterIx, Error, Limits};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

pub(crate) const UNTITLED: &str = "Untitled chapter";
const MAX_TITLE_CHARS: usize = 1_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RowRef {
    Chapter(ChapterIx),
    Entry(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Spot {
    pub row: RowRef,
    pub after: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeletedId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChapterOp {
    Add {
        after: Option<ChapterIx>,
        title: String,
    },
    Rename {
        row: RowRef,
        title: String,
    },
    Move {
        row: RowRef,
        to: Spot,
    },
    Delete {
        chapter: ChapterIx,
    },
    UndoDelete {
        deleted: DeletedId,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeletedChapter {
    pub id: DeletedId,
    pub title: String,
    pub deleted_at: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBookMeta {
    pub title: String,
    pub author: String,
    pub language: String,
    pub first_chapter: String,
}

#[derive(Debug)]
pub struct Managed {
    pub report: CommitReport,
    pub landing: Option<ChapterIx>,
    pub shown: Option<ChapterIx>,
    pub deleted: Option<DeletedId>,
}

pub(crate) fn clean_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '\u{FFFE}' | '\u{FFFF}'))
        .collect();
    let spaced: Vec<&str> = cleaned.split_whitespace().collect();
    spaced.join(" ").chars().take(MAX_TITLE_CHARS).collect()
}

pub(crate) fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn ws_start(source: &str, at: usize) -> usize {
    let bytes = source.as_bytes();
    let mut at = at;
    while at > 0 && bytes[at - 1].is_ascii_whitespace() {
        at -= 1;
    }
    at
}

pub(crate) fn with_ws(source: &str, span: Span) -> Span {
    Span::new(ws_start(source, span.start as usize), span.end as usize)
}

pub(crate) fn encode_text(form: TextForm, text: String) -> Vec<u8> {
    match form {
        TextForm::Utf8 { bom: true } => {
            let mut bytes = Vec::with_capacity(text.len() + 3);
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        _ => text.into_bytes(),
    }
}

pub(crate) struct PackageChanges {
    pub(crate) writes: HashMap<EntryName, NewEntry>,
    pub(crate) drops: HashSet<EntryName>,
    pub(crate) added: Vec<NewEntry>,
    pub(crate) delta: i64,
}

impl Changes for PackageChanges {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error> {
        if self.drops.contains(&entry.name) {
            return Ok(EntryFate::Drop);
        }
        Ok(match self.writes.remove(&entry.name) {
            Some(new) => EntryFate::Write(new),
            None => EntryFate::RawCopy,
        })
    }

    fn added(&mut self) -> Vec<NewEntry> {
        std::mem::take(&mut self.added)
    }

    fn spine_delta(&self) -> i64 {
        self.delta
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct InverseEdit {
    pub(crate) at: u32,
    pub(crate) len: u32,
    pub(crate) text: String,
}

pub(crate) fn invert(source: &str, edits: &[SourceEdit]) -> Vec<InverseEdit> {
    let mut sorted: Vec<&SourceEdit> = edits.iter().collect();
    sorted.sort_by_key(|edit| (edit.span.start, edit.span.end));
    let mut shift = 0i64;
    sorted
        .into_iter()
        .map(|edit| {
            let at = (i64::from(edit.span.start) + shift) as u32;
            shift += edit.text.len() as i64 - i64::from(edit.span.end - edit.span.start);
            InverseEdit {
                at,
                len: edit.text.len() as u32,
                text: source[edit.span.range()].to_owned(),
            }
        })
        .collect()
}

pub(crate) fn apply_inverse(source: &str, edits: &[InverseEdit]) -> String {
    let spliced = edits
        .iter()
        .map(|edit| SourceEdit {
            span: Span::new(edit.at as usize, (edit.at + edit.len) as usize),
            text: edit.text.clone(),
        })
        .collect();
    crate::markup::splice(source, spliced)
}

pub(crate) fn check_plan<R: ReadAt>(
    head: &Reader<R>,
    package: &Package<'_>,
    plan: &PackagePlan,
    limits: &Limits,
) -> Result<(), Error> {
    if plan.restores_prior {
        return Ok(());
    }
    let before = check_package(
        package.opf_name,
        package.opf.source.as_bytes().to_vec(),
        package
            .docs
            .iter()
            .map(|doc| (doc.name, doc.toc.kind, doc.toc.source.as_bytes().to_vec())),
        &head.entry_names(),
        limits,
    );
    let mut names: BTreeSet<EntryName> = head.entry_names();
    if let Some((name, _)) = &plan.added {
        names.insert(name.clone());
    }
    if let Some(name) = &plan.removed {
        names.remove(name);
    }
    let after = check_package(
        package.opf_name,
        plan.opf.clone(),
        package
            .docs
            .iter()
            .zip(&plan.docs)
            .map(|(doc, bytes)| (doc.name, doc.toc.kind, bytes.clone())),
        &names,
        limits,
    );
    let fresh: Vec<&Problem> = after.difference(&before).collect();
    match fresh.first() {
        Some(problem) => Err(Error::WouldBreak(refusal_for(problem))),
        None => Ok(()),
    }
}

pub fn check_new_book(meta: &NewBookMeta) -> Result<(), Error> {
    plan::new_book_entries(meta).map(|_| ())
}

pub fn new_book(
    path: &Path,
    data_dir: &Path,
    meta: &NewBookMeta,
    limits: Limits,
) -> Result<Book, Error> {
    Book::create(path, data_dir, plan::new_book_entries(meta)?, limits)
}

#[cfg(test)]
mod tests;
