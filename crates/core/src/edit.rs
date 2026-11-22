use crate::canon::{Atom, CanonToken, SourceAtom, Traced, canon_html, plain_atoms, to_xhtml};
use crate::chapter::{
    BlockKind, BlockLayout, ChapterModel, SourceEncoding, VisibleText, decode_source,
};
use crate::commit::CommitReport;
use crate::markup::{Event, SourceEdit, Span, is_space, scan, splice};
use crate::replace::FixOffer;
use crate::sanitize::{Context, Layout, UrlMap};
use crate::{BlockIx, ChapterRev, Error, Limits};
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

const MAX_DIFF_DEPTH: usize = 4096;
const MAX_DIFF_WORK: u64 = 200_000_000;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BlockEdits {
    pub base: ChapterRev,
    pub changes: Vec<BlockChange>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BlockChange {
    Replace { of: BlockIx, xml: String },
    Delete { of: BlockIx },
    InsertAfter { after: Option<BlockIx>, xml: String },
}

#[derive(Debug)]
pub enum SaveOutcome {
    Unchanged,
    Written(Box<CommitReport>),
}

#[derive(Debug)]
pub struct SavedChapter {
    pub rev: ChapterRev,
    pub outcome: SaveOutcome,
    pub after: Option<Vec<AfterBlock>>,
    pub warnings: Vec<String>,
    pub fix_offers: Vec<FixOffer>,
}

#[derive(Debug, Serialize)]
pub struct AfterBlock {
    pub origin: BlockOrigin,
    pub display: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "kind", content = "ix", rename_all = "camelCase")]
pub enum BlockOrigin {
    Kept(BlockIx),
    Change(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Hunk {
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
}

fn shortest_edit(old: &[Atom], new: &[Atom]) -> Option<Vec<Vec<u32>>> {
    let (n, m) = (old.len(), new.len());
    let limit = (n + m).min(MAX_DIFF_DEPTH);
    let width = 2 * limit + 3;
    let origin = limit as isize + 1;
    let mut furthest = vec![0u32; width];
    let mut trace = Vec::new();
    let mut work = 0u64;
    for depth in 0..=limit as isize {
        let mut diagonal = -depth;
        while diagonal <= depth {
            let ix = (origin + diagonal) as usize;
            let down =
                diagonal == -depth || (diagonal != depth && furthest[ix - 1] < furthest[ix + 1]);
            let mut x = if down {
                furthest[ix + 1] as usize
            } else {
                furthest[ix - 1] as usize + 1
            };
            let mut y = (x as isize - diagonal) as usize;
            while x < n && y < m && old[x] == new[y] {
                x += 1;
                y += 1;
            }
            work += 1 + (x as u64).saturating_sub(furthest[ix] as u64);
            furthest[ix] = x as u32;
            if x >= n && y >= m {
                trace
                    .push(furthest[(origin - depth) as usize..=(origin + depth) as usize].to_vec());
                return Some(trace);
            }
            diagonal += 2;
        }
        if work > MAX_DIFF_WORK {
            return None;
        }
        trace.push(furthest[(origin - depth) as usize..=(origin + depth) as usize].to_vec());
    }
    None
}

fn backtrack(trace: &[Vec<u32>], n: usize, m: usize) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut x, mut y) = (n as isize, m as isize);
    for depth in (1..trace.len() as isize).rev() {
        let previous = &trace[depth as usize - 1];
        let at = |diagonal: isize| previous[(diagonal + depth - 1) as usize] as isize;
        let diagonal = x - y;
        let down = diagonal == -depth || (diagonal != depth && at(diagonal - 1) < at(diagonal + 1));
        let from_diagonal = if down { diagonal + 1 } else { diagonal - 1 };
        let from_x = at(from_diagonal);
        let from_y = from_x - from_diagonal;
        let (edit_x, edit_y) = if down {
            (from_x, from_y + 1)
        } else {
            (from_x + 1, from_y)
        };
        let hunk = Hunk {
            old_start: from_x as usize,
            old_end: edit_x as usize,
            new_start: from_y as usize,
            new_end: edit_y as usize,
        };
        let joined = hunks
            .last_mut()
            .filter(|last| last.old_start == hunk.old_end && last.new_start == hunk.new_end);
        match joined {
            Some(last) => {
                last.old_start = hunk.old_start;
                last.new_start = hunk.new_start;
            }
            None => hunks.push(hunk),
        }
        x = from_x;
        y = from_y;
    }
    hunks.reverse();
    hunks
}

fn diff_atoms(old: &[Atom], new: &[Atom]) -> Vec<Hunk> {
    let shared = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[shared..]
        .iter()
        .rev()
        .zip(new[shared..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if shared == old.len() && shared == new.len() {
        return Vec::new();
    }
    let whole = Hunk {
        old_start: shared,
        old_end: old.len() - tail,
        new_start: shared,
        new_end: new.len() - tail,
    };
    let (old_middle, new_middle) = (
        &old[shared..old.len() - tail],
        &new[shared..new.len() - tail],
    );
    if old_middle.is_empty() || new_middle.is_empty() {
        return vec![whole];
    }
    let Some(trace) = shortest_edit(old_middle, new_middle) else {
        return vec![whole];
    };
    let mut hunks = backtrack(&trace, old_middle.len(), new_middle.len());
    for hunk in &mut hunks {
        hunk.old_start += shared;
        hunk.old_end += shared;
        hunk.new_start += shared;
        hunk.new_end += shared;
    }
    hunks
}

pub(crate) fn trim_to_diff(
    old: &[SourceAtom],
    new: &[Atom],
    fallback: u32,
    urls: &UrlMap,
) -> Vec<SourceEdit> {
    let old_atoms: Vec<Atom> = old.iter().map(|atom| atom.atom.clone()).collect();
    let mut edits = Vec::new();
    for hunk in diff_atoms(&old_atoms, new) {
        let text = to_xhtml(&new[hunk.new_start..hunk.new_end], urls);
        let removed = &old[hunk.old_start..hunk.old_end];
        let Some(first) = removed.first() else {
            let at = hunk
                .old_start
                .checked_sub(1)
                .map(|before| old[before].span.end)
                .or_else(|| old.first().map(|atom| atom.span.start))
                .unwrap_or(fallback);
            edits.push(SourceEdit {
                span: Span { start: at, end: at },
                text,
            });
            continue;
        };
        let mut text = Some(text);
        let mut run = first.span;
        for atom in &removed[1..] {
            if atom.span.start == run.end {
                run.end = atom.span.end;
            } else {
                edits.push(SourceEdit {
                    span: run,
                    text: text.take().unwrap_or_default(),
                });
                run = atom.span;
            }
        }
        edits.push(SourceEdit {
            span: run,
            text: text.take().unwrap_or_default(),
        });
    }
    edits
}

#[derive(Default)]
struct BlockPlan {
    replace: Option<(u32, Vec<Atom>)>,
    delete: bool,
    inserts_after: Vec<(u32, Vec<Atom>)>,
}

#[derive(Default)]
struct ParsedEdits {
    before_first: Vec<(u32, Vec<Atom>)>,
    blocks: Vec<BlockPlan>,
}

impl ParsedEdits {
    fn writes(&self) -> bool {
        !self.before_first.is_empty()
            || self
                .blocks
                .iter()
                .any(|plan| plan.delete || plan.replace.is_some() || !plan.inserts_after.is_empty())
    }

    fn order(&self) -> Vec<BlockOrigin> {
        let mut order: Vec<BlockOrigin> = self
            .before_first
            .iter()
            .map(|(ix, _)| BlockOrigin::Change(*ix))
            .collect();
        for (block, plan) in self.blocks.iter().enumerate() {
            if !plan.delete {
                order.push(match &plan.replace {
                    Some((ix, _)) => BlockOrigin::Change(*ix),
                    None => BlockOrigin::Kept(BlockIx(block as u32)),
                });
            }
            order.extend(
                plan.inserts_after
                    .iter()
                    .map(|(ix, _)| BlockOrigin::Change(*ix)),
            );
        }
        order
    }
}

fn parse_changes(
    edits: &BlockEdits,
    layout: &BlockLayout,
    ranges: &[Range<u32>],
    traced: &Traced<'_, '_>,
    context: &Context<'_>,
    limits: &Limits,
) -> Result<ParsedEdits, Error> {
    let known = |of: BlockIx| {
        if (of.0 as usize) < layout.blocks.len() {
            Ok(of.0 as usize)
        } else {
            Err(Error::Book(format!(
                "This edit names block {}, which this chapter does not have.",
                u64::from(of.0) + 1
            )))
        }
    };
    let once = |seen: &mut HashSet<usize>, of: BlockIx| {
        let block = known(of)?;
        if seen.insert(block) {
            Ok(block)
        } else {
            Err(Error::Book(format!(
                "This edit changes block {} twice.",
                u64::from(of.0) + 1
            )))
        }
    };
    let mut parsed = ParsedEdits::default();
    parsed
        .blocks
        .resize_with(layout.blocks.len(), BlockPlan::default);
    let mut seen = HashSet::new();
    for (ix, change) in edits.changes.iter().enumerate() {
        let ix = ix as u32;
        match change {
            BlockChange::Replace { of, xml } => {
                let block = once(&mut seen, *of)?;
                let mut new = canon_html(xml, context, limits)?;
                if layout.blocks[block].kind == BlockKind::BodyInner {
                    new = strip_wrapper(new)?;
                }
                let old = traced.atoms(ranges[block].clone())?;
                let same = |new: &[Atom]| old.iter().map(|atom| &atom.atom).eq(new);
                if same(&new) {
                    continue;
                }
                let new = without_lone_break(new);
                if !same(&new) {
                    parsed.blocks[block].replace = Some((ix, new));
                }
            }
            BlockChange::Delete { of } => {
                let block = once(&mut seen, *of)?;
                parsed.blocks[block].delete = true;
            }
            BlockChange::InsertAfter { after, xml } => {
                let new = canon_html(xml, context, limits)?;
                match after {
                    Some(of) => parsed.blocks[known(*of)?].inserts_after.push((ix, new)),
                    None => parsed.before_first.push((ix, new)),
                }
            }
        }
    }
    Ok(parsed)
}

fn indent_start(source: &str, floor: u32, start: u32) -> u32 {
    let bytes = source.as_bytes();
    let mut at = start as usize;
    while at > floor as usize && is_space(bytes[at - 1]) {
        at -= 1;
    }
    at as u32
}

fn strip_wrapper(mut atoms: Vec<Atom>) -> Result<Vec<Atom>, Error> {
    let is_tag = |atom: Option<&Atom>| matches!(atom, Some(Atom::Tag(_)));
    if atoms.len() < 2 || !is_tag(atoms.first()) || !is_tag(atoms.last()) {
        return Err(Error::Book(
            "This edit's block lost its wrapper element.".to_owned(),
        ));
    }
    atoms.pop();
    atoms.remove(0);
    Ok(atoms)
}

fn opens_br(atom: &Atom) -> bool {
    matches!(atom, Atom::Tag(tag) if matches!(&**tag, CanonToken::Open { name, .. } if name == "br"))
}

fn without_lone_break(mut atoms: Vec<Atom>) -> Vec<Atom> {
    if atoms.len() == 4 && opens_br(&atoms[1]) {
        atoms.drain(1..3);
    }
    atoms
}

fn id_of(atom: &Atom) -> Option<&str> {
    let Atom::Tag(tag) = atom else { return None };
    let CanonToken::Open { attrs, .. } = &**tag else {
        return None;
    };
    attrs
        .iter()
        .find(|(name, _)| name == "id")
        .map(|(_, value)| value.as_str())
}

fn drop_duplicate_ids(atoms: &mut [Atom], own: &[SourceAtom], taken: &mut HashSet<String>) {
    let mut allowed: HashMap<&str, u32> = HashMap::new();
    for id in own.iter().filter_map(|atom| id_of(&atom.atom)) {
        *allowed.entry(id).or_default() += 1;
    }
    for atom in atoms {
        let Some(id) = id_of(atom).map(str::to_owned) else {
            continue;
        };
        if let Some(count) = allowed.get_mut(id.as_str()).filter(|count| **count > 0) {
            *count -= 1;
            taken.insert(id);
        } else if !taken.insert(id)
            && let Atom::Tag(tag) = atom
            && let CanonToken::Open { attrs, .. } = &mut **tag
        {
            attrs.retain(|(name, _)| name != "id");
        }
    }
}

struct Segments {
    gaps: Vec<Vec<SourceAtom>>,
    blocks: Vec<Vec<SourceAtom>>,
}

fn segments_of(traced: &Traced<'_, '_>, ranges: &[Range<u32>]) -> Result<Segments, Error> {
    let mut segments = Segments {
        gaps: Vec::with_capacity(ranges.len() + 1),
        blocks: Vec::with_capacity(ranges.len()),
    };
    let mut cursor = 0;
    for range in ranges {
        segments.gaps.push(traced.atoms(cursor..range.start)?);
        segments.blocks.push(traced.atoms(range.clone())?);
        cursor = range.end;
    }
    segments
        .gaps
        .push(traced.atoms(cursor..traced.events.len() as u32)?);
    Ok(segments)
}

fn dedupe_new_ids(parsed: &mut ParsedEdits, segments: &Segments) {
    let mut taken: HashSet<String> = HashSet::new();
    let kept_blocks = segments
        .blocks
        .iter()
        .zip(&parsed.blocks)
        .filter(|(_, plan)| !plan.delete && plan.replace.is_none())
        .map(|(atoms, _)| atoms);
    for atoms in segments.gaps.iter().chain(kept_blocks) {
        taken.extend(
            atoms
                .iter()
                .filter_map(|atom| id_of(&atom.atom))
                .map(str::to_owned),
        );
    }
    for (plan, own) in parsed.blocks.iter_mut().zip(&segments.blocks) {
        if let Some((_, new)) = &mut plan.replace {
            drop_duplicate_ids(new, own, &mut taken);
        }
    }
    let inserts = parsed.before_first.iter_mut().chain(
        parsed
            .blocks
            .iter_mut()
            .flat_map(|plan| plan.inserts_after.iter_mut()),
    );
    for (_, new) in inserts {
        drop_duplicate_ids(new, &[], &mut taken);
    }
}

#[derive(Default)]
struct Assembled {
    trimmed: Vec<SourceEdit>,
    widened: Vec<SourceEdit>,
    intended: Vec<Atom>,
}

impl Assembled {
    fn both(&mut self, edit: SourceEdit) {
        self.trimmed.push(edit.clone());
        self.widened.push(edit);
    }

    fn kept(&mut self, atoms: &[SourceAtom]) {
        self.intended
            .extend(atoms.iter().map(|atom| atom.atom.clone()));
    }
}

fn zero_width(at: u32) -> Span {
    Span { start: at, end: at }
}

fn joined_split(first: &[Atom], rest: &[(u32, Vec<Atom>)], whitespace: &[Atom]) -> Vec<Atom> {
    let mut joined = first.to_vec();
    for (_, part) in rest {
        joined.extend_from_slice(whitespace);
        joined.extend_from_slice(part);
    }
    joined
}

fn assemble(
    source: &str,
    layout: &BlockLayout,
    parsed: &ParsedEdits,
    segments: &Segments,
    urls: &UrlMap,
) -> Assembled {
    let mut plan = Assembled::default();
    let mut previous_end = 0;
    if layout.blocks.is_empty() {
        for (_, new) in &parsed.before_first {
            plan.both(SourceEdit {
                span: zero_width(layout.body_inner.start),
                text: to_xhtml(new, urls),
            });
            plan.intended.extend(new.iter().cloned());
        }
    }
    for (ix, block) in layout.blocks.iter().enumerate() {
        let change = &parsed.blocks[ix];
        let span = block.span;
        let indent = indent_start(source, previous_end, span.start);
        let whitespace = &source[indent as usize..span.start as usize];
        let whitespace_atoms = plain_atoms(whitespace);
        previous_end = span.end;

        let gap = &segments.gaps[ix];
        if change.delete {
            let before_indent = gap.partition_point(|atom| atom.span.start < indent);
            plan.kept(&gap[..before_indent]);
        } else {
            plan.kept(gap);
        }

        let mut lead = String::new();
        if ix == 0 {
            for (_, new) in &parsed.before_first {
                lead.push_str(&to_xhtml(new, urls));
                lead.push_str(whitespace);
                plan.intended.extend(new.iter().cloned());
                plan.intended.extend(whitespace_atoms.iter().cloned());
            }
        }
        if change.delete {
            plan.both(SourceEdit {
                span: Span {
                    start: indent,
                    end: span.end,
                },
                text: lead,
            });
        } else {
            if !lead.is_empty() {
                plan.both(SourceEdit {
                    span: zero_width(span.start),
                    text: lead,
                });
            }
            if let Some((_, new)) = &change.replace {
                let joined = joined_split(new, &change.inserts_after, &whitespace_atoms);
                let old = &segments.blocks[ix];
                plan.trimmed
                    .extend(trim_to_diff(old, &joined, span.start, urls));
                plan.widened.push(SourceEdit {
                    span,
                    text: to_xhtml(&joined, urls),
                });
                plan.intended.extend(joined);
                continue;
            }
            plan.kept(&segments.blocks[ix]);
        }
        for (_, new) in &change.inserts_after {
            plan.both(SourceEdit {
                span: zero_width(span.end),
                text: format!("{whitespace}{}", to_xhtml(new, urls)),
            });
            plan.intended.extend(whitespace_atoms.iter().cloned());
            plan.intended.extend(new.iter().cloned());
        }
    }
    if let Some(tail) = segments.gaps.last() {
        plan.kept(tail);
    }
    plan
}

pub(crate) struct Planned {
    pub(crate) bytes: Vec<u8>,
    pub(crate) warnings: Vec<String>,
    pub(crate) order: Vec<BlockOrigin>,
}

pub(crate) enum ChapterPlan {
    Unchanged { order: Vec<BlockOrigin> },
    Write(Planned),
}

#[cfg(test)]
pub(crate) mod seam {
    use std::cell::Cell;
    thread_local! {
        pub(crate) static CORRUPT_ATTEMPTS: Cell<u32> = const { Cell::new(0) };
        pub(crate) static ATTEMPTS: Cell<u32> = const { Cell::new(0) };
    }
}

#[cfg(test)]
fn corrupt_if_asked(edits: &mut [SourceEdit]) {
    seam::ATTEMPTS.with(|count| count.set(count.get() + 1));
    let due = seam::CORRUPT_ATTEMPTS.with(|left| {
        let due = left.get() > 0;
        left.set(left.get().saturating_sub(1));
        due
    });
    if due && let Some(edit) = edits.first_mut() {
        edit.text.push_str("<!--corrupt-->X");
    }
}

fn repair_edits(model: &ChapterModel, events: &[Event<'_>]) -> Vec<SourceEdit> {
    let mut edits = model.fixes.point.clone();
    for event in events {
        match event {
            Event::Close { name, span } if span.start == span.end => edits.push(SourceEdit {
                span: *span,
                text: format!("</{name}>"),
            }),
            Event::Other { span } if model.source[span.range()].starts_with("</") => {
                edits.push(SourceEdit {
                    span: *span,
                    text: String::new(),
                });
            }
            _ => {}
        }
    }
    edits
}

struct Verify<'a> {
    intended: &'a [Atom],
    must_be_clean: bool,
    text_must_agree: bool,
    context: &'a Context<'a>,
    limits: &'a Limits,
}

fn verify(written: &str, check: &Verify<'_>) -> bool {
    let Ok(scanned) = scan(written, check.limits) else {
        return false;
    };
    if check.must_be_clean && (scanned.fixes.structural || !scanned.fixes.point.is_empty()) {
        return false;
    }
    let (layout, ranges) = BlockLayout::build(&scanned.events, written);
    let traced = Traced::new(
        &scanned.events,
        written.len(),
        &Layout {
            blocks: &layout.blocks,
            ranges: &ranges,
            body_inner: layout.body_inner,
        },
        check.context,
    );
    let Ok(atoms) = traced.atoms(0..scanned.events.len() as u32) else {
        return false;
    };
    if !atoms.iter().map(|atom| &atom.atom).eq(check.intended) {
        return false;
    }
    !check.text_must_agree || text_agrees(&scanned.events, written, &layout, &ranges, &traced)
}

fn text_agrees(
    events: &[Event<'_>],
    source: &str,
    layout: &BlockLayout,
    ranges: &[Range<u32>],
    traced: &Traced<'_, '_>,
) -> bool {
    let visible = VisibleText::fold(events, source, &layout.blocks, ranges);
    let units: Result<Vec<Vec<u32>>, Error> =
        ranges.iter().map(|range| traced.dom_units(range)).collect();
    units.is_ok_and(|units| visible.check_dom_blocks(&units).is_ok())
}

pub(crate) fn plan_chapter(
    model: &ChapterModel,
    edits: &BlockEdits,
    context: &Context<'_>,
    limits: &Limits,
) -> Result<ChapterPlan, Error> {
    let scanned = scan(&model.source, limits)?;
    let (layout, ranges) = BlockLayout::build(&scanned.events, &model.source);
    let traced = Traced::new(
        &scanned.events,
        model.source.len(),
        &Layout {
            blocks: &layout.blocks,
            ranges: &ranges,
            body_inner: layout.body_inner,
        },
        context,
    );
    let mut parsed = parse_changes(edits, &layout, &ranges, &traced, context, limits)?;
    let order = parsed.order();
    if !parsed.writes() {
        return Ok(ChapterPlan::Unchanged { order });
    }
    let segments = segments_of(&traced, &ranges)?;
    dedupe_new_ids(&mut parsed, &segments);
    let urls = &context.urls.map;
    let assembled = assemble(&model.source, &layout, &parsed, &segments, urls);

    let structural = model.fixes.structural;
    let repair = repair_edits(model, &scanned.events);
    let mut warnings = Vec::new();
    if structural || !repair.is_empty() {
        warnings.push("This chapter's markup was damaged and this save repaired it.".to_owned());
    }
    let check = Verify {
        intended: &assembled.intended,
        must_be_clean: !structural,
        text_must_agree: text_agrees(&scanned.events, &model.source, &layout, &ranges, &traced),
        context,
        limits,
    };
    let wider = if structural {
        vec![SourceEdit {
            span: layout.body_inner,
            text: to_xhtml(&assembled.intended, urls),
        }]
    } else {
        assembled.widened
    };
    for mut attempt in [assembled.trimmed, wider] {
        attempt.extend(repair.iter().cloned());
        #[cfg(test)]
        corrupt_if_asked(&mut attempt);
        let (bytes, encoding_warnings) = finish_bytes(model, attempt);
        let (written, _) = decode_source(bytes.clone());
        if verify(&written, &check) {
            warnings.extend(encoding_warnings);
            if structural && scan(&written, limits).is_ok_and(|after| after.fixes.structural) {
                warnings.push(
                    "Part of this chapter's damaged markup could not be repaired.".to_owned(),
                );
            }
            return Ok(ChapterPlan::Write(Planned {
                bytes,
                warnings,
                order,
            }));
        }
    }
    Err(Error::WouldBreak(
        "This edit could not be saved safely, so the book was not changed. Your text is still on screen."
            .to_owned(),
    ))
}

fn round_trips(encoding: &'static Encoding, raw: &[u8], source: &str) -> bool {
    let (encoded, _, had_errors) = encoding.encode(source);
    !had_errors && encoded.as_ref() == raw
}

pub(crate) fn finish_bytes(model: &ChapterModel, edits: Vec<SourceEdit>) -> (Vec<u8>, Vec<String>) {
    let mut warnings = Vec::new();
    match &model.encoding {
        SourceEncoding::Utf8 { bom } => {
            let spliced = splice(&model.source, edits);
            let mut bytes = Vec::with_capacity(spliced.len() + 3);
            if *bom {
                bytes.extend_from_slice(b"\xEF\xBB\xBF");
            }
            bytes.extend_from_slice(spliced.as_bytes());
            (bytes, warnings)
        }
        SourceEncoding::Other { encoding, raw } => {
            let codec = Encoding::for_label_no_replacement(encoding.as_bytes())
                .unwrap_or(encoding_rs::UTF_8);
            let utf16 = codec == encoding_rs::UTF_16LE || codec == encoding_rs::UTF_16BE;
            if !utf16 && codec != encoding_rs::UTF_8 && round_trips(codec, raw, &model.source) {
                let spliced = splice(&model.source, edits);
                let (encoded, _, _) = codec.encode(&spliced);
                return (encoded.into_owned(), warnings);
            }
            let mut converted = edits;
            let mut declarations: Vec<SourceEdit> = model
                .layout
                .declarations
                .iter()
                .map(|span| SourceEdit {
                    span: *span,
                    text: "UTF-8".to_owned(),
                })
                .collect();
            converted.retain(|edit| {
                !declarations
                    .iter()
                    .any(|declared| declared.span == edit.span)
            });
            converted.append(&mut declarations);
            warnings.push(format!(
                "This chapter was stored as {encoding} and could not be kept in that encoding, so this save converted it to UTF-8."
            ));
            let spliced = splice(&model.source, converted);
            (spliced.into_bytes(), warnings)
        }
    }
}

#[cfg(test)]
mod tests;
