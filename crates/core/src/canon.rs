use crate::chapter::BREAK_UNIT;
use crate::chapter::{Block, BlockKind};
use crate::markup::{
    Attr, ElementClass, Event, Span, char_ref, class_of, decode_attr, decode_text,
    is_invisible_char, is_void, local_name, scan,
};
use crate::sanitize::{Context, Layout, ResourceUrls, UrlMap, push_escaped_attr, trace_output};
use crate::{DEFAULT_LIMITS, Error, Limits};
use std::ops::Range;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CanonToken {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Close {
        name: String,
    },
    Text(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Atom {
    Char(char),
    Tag(Box<CanonToken>),
}

#[derive(Clone, Debug)]
pub(crate) struct SourceAtom {
    pub(crate) atom: Atom,
    pub(crate) span: Span,
}

const SCHEME_PREFIX: &str = "jadeslip-book://";
const SESSION_HEX_LEN: usize = 16;

fn without_session(value: String) -> String {
    let Some(tail) = value.strip_prefix(SCHEME_PREFIX) else {
        return value;
    };
    match tail.get(SESSION_HEX_LEN..) {
        Some(rest) if rest.starts_with('/') => format!("{SCHEME_PREFIX}session{rest}"),
        _ => value,
    }
}

fn is_ignored_attr(name: &str) -> bool {
    name == "xmlns" || name.starts_with("xmlns:") || name.starts_with("data-ss-")
}

fn open_token(name: &str, attrs: &[Attr<'_>]) -> CanonToken {
    let mut canon: Vec<(String, String)> = attrs
        .iter()
        .filter(|attr| !is_ignored_attr(attr.name))
        .map(|attr| (attr.name.to_owned(), decode_attr(attr.raw_value)))
        .collect();
    canon.sort();
    CanonToken::Open {
        name: name.to_owned(),
        attrs: canon,
    }
}

fn fold(ch: char) -> char {
    if ch == '\u{A0}' { ' ' } else { ch }
}

fn push_text(tokens: &mut Vec<CanonToken>, decoded: &str) {
    let text: String = decoded.chars().map(fold).collect();
    if text.is_empty() {
        return;
    }
    if let Some(CanonToken::Text(last)) = tokens.last_mut() {
        last.push_str(&text);
    } else {
        tokens.push(CanonToken::Text(text));
    }
}

fn tokens_of_events(events: &[Event<'_>]) -> Vec<CanonToken> {
    let mut tokens = Vec::with_capacity(events.len());
    for event in events {
        match event {
            Event::Open {
                name,
                attrs,
                self_closed,
                ..
            } => {
                tokens.push(open_token(name, attrs));
                if *self_closed {
                    tokens.push(CanonToken::Close {
                        name: (*name).to_owned(),
                    });
                }
            }
            Event::Close { name, .. } => tokens.push(CanonToken::Close {
                name: (*name).to_owned(),
            }),
            Event::Text { raw, .. } => push_text(&mut tokens, &decode_text(raw)),
            Event::CData { raw, .. } => push_text(&mut tokens, raw),
            Event::Other { .. } => {}
        }
    }
    tokens
}

fn atoms_of_tokens(tokens: Vec<CanonToken>) -> Vec<Atom> {
    let mut atoms = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token {
            CanonToken::Text(text) => atoms.extend(text.chars().map(Atom::Char)),
            tag => atoms.push(Atom::Tag(Box::new(tag))),
        }
    }
    atoms
}

fn push_text_atoms(atoms: &mut Vec<SourceAtom>, raw: &str, base: u32, entities: bool) {
    let bytes = raw.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let (ch, len) = match bytes[at] {
            b'\r' => (
                Some('\n'),
                1 + usize::from(bytes.get(at + 1) == Some(&b'\n')),
            ),
            0x0C => (Some(' '), 1),
            b'&' if entities => match char_ref(&raw[at..]) {
                Some(found) => (Some(found.ch), found.len),
                None => (Some('&'), 1),
            },
            _ if is_invisible_char(&bytes[at..]) => (None, if bytes[at] == 0xEF { 3 } else { 1 }),
            _ => {
                let ch = raw[at..].chars().next().expect("at is a char boundary");
                (Some(ch), ch.len_utf8())
            }
        };
        if let Some(ch) = ch {
            atoms.push(SourceAtom {
                atom: Atom::Char(fold(ch)),
                span: Span::new(base as usize + at, base as usize + at + len),
            });
        }
        at += len;
    }
}

pub(crate) fn plain_atoms(text: &str) -> Vec<Atom> {
    let mut source = Vec::new();
    push_text_atoms(&mut source, text, 0, false);
    source.into_iter().map(|atom| atom.atom).collect()
}

fn matching_close(events: &[Event<'_>], open: usize) -> Option<Span> {
    let mut depth = 0u32;
    for event in &events[open..] {
        match event {
            Event::Open {
                self_closed: false, ..
            } => depth += 1,
            Event::Close { span, .. } => {
                depth -= 1;
                if depth == 0 {
                    return Some(*span);
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) struct Traced<'e, 's> {
    pub(crate) events: &'e [Event<'s>],
    pub(crate) out: String,
    pub(crate) out_of_event: Vec<Range<u32>>,
}

impl<'e, 's> Traced<'e, 's> {
    pub(crate) fn new(
        events: &'e [Event<'s>],
        capacity: usize,
        layout: &Layout<'_>,
        context: &Context<'_>,
    ) -> Traced<'e, 's> {
        let (out, out_of_event) = trace_output(events, capacity, layout, context);
        Traced {
            events,
            out,
            out_of_event,
        }
    }

    pub(crate) fn display(&self, range: &Range<u32>) -> &str {
        let start = self.out_of_event[range.start as usize].start as usize;
        let end = self.out_of_event[range.end as usize - 1].end as usize;
        &self.out[start..end]
    }

    pub(crate) fn dom_units(&self, range: &Range<u32>) -> Result<Vec<u32>, Error> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let shown = scan(self.display(range), &DEFAULT_LIMITS)?;
        Ok(dom_units(&shown.events))
    }

    pub(crate) fn atoms(&self, range: Range<u32>) -> Result<Vec<SourceAtom>, Error> {
        let mut atoms = Vec::new();
        for ix in range.start as usize..range.end as usize {
            let written = &self.out_of_event[ix];
            if written.is_empty() {
                continue;
            }
            match &self.events[ix] {
                Event::Open {
                    span, self_closed, ..
                } => {
                    let tag = &self.out[written.start as usize..written.end as usize];
                    let shown = scan(tag, &DEFAULT_LIMITS)?;
                    let Some(Event::Open {
                        name,
                        attrs,
                        self_closed: shown_closed,
                        ..
                    }) = shown.events.first()
                    else {
                        continue;
                    };
                    let mut span = *span;
                    if *shown_closed
                        && !self_closed
                        && let Some(close) = matching_close(self.events, ix)
                    {
                        span.end = close.end;
                    }
                    atoms.push(SourceAtom {
                        atom: Atom::Tag(Box::new(open_token(name, attrs))),
                        span,
                    });
                    if *shown_closed {
                        atoms.push(SourceAtom {
                            atom: Atom::Tag(Box::new(CanonToken::Close {
                                name: (*name).to_owned(),
                            })),
                            span: Span {
                                start: span.end,
                                end: span.end,
                            },
                        });
                    }
                }
                Event::Close { span, .. } => {
                    let tag = &self.out[written.start as usize + 2..written.end as usize - 1];
                    atoms.push(SourceAtom {
                        atom: Atom::Tag(Box::new(CanonToken::Close {
                            name: tag.to_owned(),
                        })),
                        span: *span,
                    });
                }
                Event::Text { raw, span } => push_text_atoms(&mut atoms, raw, span.start, true),
                Event::CData { raw, span } => push_text_atoms(&mut atoms, raw, span.start, false),
                Event::Other { .. } => {}
            }
        }
        Ok(atoms)
    }
}

pub(crate) fn canon_html(
    xml: &str,
    context: &Context<'_>,
    limits: &Limits,
) -> Result<Vec<Atom>, Error> {
    let display = Context {
        urls: ResourceUrls {
            map: context.urls.map.clone().reading_display(),
            serveable: context.urls.serveable,
        },
        is_stylesheet: context.is_stylesheet,
    };
    let scanned = scan(xml, limits)?;
    let whole = [Block {
        span: Span::new(0, xml.len()),
        kind: BlockKind::Element,
    }];
    let everything = 0..scanned.events.len() as u32;
    let layout = Layout {
        blocks: &whole,
        ranges: std::slice::from_ref(&everything),
        body_inner: whole[0].span,
    };
    let (out, _) = trace_output(&scanned.events, xml.len(), &layout, &display);
    let clean = scan(&out, limits)?;
    Ok(atoms_of_tokens(tokens_of_events(&clean.events)))
}

fn invert_urls(value: &str, urls: &UrlMap) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find(SCHEME_PREFIX) {
        out.push_str(&rest[..at]);
        let url = &rest[at..];
        let end = url
            .find([')', '"', '\'', ' ', '\t', '\n'])
            .unwrap_or(url.len());
        out.push_str(&urls.to_source_href(&url[..end])?);
        rest = &url[end..];
    }
    out.push_str(rest);
    Some(out)
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '\r' => out.push_str("&#13;"),
        other => out.push(other),
    }
}

pub(crate) fn to_xhtml<'a>(atoms: impl IntoIterator<Item = &'a Atom>, urls: &UrlMap) -> String {
    let mut out = String::new();
    let mut atoms = atoms.into_iter().peekable();
    while let Some(atom) = atoms.next() {
        let tag = match atom {
            Atom::Char(ch) => {
                push_escaped_char(&mut out, *ch);
                continue;
            }
            Atom::Tag(tag) => tag,
        };
        match &**tag {
            CanonToken::Open { name, attrs } => {
                out.push('<');
                out.push_str(name);
                for (attr, value) in attrs {
                    let Some(value) = invert_urls(value, urls) else {
                        continue;
                    };
                    out.push(' ');
                    out.push_str(attr);
                    out.push_str("=\"");
                    push_escaped_attr(&mut out, &value);
                    out.push('"');
                }
                let closes_next = matches!(atoms.peek(), Some(Atom::Tag(next))
                    if matches!(&**next, CanonToken::Close { name: closing } if closing == name));
                if closes_next && is_void(local_name(name)) {
                    atoms.next();
                    out.push_str("/>");
                } else {
                    out.push('>');
                }
            }
            CanonToken::Close { name } => {
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            CanonToken::Text(text) => text.chars().for_each(|ch| push_escaped_char(&mut out, ch)),
        }
    }
    out
}

fn block_ranges(events: &[Event<'_>]) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut at = 0;
    while at < events.len() {
        let is_block = matches!(&events[at], Event::Open { attrs, .. }
            if attrs.iter().any(|attr| attr.name == "data-ss-b"));
        if !is_block {
            at += 1;
            continue;
        }
        let start = at;
        let mut depth = 0u32;
        while at < events.len() {
            match &events[at] {
                Event::Open {
                    self_closed: false, ..
                } => depth += 1,
                Event::Close { .. } => depth -= 1,
                _ => {}
            }
            at += 1;
            if depth == 0 {
                break;
            }
        }
        ranges.push(start..at);
    }
    ranges
}

fn dom_units(events: &[Event<'_>]) -> Vec<u32> {
    let mut units = Vec::new();
    let mut skipping = 0u32;
    for event in events {
        match event {
            Event::Open {
                name, self_closed, ..
            } => {
                let local = local_name(name);
                if skipping > 0 || class_of(local) == ElementClass::Replaced {
                    skipping += u32::from(!self_closed);
                } else if local == "br" {
                    units.push(BREAK_UNIT);
                }
            }
            Event::Close { .. } => skipping = skipping.saturating_sub(1),
            Event::Text { raw, .. } if skipping == 0 => {
                units.extend(decode_text(raw).encode_utf16().map(u32::from));
            }
            Event::CData { raw, .. } if skipping == 0 => {
                units.extend(raw.encode_utf16().map(u32::from));
            }
            _ => {}
        }
    }
    units
}

#[doc(hidden)]
pub mod proof {
    use super::*;
    use std::collections::BTreeMap;

    fn scanned<T>(xml: &str, read: impl FnOnce(&[Event<'_>]) -> T) -> Result<T, Error> {
        Ok(read(&scan(xml, &DEFAULT_LIMITS)?.events))
    }

    fn compared(tokens: Vec<CanonToken>) -> Vec<CanonToken> {
        tokens
            .into_iter()
            .map(|token| match token {
                CanonToken::Open { name, attrs } => {
                    let mut attrs: Vec<(String, String)> = attrs
                        .into_iter()
                        .map(|(name, value)| (name.to_ascii_lowercase(), without_session(value)))
                        .collect();
                    attrs.sort();
                    CanonToken::Open {
                        name: name.to_ascii_lowercase(),
                        attrs,
                    }
                }
                CanonToken::Close { name } => CanonToken::Close {
                    name: name.to_ascii_lowercase(),
                },
                text => text,
            })
            .collect()
    }

    pub fn tokens_of(xml: &str) -> Result<Vec<CanonToken>, Error> {
        scanned(xml, |events| compared(tokens_of_events(events)))
    }

    pub fn block_tokens_of(xhtml: &str) -> Result<Vec<Vec<CanonToken>>, Error> {
        scanned(xhtml, |events| {
            block_ranges(events)
                .into_iter()
                .map(|range| compared(tokens_of_events(&events[range])))
                .collect()
        })
    }

    pub fn block_units_of(xhtml: &str) -> Result<Vec<Vec<u32>>, Error> {
        scanned(xhtml, |events| {
            block_ranges(events)
                .into_iter()
                .map(|range| dom_units(&events[range]))
                .collect()
        })
    }

    pub fn visible_text_of(raw: Vec<u8>) -> Result<crate::VisibleText, Error> {
        Ok(crate::chapter::decode_and_model(raw, &DEFAULT_LIMITS)?.text)
    }

    pub fn damage_of(raw: Vec<u8>) -> Result<(BTreeMap<String, u32>, bool), Error> {
        let model = crate::chapter::decode_and_model(raw, &DEFAULT_LIMITS)?;
        let mut kinds = BTreeMap::new();
        for fix in &model.fixes.point {
            let kind = match fix.text.as_str() {
                "/" => "void close added",
                " " => "attribute space added",
                "" => "invisible removed",
                "&amp;" => "bare ampersand",
                "&lt;" => "bare less-than",
                "&gt;" => "cdata end escaped",
                "UTF-8" => "declaration",
                text if text.starts_with("&#") => "named entity made numeric",
                _ => "other",
            };
            *kinds.entry(kind.to_owned()).or_default() += 1;
        }
        Ok((kinds, model.fixes.structural))
    }

    pub fn decoded_of(raw: Vec<u8>) -> String {
        crate::chapter::decode_source(raw).0
    }

    pub fn block_xml_of(xhtml: &str) -> Result<Vec<String>, Error> {
        scanned(xhtml, |events| {
            block_ranges(events)
                .into_iter()
                .map(|range| {
                    let start = event_span(&events[range.start]).start as usize;
                    let end = event_span(&events[range.end - 1]).end as usize;
                    xhtml[start..end].to_owned()
                })
                .collect()
        })
    }

    fn event_span(event: &Event<'_>) -> Span {
        match event {
            Event::Open { span, .. }
            | Event::Close { span, .. }
            | Event::Text { span, .. }
            | Event::CData { span, .. }
            | Event::Other { span } => *span,
        }
    }

    pub use super::CanonToken;
    pub use crate::chapter::BREAK_UNIT;
}

#[cfg(test)]
mod tests {
    use super::CanonToken::{Close, Open, Text};
    use super::proof::{block_tokens_of, block_units_of, tokens_of};

    #[test]
    fn serializer_noise_does_not_change_tokens() {
        let source = "<p data-ss-b=\"1\" class=\"a\" id=\"b\">x&#160;y<br/></p>";
        let serialized = "<p xmlns=\"http://www.w3.org/1999/xhtml\" id=\"b\" class=\"a\" data-ss-b=\"1\">x y<br></br></p>";
        assert_eq!(tokens_of(source).unwrap(), tokens_of(serialized).unwrap());
    }

    #[test]
    fn a_changed_attribute_or_word_changes_tokens() {
        let base = tokens_of("<p class=\"a\">word</p>").unwrap();
        assert_ne!(base, tokens_of("<p class=\"b\">word</p>").unwrap());
        assert_ne!(base, tokens_of("<p class=\"a\">wurd</p>").unwrap());
    }

    #[test]
    fn book_session_hosts_compare_equal() {
        let left = "<img src=\"jadeslip-book://0123456789abcdef/OEBPS/a.png\"/>";
        let right = "<img src=\"jadeslip-book://fedcba9876543210/OEBPS/a.png\"/>";
        assert_eq!(tokens_of(left).unwrap(), tokens_of(right).unwrap());
    }

    #[test]
    fn blocks_split_at_their_markers_and_count_units_like_the_walker() {
        let xhtml = "<div><div data-ss-w=\"0\"><p data-ss-b=\"0\">a<br/>b<img src=\"x\" alt=\"t\"/></p></div><p data-ss-b=\"1\"><b>c</b></p></div>";
        let blocks = block_tokens_of(xhtml).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[1],
            vec![
                Open {
                    name: "p".into(),
                    attrs: vec![]
                },
                Open {
                    name: "b".into(),
                    attrs: vec![]
                },
                Text("c".into()),
                Close { name: "b".into() },
                Close { name: "p".into() },
            ]
        );
        let units = block_units_of(xhtml).unwrap();
        assert_eq!(units[0], vec![97, super::BREAK_UNIT, 98]);
        assert_eq!(units[1], vec![99]);
    }
}
