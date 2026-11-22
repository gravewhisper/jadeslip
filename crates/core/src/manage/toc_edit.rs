use super::{with_ws, ws_start};
use crate::markup::{SourceEdit, Span};
use crate::package::{TocDoc, TocSource};

#[derive(Default)]
pub(crate) struct DocEdits {
    cuts: Vec<Span>,
    inserts: Vec<(u32, String)>,
    swaps: Vec<SourceEdit>,
}

impl DocEdits {
    pub(crate) fn cut(&mut self, span: Span) {
        self.cuts.push(span);
    }

    pub(crate) fn insert(&mut self, at: u32, text: String) {
        self.inserts.push((at, text));
    }

    pub(crate) fn swap(&mut self, span: Span, text: String) {
        self.swaps.push(SourceEdit { span, text });
    }

    pub(crate) fn finish(self) -> Vec<SourceEdit> {
        let mut cuts = self.cuts;
        cuts.sort_by_key(|span| span.start);
        let mut merged: Vec<Span> = Vec::with_capacity(cuts.len());
        for span in cuts {
            match merged.last_mut() {
                Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
                _ => merged.push(span),
            }
        }
        let outside = |at: u32| -> u32 {
            let found = merged.partition_point(|span| span.end <= at);
            match merged.get(found) {
                Some(span) if span.start < at => span.start,
                _ => at,
            }
        };
        let mut edits: Vec<SourceEdit> = Vec::with_capacity(merged.len() + self.inserts.len());
        for (at, text) in self.inserts {
            let at = outside(at) as usize;
            edits.push(SourceEdit {
                span: Span::new(at, at),
                text,
            });
        }
        edits.extend(self.swaps.into_iter().filter(|swap| {
            let found = merged.partition_point(|span| span.end <= swap.span.start);
            merged
                .get(found)
                .is_none_or(|span| span.start >= swap.span.end)
        }));
        edits.extend(merged.into_iter().map(|span| SourceEdit {
            span,
            text: String::new(),
        }));
        edits
    }
}

pub(crate) struct Removal {
    pub(crate) gone: Vec<bool>,
}

impl Removal {
    pub(crate) fn apply(&self, toc: &TocDoc, edits: &mut DocEdits) {
        let count = toc.entries.len();
        let mut vanish = vec![false; count];
        for at in (0..count).rev() {
            let entry = &toc.entries[at];
            let kids_vanish = entry.kids.iter().all(|&kid| vanish[kid as usize]);
            vanish[at] = if self.gone[at] {
                kids_vanish
            } else {
                !entry.kids.is_empty() && kids_vanish && entry.href.is_none()
            };
        }
        for at in 0..count {
            let entry = &toc.entries[at];
            let parent_vanishes = entry.parent.is_some_and(|parent| vanish[parent as usize]);
            if vanish[at] {
                if !parent_vanishes {
                    edits.cut(with_ws(&toc.source, entry.whole));
                }
            } else if self.gone[at] {
                let first = &toc.entries[entry.kids[0] as usize];
                let last =
                    &toc.entries[*entry.kids.last().expect("a kept group has kids") as usize];
                edits.cut(Span::new(
                    ws_start(&toc.source, entry.whole.start as usize),
                    ws_start(&toc.source, first.whole.start as usize),
                ));
                edits.cut(Span::new(last.whole.end as usize, entry.whole.end as usize));
            } else if !entry.kids.is_empty()
                && entry.kids.iter().all(|&kid| vanish[kid as usize])
                && let Some(nested) = entry.nested
            {
                edits.cut(with_ws(&toc.source, nested));
            }
        }
    }

    fn all_gone(&self, toc: &TocDoc, at: u32) -> bool {
        (at..subtree_end(toc, at)).all(|inner| self.gone[inner as usize])
    }

    pub(crate) fn moved_text(&self, toc: &TocDoc, at: u32, out: &mut String) {
        let source = &toc.source;
        let entry = &toc.entries[at as usize];
        let whole = with_ws(source, entry.whole);
        if self.all_gone(toc, at) {
            out.push_str(&source[whole.range()]);
            return;
        }
        let first = toc.entries[entry.kids[0] as usize].whole.start as usize;
        let last = toc.entries[*entry.kids.last().expect("it has kids") as usize]
            .whole
            .end as usize;
        let moving: Vec<u32> = entry
            .kids
            .iter()
            .copied()
            .filter(|&kid| self.gone[kid as usize])
            .collect();
        let (head_end, tail_start) = match entry.nested {
            Some(nested) if moving.is_empty() => {
                (ws_start(source, nested.start as usize), nested.end as usize)
            }
            _ => (ws_start(source, first), last),
        };
        out.push_str(&source[whole.start as usize..head_end]);
        for kid in moving {
            self.moved_text(toc, kid, out);
        }
        out.push_str(&source[tail_start..whole.end as usize]);
    }

    pub(crate) fn roots(&self, toc: &TocDoc) -> Vec<u32> {
        (0..toc.entries.len() as u32)
            .filter(|&at| {
                self.gone[at as usize]
                    && toc.entries[at as usize]
                        .parent
                        .is_none_or(|parent| !self.gone[parent as usize])
            })
            .collect()
    }
}

pub(crate) fn subtree_end(toc: &TocDoc, at: u32) -> u32 {
    let depth = toc.entries[at as usize].depth;
    let mut end = at + 1;
    while (end as usize) < toc.entries.len() && toc.entries[end as usize].depth > depth {
        end += 1;
    }
    end
}

pub(crate) fn entry_markup(
    toc: &TocDoc,
    href: &str,
    label: &str,
    nav_point: Option<(&str, Option<u64>)>,
) -> String {
    let p = &toc.prefix;
    match (toc.kind, nav_point) {
        (TocSource::Ncx, Some((id, play_order))) => {
            let order = play_order.map_or(String::new(), |value| format!(" playOrder=\"{value}\""));
            format!(
                "<{p}navPoint id=\"{id}\"{order}><{p}navLabel><{p}text>{label}</{p}text></{p}navLabel><{p}content src=\"{href}\"/></{p}navPoint>"
            )
        }
        _ => format!("<{p}li><{p}a href=\"{href}\">{label}</{p}a></{p}li>"),
    }
}
