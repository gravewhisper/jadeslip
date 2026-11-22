use super::plan::{hex_decode, hex_encode};
use crate::book::Reader;
use crate::chapter::{decode_and_model, decode_source};
use crate::edit::finish_bytes;
use crate::markup::{Attr, Event, SourceEdit, attr_value, local_name, scan};
use crate::package::{has_url_scheme, resolve_href};
use crate::zip::{EntryName, ReadAt};
use crate::{Error, Limits};
use memchr::memchr2_iter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct LinkWindow {
    pub(crate) name: String,
    pub(crate) after_crc: u32,
    pub(crate) at: u32,
    pub(crate) len: u32,
    pub(crate) bytes: String,
}

impl LinkWindow {
    fn between(name: &EntryName, old: &[u8], new: &[u8]) -> LinkWindow {
        let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let tail = old[head..]
            .iter()
            .rev()
            .zip(new[head..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        LinkWindow {
            name: name.as_str().to_owned(),
            after_crc: crc32fast::hash(new),
            at: head as u32,
            len: (new.len() - head - tail) as u32,
            bytes: hex_encode(&old[head..old.len() - tail]),
        }
    }

    pub(crate) fn undo(&self, current: &[u8]) -> Option<Vec<u8>> {
        if crc32fast::hash(current) != self.after_crc {
            return None;
        }
        let (at, len) = (self.at as usize, self.len as usize);
        let old = hex_decode(&self.bytes)?;
        let tail = current.get(at + len..)?;
        let mut restored = Vec::with_capacity(at + old.len() + tail.len());
        restored.extend_from_slice(current.get(..at)?);
        restored.extend_from_slice(&old);
        restored.extend_from_slice(tail);
        Some(restored)
    }
}

pub(crate) struct Rewritten {
    pub(crate) name: EntryName,
    pub(crate) bytes: Vec<u8>,
    pub(crate) window: LinkWindow,
}

#[derive(Default)]
pub(crate) struct Sweep {
    pub(crate) rewritten: Vec<Rewritten>,
    pub(crate) warnings: Vec<String>,
    pub(crate) unchecked: u32,
}

impl Sweep {
    pub(crate) fn all_warnings(mut self) -> (Vec<Rewritten>, Vec<String>) {
        if self.unchecked > 0 {
            self.warnings.push(format!(
                "{} other chapters could not be checked, so they may still link to the deleted chapter.",
                self.unchecked
            ));
        }
        (self.rewritten, self.warnings)
    }
}

fn is_link_element(name: &str) -> bool {
    let local = local_name(name);
    local.eq_ignore_ascii_case("a") || local.eq_ignore_ascii_case("area")
}

fn links_to_gone(attrs: &[Attr<'_>], from: &EntryName, gone: &BTreeSet<EntryName>) -> bool {
    let Some(href) = attr_value(attrs, "href") else {
        return false;
    };
    let path = href.trim();
    !has_url_scheme(path)
        && resolve_href(from, path)
            .ok()
            .flatten()
            .is_some_and(|target| gone.contains(&target))
}

fn plain_anchor_id<'s>(name: &str, attrs: &[Attr<'s>]) -> Option<&'s str> {
    if !local_name(name).eq_ignore_ascii_case("a") || name.contains(':') {
        return None;
    }
    let in_svg = attrs
        .iter()
        .any(|attr| attr.name.contains(':') && local_name(attr.name) == "href");
    if in_svg {
        return None;
    }
    attrs
        .iter()
        .find(|attr| attr.name == "id")
        .map(|attr| attr.raw_value)
}

fn quoted_id(raw: &str) -> String {
    raw.replace('"', "&quot;")
}

struct HeldLink<'s> {
    name: &'s str,
    close_as: Option<&'static str>,
}

fn unwrap_edits(
    events: &[Event<'_>],
    from: &EntryName,
    gone: &BTreeSet<EntryName>,
) -> Vec<SourceEdit> {
    let mut edits = Vec::new();
    let mut held: Vec<HeldLink<'_>> = Vec::new();
    for event in events {
        match event {
            Event::Open {
                name,
                attrs,
                span,
                self_closed,
            } if is_link_element(name) => {
                let doomed = links_to_gone(attrs, from, gone);
                let id = plain_anchor_id(name, attrs).filter(|_| doomed);
                if doomed {
                    let text = match (id.map(quoted_id), *self_closed) {
                        (Some(id), true) => format!("<span id=\"{id}\"></span>"),
                        (Some(id), false) => format!("<span id=\"{id}\">"),
                        (None, _) => String::new(),
                    };
                    edits.push(SourceEdit { span: *span, text });
                }
                if !*self_closed {
                    let close_as = doomed.then_some(if id.is_some() { "</span>" } else { "" });
                    held.push(HeldLink { name, close_as });
                }
            }
            Event::Close { name, span } if is_link_element(name) => {
                let found = held
                    .iter()
                    .rposition(|link| link.name.eq_ignore_ascii_case(name));
                if let Some(at) = found
                    && let Some(text) = held.remove(at).close_as
                {
                    edits.push(SourceEdit {
                        span: *span,
                        text: text.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    edits
}

fn may_hold_a_link(raw: &[u8]) -> bool {
    if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    memchr2_iter(b'h', b'H', raw).any(|at| {
        raw.get(at + 1..at + 4)
            .is_some_and(|rest| rest.eq_ignore_ascii_case(b"ref"))
    })
}

pub(crate) struct Unwrapped {
    pub(crate) bytes: Vec<u8>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn unwrap_links(
    raw: &[u8],
    from: &EntryName,
    gone: &BTreeSet<EntryName>,
    limits: &Limits,
) -> Result<Option<Unwrapped>, Error> {
    if !may_hold_a_link(raw) {
        return Ok(None);
    }
    let (source, _) = decode_source(raw.to_vec());
    let edits = unwrap_edits(&scan(&source, limits)?.events, from, gone);
    if edits.is_empty() {
        return Ok(None);
    }
    let model = decode_and_model(raw.to_vec(), limits)?;
    let (bytes, warnings) = finish_bytes(&model, edits);
    let after = decode_and_model(bytes.clone(), limits)?;
    if after.text.as_str() != model.text.as_str() {
        return Err(Error::WouldBreak(
            "Removing a link would have changed this chapter's text.".to_owned(),
        ));
    }
    Ok(Some(Unwrapped { bytes, warnings }))
}

pub(crate) fn sweep<R: ReadAt + Sync>(
    head: &Reader<R>,
    documents: &[EntryName],
    gone: &BTreeSet<EntryName>,
    limits: &Limits,
) -> Sweep {
    let next = AtomicUsize::new(0);
    let found = Mutex::new(Sweep::default());
    let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(name) = documents.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let outcome = head
                        .zip()
                        .entry(name)
                        .ok_or_else(|| Error::Book("The chapter is missing.".to_owned()))
                        .and_then(|entry| head.zip().read_entry(head.source(), entry, limits))
                        .and_then(|raw| {
                            let unwrapped = unwrap_links(&raw, name, gone, limits)?;
                            Ok((raw, unwrapped))
                        });
                    let mut found = found
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match outcome {
                        Ok((raw, Some(unwrapped))) => {
                            found.warnings.extend(unwrapped.warnings);
                            found.rewritten.push(Rewritten {
                                name: name.clone(),
                                window: LinkWindow::between(name, &raw, &unwrapped.bytes),
                                bytes: unwrapped.bytes,
                            });
                        }
                        Ok((_, None)) => {}
                        Err(_) => found.unchecked += 1,
                    }
                }
            });
        }
    });
    let mut found = found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    found.rewritten.sort_by(|a, b| a.name.cmp(&b.name));
    found
}
