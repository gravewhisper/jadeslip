use crate::Limits;
use crate::package::{OpfDoc, TocDoc, TocSource, has_url_scheme, resolve_href};
use crate::zip::EntryName;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct Problem {
    pub(crate) kind: &'static str,
    pub(crate) detail: String,
}

pub(crate) fn refusal_for(problem: &Problem) -> String {
    let why = match problem.kind {
        "opf-markup" | "toc-markup" => "the package files would stop being well-formed",
        "duplicate-id" => "two package entries would share an id",
        "duplicate-href" => "two package entries would share a file",
        "missing-file" => "the package would list a file that is not in the book",
        "spine-idref" => "the reading order would point at a missing entry",
        "duplicate-idref" => "a chapter would appear twice in the reading order",
        "empty-spine" => "the book would have no chapters",
        "no-linear" => "the book would have no chapters in its reading order",
        "nav-count" => "the book would not have exactly one contents document",
        "dangling-ref" => "another part of the package depends on this chapter's file",
        "ncx-missing" => "the older contents file would be lost",
        "toc-empty" => "the table of contents would have no entries left",
        "toc-nested-empty" => "the table of contents would hold an empty list",
        "toc-href" => "the table of contents would link to a missing file",
        "toc-order" => "the table of contents would no longer follow the reading order",
        "play-order" => "the older contents file would lose its order",
        "duplicate-nav-point" => "two entries in the older contents file would share an id",
        _ => "the package would become invalid",
    };
    format!("This change was refused, because {why}. The book was not changed.")
}

fn add(problems: &mut BTreeSet<Problem>, kind: &'static str, detail: impl Into<String>) {
    problems.insert(Problem {
        kind,
        detail: detail.into(),
    });
}

fn check_toc(
    name: &EntryName,
    toc: &TocDoc,
    spine_rows: &HashMap<EntryName, u32>,
    names: &BTreeSet<EntryName>,
    problems: &mut BTreeSet<Problem>,
) {
    let doc = name.as_str();
    if toc.repaired {
        add(problems, "toc-markup", doc);
    }
    if toc.entries.is_empty() {
        add(problems, "toc-empty", doc);
    }
    let mut last_row = None;
    let mut in_order = true;
    let mut last_play = None;
    for entry in &toc.entries {
        if entry.nested.is_some() && entry.kids.is_empty() {
            add(problems, "toc-nested-empty", doc);
        }
        if let Some(span) = entry.play_order {
            let value = toc.source[span.range()].trim().parse::<u64>().ok();
            if value.is_none() || (last_play.is_some() && value <= last_play) {
                add(problems, "play-order", doc);
            }
            last_play = value;
        }
        let Some(href) = entry.href.as_deref() else {
            continue;
        };
        if has_url_scheme(href) {
            continue;
        }
        let Ok(Some(file)) = resolve_href(name, href) else {
            continue;
        };
        if !names.contains(&file) {
            add(problems, "toc-href", file.as_str());
        }
        if let Some(&row) = spine_rows.get(&file) {
            if last_row.is_some_and(|last| row < last) {
                in_order = false;
            }
            last_row = Some(row);
        }
    }
    if !in_order {
        add(problems, "toc-order", doc);
    }
    for list in &toc.others {
        for link in &list.links {
            if has_url_scheme(&link.href) {
                continue;
            }
            if let Ok(Some(file)) = resolve_href(name, &link.href)
                && !names.contains(&file)
            {
                add(problems, "toc-href", file.as_str());
            }
        }
    }
}

pub(crate) fn check_package<'n>(
    opf_name: &EntryName,
    opf: Vec<u8>,
    docs: impl Iterator<Item = (&'n EntryName, TocSource, Vec<u8>)>,
    names: &BTreeSet<EntryName>,
    limits: &Limits,
) -> BTreeSet<Problem> {
    let mut problems = BTreeSet::new();
    let doc = match OpfDoc::parse(opf, limits) {
        Ok(doc) => doc,
        Err(_) => {
            add(&mut problems, "empty-spine", "");
            return problems;
        }
    };
    if doc.repaired {
        add(&mut problems, "opf-markup", "");
    }
    let mut seen_ids = HashSet::new();
    for id in &doc.ids {
        if !seen_ids.insert(id.as_str()) {
            add(&mut problems, "duplicate-id", id.clone());
        }
    }
    let mut seen_files = HashSet::new();
    let mut files: HashMap<&str, Option<EntryName>> = HashMap::with_capacity(doc.manifest.len());
    let mut navs = 0;
    for item in &doc.manifest {
        let file = resolve_href(opf_name, &item.href).ok().flatten();
        if let Some(file) = &file {
            if !names.contains(file) {
                add(&mut problems, "missing-file", file.as_str());
            }
            if !seen_files.insert(file.clone()) {
                add(&mut problems, "duplicate-href", file.as_str());
            }
        }
        if item
            .properties
            .split_ascii_whitespace()
            .any(|property| property == "nav")
        {
            navs += 1;
        }
        if let Some(fallback) = &item.fallback
            && !seen_ids.contains(fallback.as_str())
        {
            add(&mut problems, "dangling-ref", fallback.clone());
        }
        files.insert(item.id.as_str(), file);
    }
    if navs != 1 {
        add(&mut problems, "nav-count", navs.to_string());
    }
    for target in doc.refines.iter().chain(doc.cover_id.iter()) {
        if !seen_ids.contains(target.as_str()) {
            add(&mut problems, "dangling-ref", target.clone());
        }
    }
    if let Some(ncx) = &doc.ncx_id
        && !files.contains_key(ncx.as_str())
    {
        add(&mut problems, "ncx-missing", ncx.clone());
    }
    let mut spine_rows: HashMap<EntryName, u32> = HashMap::with_capacity(doc.spine.len());
    let mut seen_refs = HashSet::new();
    if doc.spine.is_empty() {
        add(&mut problems, "empty-spine", "");
    }
    if !doc.spine.iter().any(|row| row.linear) {
        add(&mut problems, "no-linear", "");
    }
    for (at, row) in doc.spine.iter().enumerate() {
        if !seen_refs.insert(row.idref.as_str()) {
            add(&mut problems, "duplicate-idref", row.idref.clone());
        }
        match files.get(row.idref.as_str()) {
            None => add(&mut problems, "spine-idref", row.idref.clone()),
            Some(Some(file)) => {
                spine_rows.entry(file.clone()).or_insert(at as u32);
            }
            Some(None) => {}
        }
    }
    for (name, kind, bytes) in docs {
        let parsed = match kind {
            TocSource::Ncx => TocDoc::parse_ncx(bytes, limits),
            _ => TocDoc::parse_nav(bytes, limits),
        };
        match parsed {
            Ok(toc) => check_toc(name, &toc, &spine_rows, names, &mut problems),
            Err(_) => add(&mut problems, "toc-markup", name.as_str()),
        }
    }
    problems
}
