use crate::book::Reader;
use crate::chapter::VisibleText;
use crate::zip::ReadAt;
use crate::{ChapterIx, DomRange, Error, VisRange};
use memchr::memmem;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

const CONTEXT_CHARS: usize = 48;
const COLLAPSED_CHARS: usize = 40;
const MAX_EXACT_BYTES: usize = 4096;
const LONE_MATCH_CHARS: usize = 16;
const MIN_CONTEXT_CHARS: usize = 4;
const BRIDGE_MIN_CHARS: usize = 8;
const BRIDGE_SLACK_BYTES: usize = 64;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub href: String,
    pub vis_hint: u32,
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorResolution {
    Exact { chapter: ChapterIx, range: VisRange },
    Moved { chapter: ChapterIx, range: VisRange },
    Detached,
}

#[derive(Clone, Copy)]
struct Found {
    score: usize,
    range: VisRange,
}

fn floor_char(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn chars_before(text: &str, at: usize, count: usize) -> usize {
    text[..at]
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(0, |(start, _)| start)
}

fn chars_after(text: &str, at: usize, count: usize) -> usize {
    text[at..]
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(offset, _)| at + offset)
}

fn collapsed_span(text: &str, at: usize) -> (usize, usize) {
    let line_end = text[at..]
        .find('\n')
        .map_or(text.len(), |offset| at + offset);
    if line_end > at {
        return (at, chars_after(text, at, COLLAPSED_CHARS).min(line_end));
    }
    let before = text[..at].trim_end_matches('\n').len();
    let line_start = text[..before].rfind('\n').map_or(0, |offset| offset + 1);
    (
        chars_before(text, before, COLLAPSED_CHARS).max(line_start),
        before,
    )
}

fn anchor_of(text: &str, href: String, range: VisRange) -> Anchor {
    let start = floor_char(text, range.start as usize);
    let end = floor_char(text, range.end as usize).max(start);
    let (start, end) = if start == end {
        collapsed_span(text, start)
    } else {
        (start, floor_char(text, end.min(start + MAX_EXACT_BYTES)))
    };
    Anchor {
        href,
        vis_hint: start as u32,
        exact: text[start..end].to_owned(),
        prefix: text[chars_before(text, start, CONTEXT_CHARS)..start].to_owned(),
        suffix: text[end..chars_after(text, end, CONTEXT_CHARS)].to_owned(),
    }
}

fn shared_tail(wanted: &str, text: &str) -> usize {
    wanted
        .chars()
        .rev()
        .zip(text.chars().rev())
        .take_while(|(one, other)| one == other)
        .count()
}

fn shared_head(wanted: &str, text: &str) -> usize {
    wanted
        .chars()
        .zip(text.chars())
        .take_while(|(one, other)| one == other)
        .count()
}

fn context_score(anchor: &Anchor, text: &str, start: usize, end: usize) -> usize {
    shared_tail(&anchor.prefix, &text[..start]) + shared_head(&anchor.suffix, &text[end..])
}

fn full_context(anchor: &Anchor) -> usize {
    anchor.prefix.chars().count() + anchor.suffix.chars().count()
}

fn nearer(hint: usize, at: usize, other: usize) -> bool {
    at.abs_diff(hint) < other.abs_diff(hint)
}

fn best_occurrence(anchor: &Anchor, text: &str) -> Option<Found> {
    let exact = anchor.exact.as_bytes();
    let hint = anchor.vis_hint as usize;
    let mut best: Option<Found> = None;
    for start in memmem::find_iter(text.as_bytes(), exact) {
        let end = start + exact.len();
        let score = context_score(anchor, text, start, end);
        let better = best.is_none_or(|found| {
            score > found.score
                || (score == found.score && nearer(hint, start, found.range.start as usize))
        });
        if better {
            best = Some(Found {
                score,
                range: VisRange {
                    start: start as u32,
                    end: end as u32,
                },
            });
        }
    }
    let lone = anchor.exact.chars().count() >= LONE_MATCH_CHARS;
    best.filter(|found| lone || found.score >= MIN_CONTEXT_CHARS)
}

fn bridged(anchor: &Anchor, text: &str) -> Option<Found> {
    if anchor.prefix.chars().count() < BRIDGE_MIN_CHARS
        || anchor.suffix.chars().count() < BRIDGE_MIN_CHARS
    {
        return None;
    }
    let reach = anchor.exact.len() * 2 + BRIDGE_SLACK_BYTES;
    let hint = anchor.vis_hint as usize;
    let mut best: Option<Found> = None;
    for at in memmem::find_iter(text.as_bytes(), anchor.prefix.as_bytes()) {
        let start = at + anchor.prefix.len();
        let window = floor_char(text, start + reach);
        let Some(gap) = text[start..window].find(anchor.suffix.as_str()) else {
            continue;
        };
        if gap == 0 {
            continue;
        }
        let closer = best.is_none_or(|found| nearer(hint, start, found.range.start as usize));
        if closer {
            best = Some(Found {
                score: 0,
                range: VisRange {
                    start: start as u32,
                    end: (start + gap) as u32,
                },
            });
        }
    }
    best
}

fn resolve_in(anchor: &Anchor, chapter: ChapterIx, text: &str) -> AnchorResolution {
    let empty = VisRange { start: 0, end: 0 };
    if anchor.exact.is_empty() {
        return AnchorResolution::Exact {
            chapter,
            range: empty,
        };
    }
    let Some(found) = best_occurrence(anchor, text).or_else(|| bridged(anchor, text)) else {
        if anchor.vis_hint == 0 && anchor.prefix.is_empty() {
            return AnchorResolution::Moved {
                chapter,
                range: empty,
            };
        }
        return AnchorResolution::Detached;
    };
    let in_place = found.range.start == anchor.vis_hint
        && found.range.end as usize - found.range.start as usize == anchor.exact.len()
        && found.score == full_context(anchor);
    if in_place {
        AnchorResolution::Exact {
            chapter,
            range: found.range,
        }
    } else {
        AnchorResolution::Moved {
            chapter,
            range: found.range,
        }
    }
}

impl<R: ReadAt + Sync> Reader<R> {
    pub fn anchor_at(&self, ix: ChapterIx, range: DomRange) -> Result<Anchor, Error> {
        let href = self
            .rows()
            .get(ix.0 as usize)
            .and_then(|row| row.href.as_ref())
            .ok_or_else(|| Error::Book("That chapter is not in this book.".to_owned()))?
            .as_str()
            .to_owned();
        let visible = self.visible_text(ix)?;
        Ok(anchor_of(visible.as_str(), href, visible.from_dom(range)))
    }

    pub fn resolve(&self, anchor: &Anchor) -> AnchorResolution {
        self.resolve_all(std::slice::from_ref(anchor))[0]
    }

    pub fn resolve_all(&self, anchors: &[Anchor]) -> Vec<AnchorResolution> {
        let mut resolved = vec![AnchorResolution::Detached; anchors.len()];
        let mut homeless = Vec::new();
        for (at, anchor) in anchors.iter().enumerate() {
            match self.chapter_of_entry(&anchor.href) {
                Some(chapter) => {
                    if let Ok(visible) = self.visible_text(chapter) {
                        resolved[at] = resolve_in(anchor, chapter, visible.as_str());
                    }
                }
                None if !anchor.exact.is_empty() => homeless.push(at),
                None => {}
            }
        }
        if !homeless.is_empty() {
            for (at, found) in homeless.iter().zip(self.scan_book(anchors, &homeless)) {
                if let Some((chapter, found)) = found {
                    resolved[*at] = AnchorResolution::Moved {
                        chapter,
                        range: found.range,
                    };
                }
            }
        }
        resolved
    }

    fn scan_book(&self, anchors: &[Anchor], wanted: &[usize]) -> Vec<Option<(ChapterIx, Found)>> {
        let rows = self.rows();
        let next = AtomicU32::new(0);
        let found = Mutex::new(vec![None; wanted.len()]);
        let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed) as usize;
                        let Some(row) = rows.get(at) else { break };
                        if row.is_nav {
                            continue;
                        }
                        let Ok(visible) = self.visible_text(row.ix) else {
                            continue;
                        };
                        self.note_matches(anchors, wanted, row.ix, &visible, &found);
                    }
                });
            }
        });
        found
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn note_matches(
        &self,
        anchors: &[Anchor],
        wanted: &[usize],
        chapter: ChapterIx,
        visible: &VisibleText,
        found: &Mutex<Vec<Option<(ChapterIx, Found)>>>,
    ) {
        for (slot, at) in wanted.iter().enumerate() {
            let Some(hit) = best_occurrence(&anchors[*at], visible.as_str()) else {
                continue;
            };
            let mut found = found
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let better = found[slot].is_none_or(|(ix, held): (ChapterIx, Found)| {
                hit.score > held.score || (hit.score == held.score && chapter < ix)
            });
            if better {
                found[slot] = Some((chapter, hit));
            }
        }
    }
}

#[cfg(test)]
mod tests;
