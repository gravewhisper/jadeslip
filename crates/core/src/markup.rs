use crate::entities::NAMED_ENTITIES;
use crate::{Error, Limits};
use memchr::{memchr, memmem};
use std::borrow::Cow;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Span {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl Span {
    pub(crate) fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    pub(crate) fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct SourceEdit {
    pub(crate) span: Span,
    pub(crate) text: String,
}

pub(crate) fn splice(source: &str, mut edits: Vec<SourceEdit>) -> String {
    edits.sort_by_key(|edit| {
        (
            edit.span.start,
            edit.span.end != edit.span.start,
            std::cmp::Reverse(edit.span.end),
        )
    });
    let added: usize = edits.iter().map(|edit| edit.text.len()).sum();
    let mut out = String::with_capacity(source.len() + added);
    let mut cursor = 0;
    for edit in &edits {
        let range = edit.span.range();
        if range.start < cursor {
            continue;
        }
        out.push_str(&source[cursor..range.start]);
        out.push_str(&edit.text);
        cursor = range.end;
    }
    out.push_str(&source[cursor..]);
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ElementClass {
    Container,
    Block,
    Replaced,
    Inline,
    NonRendered,
    Forbidden,
    Unknown,
}

const LONGEST_KNOWN_NAME: usize = 12;

pub(crate) fn class_of(local_name: &str) -> ElementClass {
    let mut lowered = [0u8; LONGEST_KNOWN_NAME];
    let Some(buffer) = lowered.get_mut(..local_name.len()) else {
        return ElementClass::Unknown;
    };
    buffer.copy_from_slice(local_name.as_bytes());
    buffer.make_ascii_lowercase();
    match &*buffer {
        b"html" | b"body" | b"div" | b"section" | b"article" | b"aside" | b"nav" | b"header"
        | b"footer" | b"main" | b"hgroup" => ElementClass::Container,
        b"p" | b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" | b"ul" | b"ol" | b"li" | b"dl"
        | b"dt" | b"dd" | b"dir" | b"menu" | b"table" | b"caption" | b"thead" | b"tbody"
        | b"tfoot" | b"tr" | b"td" | b"th" | b"colgroup" | b"col" | b"pre" | b"blockquote"
        | b"address" | b"center" | b"figure" | b"figcaption" | b"details" | b"summary" | b"hr" => {
            ElementClass::Block
        }
        b"img" | b"image" | b"svg" | b"math" | b"video" | b"audio" | b"canvas" | b"picture" => {
            ElementClass::Replaced
        }
        b"a" | b"abbr" | b"acronym" | b"b" | b"bdi" | b"bdo" | b"big" | b"br" | b"cite"
        | b"code" | b"data" | b"del" | b"dfn" | b"em" | b"font" | b"i" | b"ins" | b"kbd"
        | b"label" | b"mark" | b"nobr" | b"q" | b"rb" | b"rp" | b"rt" | b"rtc" | b"ruby" | b"s"
        | b"samp" | b"small" | b"span" | b"strike" | b"strong" | b"sub" | b"sup" | b"time"
        | b"tt" | b"u" | b"var" | b"wbr" => ElementClass::Inline,
        b"script" | b"style" | b"head" | b"template" | b"title" | b"meta" | b"link" | b"base"
        | b"noscript" | b"source" | b"track" | b"param" => ElementClass::NonRendered,
        b"form" | b"iframe" | b"object" | b"embed" | b"applet" | b"frame" | b"frameset"
        | b"noframes" | b"input" | b"button" | b"select" | b"option" | b"optgroup"
        | b"textarea" | b"datalist" | b"dialog" | b"fieldset" | b"legend" | b"map" | b"area"
        | b"marquee" | b"xi:include" => ElementClass::Forbidden,
        _ => ElementClass::Unknown,
    }
}

pub(crate) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub(crate) fn is_void(local: &str) -> bool {
    const VOID: &[&str] = &[
        "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
        "isindex", "keygen", "link", "meta", "param", "source", "track", "wbr",
    ];
    VOID.iter().any(|name| name.eq_ignore_ascii_case(local))
}

fn in_scope_of_list(local: &str) -> Option<&'static [&'static str]> {
    const ITEM_LISTS: &[&str] = &["ul", "ol", "dir", "menu"];
    const DEF_LISTS: &[&str] = &["dl"];
    const ROWS: &[&str] = &["tr", "table"];
    const TABLES: &[&str] = &["table"];
    const SELECTS: &[&str] = &["select", "datalist"];
    let lowered = local.to_ascii_lowercase();
    match lowered.as_str() {
        "li" => Some(ITEM_LISTS),
        "dt" | "dd" => Some(DEF_LISTS),
        "td" | "th" => Some(ROWS),
        "tr" | "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => Some(TABLES),
        "option" | "optgroup" => Some(SELECTS),
        _ => None,
    }
}

fn implied_siblings(local: &str) -> &'static [&'static str] {
    let lowered = local.to_ascii_lowercase();
    match lowered.as_str() {
        "li" => &["li"],
        "dt" | "dd" => &["dt", "dd"],
        "td" | "th" => &["td", "th"],
        "tr" => &["tr", "td", "th"],
        "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => &[
            "thead", "tbody", "tfoot", "caption", "colgroup", "tr", "td", "th",
        ],
        "option" => &["option"],
        "optgroup" => &["option", "optgroup"],
        _ => &[],
    }
}

fn closes_open_paragraph(local: &str) -> bool {
    matches!(
        class_of(local),
        ElementClass::Container | ElementClass::Block
    ) && ![
        "html", "body", "td", "th", "tr", "thead", "tbody", "tfoot", "caption", "col", "colgroup",
        "dt", "dd",
    ]
    .iter()
    .any(|name| name.eq_ignore_ascii_case(local))
}

fn is_raw_text(local: &str) -> bool {
    local.eq_ignore_ascii_case("script") || local.eq_ignore_ascii_case("style")
}

pub(crate) enum Event<'s> {
    Open {
        name: &'s str,
        attrs: Vec<Attr<'s>>,
        span: Span,
        self_closed: bool,
    },
    Close {
        name: &'s str,
        span: Span,
    },
    Text {
        raw: &'s str,
        span: Span,
    },
    CData {
        raw: &'s str,
        span: Span,
    },
    Other {
        span: Span,
    },
}

pub(crate) struct Attr<'s> {
    pub(crate) name: &'s str,
    pub(crate) raw_value: &'s str,
    pub(crate) value_span: Span,
}

pub(crate) fn attr_value(attrs: &[Attr<'_>], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| local_name(attr.name).eq_ignore_ascii_case(name))
        .map(|attr| decode_attr(attr.raw_value))
}

pub(crate) struct Scanned<'s> {
    pub(crate) events: Vec<Event<'s>>,
    pub(crate) fixes: Fixes,
}

#[derive(Default)]
pub(crate) struct Fixes {
    pub(crate) point: Vec<SourceEdit>,
    pub(crate) structural: bool,
}

pub(crate) struct CharRef {
    pub(crate) ch: char,
    pub(crate) len: usize,
    needs_numeric_form: bool,
}

const fn is_xml_char(code: u32) -> bool {
    matches!(code, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

pub(crate) fn char_ref(text: &str) -> Option<CharRef> {
    let bytes = text.as_bytes();
    if bytes.get(1) == Some(&b'#') {
        let (digits_at, radix) = if bytes.get(2) == Some(&b'x') {
            (3, 16)
        } else {
            (2, 10)
        };
        let digits = bytes[digits_at..]
            .iter()
            .take_while(|byte| (*byte).is_ascii_alphanumeric())
            .count();
        if digits == 0 || bytes.get(digits_at + digits) != Some(&b';') {
            return None;
        }
        let code = u32::from_str_radix(&text[digits_at..digits_at + digits], radix).ok()?;
        if !is_xml_char(code) {
            return None;
        }
        return Some(CharRef {
            ch: char::from_u32(code)?,
            len: digits_at + digits + 1,
            needs_numeric_form: false,
        });
    }
    const LONGEST_NAME: usize = 8;
    let window = &bytes[1..bytes.len().min(LONGEST_NAME + 2)];
    let name_len = memchr(b';', window)?;
    let name = text.get(1..1 + name_len)?;
    let at = NAMED_ENTITIES
        .binary_search_by_key(&name, |(entity, _)| entity)
        .ok()?;
    Some(CharRef {
        ch: char::from_u32(NAMED_ENTITIES[at].1)?,
        len: name_len + 2,
        needs_numeric_form: !matches!(name, "amp" | "lt" | "gt" | "quot" | "apos"),
    })
}

pub(crate) fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

pub(crate) fn decode_text(raw: &str) -> Cow<'_, str> {
    if !raw.contains(['&', '\r']) {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find(['&', '\r']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if rest.starts_with('\r') {
            out.push('\n');
            rest = rest.strip_prefix("\r\n").unwrap_or(&rest[1..]);
        } else if let Some(reference) = char_ref(rest) {
            out.push(reference.ch);
            rest = &rest[reference.len..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

pub(crate) fn decode_attr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find(['&', '\t', '\n', '\r']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some(tail) = rest.strip_prefix("\r\n") {
            out.push(' ');
            rest = tail;
        } else if rest.starts_with('&') {
            if let Some(reference) = char_ref(rest) {
                out.push(reference.ch);
                rest = &rest[reference.len..];
            } else {
                out.push('&');
                rest = &rest[1..];
            }
        } else {
            out.push(' ');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':') || byte >= 0x80
}

pub(crate) fn is_invisible_char(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        [0x00..=0x08 | 0x0B | 0x0E..=0x1F, ..] | [0xEF, 0xBF, 0xBE | 0xBF, ..]
    )
}

fn skip_invisible(bytes: &[u8], mut at: usize) -> usize {
    while is_invisible_char(&bytes[at..]) {
        at += if bytes[at] == 0xEF { 3 } else { 1 };
    }
    at
}

fn cdata_end_after_removals(bytes: &[u8]) -> Option<usize> {
    let second = skip_invisible(bytes, 1);
    if bytes.get(second) != Some(&b']') {
        return None;
    }
    let close = skip_invisible(bytes, second + 1);
    (bytes.get(close) == Some(&b'>')).then_some(close)
}

fn is_xml_name_start(ch: char) -> bool {
    matches!(ch,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FEFE}' | '\u{10000}'..='\u{EFFFF}')
}

fn is_xml_name_char(ch: char) -> bool {
    is_xml_name_start(ch)
        || matches!(ch, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_xml_name_start) && chars.all(is_xml_name_char)
}

fn is_valid_xml_declaration(pseudo_attrs: &str) -> bool {
    let mut rest = pseudo_attrs;
    let mut expected = ["version", "encoding", "standalone"].into_iter();
    let mut saw_version = false;
    loop {
        let trimmed = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if trimmed.is_empty() {
            return saw_version;
        }
        if trimmed.len() == rest.len() {
            return false;
        }
        let Some((name, after)) = trimmed.split_once('=') else {
            return false;
        };
        let name = name.trim_end_matches(|ch: char| ch.is_ascii_whitespace());
        if !expected.any(|known| known == name) || (!saw_version && name != "version") {
            return false;
        }
        saw_version = true;
        let after = after.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let Some(quote) = after.chars().next().filter(|ch| matches!(ch, '"' | '\'')) else {
            return false;
        };
        let Some((value, tail)) = after[1..].split_once(quote) else {
            return false;
        };
        let valid = match name {
            "version" => value.strip_prefix("1.").is_some_and(|minor| {
                !minor.is_empty() && minor.bytes().all(|byte| byte.is_ascii_digit())
            }),
            "encoding" => {
                value.starts_with(|ch: char| ch.is_ascii_alphabetic())
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
            }
            _ => matches!(value, "yes" | "no"),
        };
        if !valid {
            return false;
        }
        rest = tail;
    }
}

fn is_valid_doctype(inner: &str) -> bool {
    let is_xml_space = |ch: char| matches!(ch, ' ' | '\t' | '\n' | '\r');
    let quoted = |text: &'_ str| -> Option<(usize, usize)> {
        let quote = text.chars().next().filter(|ch| matches!(ch, '"' | '\''))?;
        let end = text[1..].find(quote)?;
        let clean = !text[1..1 + end].chars().any(char::is_control);
        clean.then_some((end, end + 2))
    };
    let trimmed = inner.trim_matches(is_xml_space);
    let name_end = trimmed.find(is_xml_space).unwrap_or(trimmed.len());
    if !is_valid_name(&trimmed[..name_end]) {
        return false;
    }
    let external = trimmed[name_end..].trim_start_matches(is_xml_space);
    let literals = if let Some(tail) = external.strip_prefix("SYSTEM") {
        Some((tail, 1))
    } else {
        external.strip_prefix("PUBLIC").map(|tail| (tail, 2))
    };
    let Some((mut tail, count)) = literals else {
        return external.is_empty();
    };
    for _ in 0..count {
        let spaced = tail.trim_start_matches(is_xml_space);
        if spaced.len() == tail.len() {
            return false;
        }
        let Some((_, consumed)) = quoted(spaced) else {
            return false;
        };
        tail = &spaced[consumed..];
    }
    tail.is_empty()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Strict,
    Repair,
}

enum Stop {
    NotWellNested,
    Failed(Error),
}

impl From<Error> for Stop {
    fn from(error: Error) -> Self {
        Stop::Failed(error)
    }
}

struct Scanner<'s, 'l> {
    src: &'s str,
    bytes: &'s [u8],
    pos: usize,
    limits: &'l Limits,
    mode: Mode,
    events: Vec<Event<'s>>,
    stack: Vec<&'s str>,
    fixes: Fixes,
    root_seen: bool,
}

pub(crate) fn scan<'s>(source: &'s str, limits: &Limits) -> Result<Scanned<'s>, Error> {
    if u32::try_from(source.len()).is_err() {
        return Err(Error::Limit(
            "This document is too large to read.".to_owned(),
        ));
    }
    match Scanner::new(source, limits, Mode::Strict).run() {
        Ok(scanned) => Ok(scanned),
        Err(Stop::Failed(error)) => Err(error),
        Err(Stop::NotWellNested) => match Scanner::new(source, limits, Mode::Repair).run() {
            Ok(scanned) => Ok(scanned),
            Err(Stop::Failed(error)) => Err(error),
            Err(Stop::NotWellNested) => unreachable!("repair mode closes everything itself"),
        },
    }
}

impl<'s, 'l> Scanner<'s, 'l> {
    fn new(src: &'s str, limits: &'l Limits, mode: Mode) -> Self {
        Scanner {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            limits,
            mode,
            events: Vec::with_capacity(src.len() / 128 + 8),
            stack: Vec::new(),
            fixes: Fixes::default(),
            root_seen: false,
        }
    }

    fn run(mut self) -> Result<Scanned<'s>, Stop> {
        while self.pos < self.bytes.len() {
            self.step()?;
            if self.events.len() > self.limits.max_markup_events as usize {
                return Err(Stop::Failed(Error::Limit(format!(
                    "This document has more than the limit of {} pieces of markup.",
                    self.limits.max_markup_events
                ))));
            }
        }
        if !self.stack.is_empty() {
            if self.mode == Mode::Strict {
                return Err(Stop::NotWellNested);
            }
            self.close_down_to(0, self.bytes.len());
        }
        if !self.root_seen {
            self.fixes.structural = true;
        }
        Ok(Scanned {
            events: self.events,
            fixes: self.fixes,
        })
    }

    fn step(&mut self) -> Result<(), Stop> {
        let text_end = self.next_markup();
        if text_end > self.pos {
            self.text(self.pos, text_end);
            self.pos = text_end;
            return Ok(());
        }
        let rest = &self.bytes[self.pos..];
        if rest.starts_with(b"<!--") {
            self.comment();
        } else if rest.starts_with(b"<![CDATA[") {
            self.cdata();
        } else if rest.starts_with(b"<!") {
            self.declaration();
        } else if rest.starts_with(b"<?") {
            self.instruction();
        } else if rest.starts_with(b"</") {
            self.close_tag()?;
        } else {
            self.open_tag()?;
        }
        Ok(())
    }

    fn next_markup(&self) -> usize {
        let mut at = self.pos;
        while let Some(found) = memchr(b'<', &self.bytes[at..]) {
            at += found;
            match self.bytes.get(at + 1) {
                Some(b'!' | b'?') => return at,
                Some(b'/') if self.bytes.get(at + 2).copied().is_some_and(is_name_start) => {
                    return at;
                }
                Some(&byte) if is_name_start(byte) => return at,
                _ => at += 1,
            }
        }
        self.bytes.len()
    }

    fn edit(&mut self, start: usize, end: usize, text: impl Into<String>) {
        self.fixes.point.push(SourceEdit {
            span: Span::new(start, end),
            text: text.into(),
        });
    }

    fn text(&mut self, start: usize, end: usize) {
        self.check_chars(start, end, false);
        if self.stack.is_empty() && !self.bytes[start..end].iter().all(|&byte| is_space(byte)) {
            self.fixes.structural = true;
        }
        self.events.push(Event::Text {
            raw: &self.src[start..end],
            span: Span::new(start, end),
        });
    }

    fn check_chars(&mut self, start: usize, end: usize, in_attr: bool) {
        let mut at = start;
        while at < end {
            match self.bytes[at] {
                b'&' => {
                    let reference = char_ref(&self.src[at..end]);
                    match reference {
                        Some(found) => {
                            if found.needs_numeric_form {
                                self.edit(at, at + found.len, format!("&#{};", found.ch as u32));
                            }
                            at += found.len;
                        }
                        None => {
                            self.edit(at, at + 1, "&amp;");
                            at += 1;
                        }
                    }
                }
                b'<' => {
                    self.edit(at, at + 1, "&lt;");
                    at += 1;
                }
                b']' if !in_attr => {
                    if let Some(close) = cdata_end_after_removals(&self.bytes[at..end]) {
                        self.edit(at + close, at + close + 1, "&gt;");
                    }
                    at += 1;
                }
                0x0C => {
                    self.edit(at, at + 1, " ");
                    at += 1;
                }
                0x00..=0x08 | 0x0B | 0x0E..=0x1F => {
                    self.edit(at, at + 1, "");
                    at += 1;
                }
                0xEF if matches!(self.bytes[at..end], [0xEF, 0xBF, 0xBE | 0xBF, ..]) => {
                    self.edit(at, at + 3, "");
                    at += 3;
                }
                _ => at += 1,
            }
        }
    }

    fn rest_is_other(&mut self) {
        self.fixes.structural = true;
        self.events.push(Event::Other {
            span: Span::new(self.pos, self.bytes.len()),
        });
        self.pos = self.bytes.len();
    }

    fn other_until(&mut self, end: usize) {
        self.events.push(Event::Other {
            span: Span::new(self.pos, end),
        });
        self.pos = end;
    }

    fn comment(&mut self) {
        let body = self.pos + 4;
        let Some(found) = memmem::find(&self.bytes[body..], b"-->") else {
            self.rest_is_other();
            return;
        };
        if memmem::find(&self.bytes[body..body + found], b"--").is_some()
            || self.bytes[body + found - 1.min(found)..body + found] == *b"-"
        {
            self.fixes.structural = true;
        }
        self.check_markup_chars(body, body + found);
        self.other_until(body + found + 3);
    }

    fn check_markup_chars(&mut self, start: usize, end: usize) {
        let bad = self.bytes[start..end]
            .iter()
            .any(|&byte| byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r'))
            || memmem::find(&self.bytes[start..end], b"\xEF\xBF\xBE").is_some()
            || memmem::find(&self.bytes[start..end], b"\xEF\xBF\xBF").is_some();
        if bad {
            self.fixes.structural = true;
        }
    }

    fn cdata(&mut self) {
        let body = self.pos + 9;
        let Some(found) = memmem::find(&self.bytes[body..], b"]]>") else {
            self.rest_is_other();
            return;
        };
        if self.stack.is_empty() {
            self.fixes.structural = true;
        }
        self.check_markup_chars(body, body + found);
        self.events.push(Event::CData {
            raw: &self.src[body..body + found],
            span: Span::new(body, body + found),
        });
        self.pos = body + found + 3;
    }

    fn declaration(&mut self) {
        let is_doctype = self.bytes[self.pos..].len() >= 9
            && self.bytes[self.pos + 2..self.pos + 9].eq_ignore_ascii_case(b"DOCTYPE");
        let space_follows = self
            .bytes
            .get(self.pos + 9)
            .is_some_and(|&byte| is_space(byte) && byte != 0x0C);
        if !is_doctype
            || !space_follows
            || !self.bytes[self.pos..].starts_with(b"<!DOCTYPE")
            || self.root_seen
        {
            self.fixes.structural = true;
        }
        let mut at = self.pos + 2;
        let mut quote = None;
        let mut in_subset = false;
        while let Some(&byte) = self.bytes.get(at) {
            if in_subset && quote.is_none() && byte == b'<' {
                let skipped = if self.bytes[at..].starts_with(b"<!--") {
                    memmem::find(&self.bytes[at..], b"-->").map(|found| found + 3)
                } else if self.bytes[at..].starts_with(b"<?") {
                    memmem::find(&self.bytes[at..], b"?>").map(|found| found + 2)
                } else {
                    Some(1)
                };
                let Some(skipped) = skipped else {
                    break;
                };
                at += skipped;
                continue;
            }
            match (quote, byte) {
                (Some(open), _) if open == byte => quote = None,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(byte),
                (None, b'[') => {
                    in_subset = true;
                    self.fixes.structural = true;
                }
                (None, b']') => in_subset = false,
                (None, b'>') if !in_subset => {
                    if is_doctype
                        && !self.fixes.structural
                        && !is_valid_doctype(&self.src[self.pos + 9..at])
                    {
                        self.fixes.structural = true;
                    }
                    self.other_until(at + 1);
                    return;
                }
                _ => {}
            }
            at += 1;
        }
        self.rest_is_other();
    }

    fn instruction(&mut self) {
        let body = self.pos + 2;
        let Some(found) = memmem::find(&self.bytes[body..], b"?>") else {
            self.rest_is_other();
            return;
        };
        let target_len = self.bytes[body..body + found]
            .iter()
            .take_while(|&&byte| !is_space(byte))
            .count();
        let target = &self.src[body..body + target_len];
        let bad_declaration = target.eq_ignore_ascii_case("xml")
            && (self.pos != 0
                || target != "xml"
                || !is_valid_xml_declaration(&self.src[body + target_len..body + found]));
        if bad_declaration || !is_valid_name(target) {
            self.fixes.structural = true;
        }
        self.check_markup_chars(body, body + found);
        self.other_until(body + found + 2);
    }

    fn close_tag(&mut self) -> Result<(), Stop> {
        let start = self.pos;
        let name_start = start + 2;
        let name_len = self.bytes[name_start..]
            .iter()
            .take_while(|&&byte| !is_space(byte) && byte != b'>')
            .count();
        let name = &self.src[name_start..name_start + name_len];
        let Some(close) = memchr(b'>', &self.bytes[name_start + name_len..]) else {
            return self.close_tag_at_eof();
        };
        let end = name_start + name_len + close + 1;
        if end - start > self.limits.max_tag_bytes as usize {
            return Err(self.tag_too_long());
        }
        let tail = &self.bytes[name_start + name_len..end - 1];
        if !tail.iter().all(|&byte| is_space(byte)) || tail.contains(&0x0C) || !is_valid_name(name)
        {
            self.fixes.structural = true;
        }
        match self.mode {
            Mode::Strict => {
                if self.stack.last() != Some(&name) {
                    return Err(Stop::NotWellNested);
                }
                self.stack.pop();
                self.events.push(Event::Close {
                    name,
                    span: Span::new(start, end),
                });
            }
            Mode::Repair => {
                let found = self
                    .stack
                    .iter()
                    .rposition(|open| open.eq_ignore_ascii_case(name));
                match found {
                    Some(depth) => {
                        let opened = self.stack[depth];
                        self.close_down_to(depth + 1, start);
                        if opened != name {
                            self.edit(name_start, name_start + name_len, opened);
                        }
                        self.stack.pop();
                        self.events.push(Event::Close {
                            name: opened,
                            span: Span::new(start, end),
                        });
                    }
                    None => {
                        self.fixes.structural = true;
                        self.events.push(Event::Other {
                            span: Span::new(start, end),
                        });
                    }
                }
            }
        }
        self.pos = end;
        Ok(())
    }

    fn close_tag_at_eof(&mut self) -> Result<(), Stop> {
        if self.mode == Mode::Strict {
            return Err(Stop::NotWellNested);
        }
        self.rest_is_other();
        Ok(())
    }

    fn close_down_to(&mut self, keep: usize, at: usize) {
        while self.stack.len() > keep {
            let name = self.stack.pop().expect("stack is longer than keep");
            self.fixes.structural = true;
            self.events.push(Event::Close {
                name,
                span: Span::new(at, at),
            });
        }
    }

    fn tag_too_long(&self) -> Stop {
        Stop::Failed(Error::Limit(format!(
            "A tag in this document is longer than the limit of {} bytes.",
            self.limits.max_tag_bytes
        )))
    }

    fn open_tag(&mut self) -> Result<(), Stop> {
        let start = self.pos;
        let name_start = start + 1;
        let name_len = self.bytes[name_start..]
            .iter()
            .take_while(|&&byte| !is_space(byte) && byte != b'/' && byte != b'>')
            .count();
        let name = &self.src[name_start..name_start + name_len];
        if !is_valid_name(name) {
            self.fixes.structural = true;
        }
        let Some(tag) = self.parse_attrs(start, name_start + name_len)? else {
            if self.mode == Mode::Strict {
                return Err(Stop::NotWellNested);
            }
            self.rest_is_other();
            return Ok(());
        };
        if memchr(0x0C, &self.bytes[start..tag.end]).is_some() {
            self.fixes.structural = true;
        }
        if self.mode == Mode::Repair {
            self.imply_closes(name, start);
        }
        if self.stack.is_empty() {
            if self.root_seen {
                self.fixes.structural = true;
            }
            self.root_seen = true;
        }
        let local = local_name(name);
        let mut self_closed = tag.self_closed;
        if self.mode == Mode::Repair
            && !self_closed
            && is_void(local)
            && !self.closes_immediately(tag.end, name)
        {
            self_closed = true;
            self.edit(tag.end - 1, tag.end - 1, "/");
        }
        self.events.push(Event::Open {
            name,
            attrs: tag.attrs,
            span: Span::new(start, tag.end),
            self_closed,
        });
        self.pos = tag.end;
        if self_closed {
            return Ok(());
        }
        if self.stack.len() >= self.limits.max_xml_depth as usize {
            return Err(match self.mode {
                Mode::Strict => Stop::NotWellNested,
                Mode::Repair => Stop::Failed(Error::Limit(format!(
                    "This document nests elements deeper than the limit of {}.",
                    self.limits.max_xml_depth
                ))),
            });
        }
        self.stack.push(name);
        if self.mode == Mode::Repair && is_raw_text(local) {
            self.raw_text(name);
        }
        Ok(())
    }

    fn closes_immediately(&self, at: usize, name: &str) -> bool {
        let rest = &self.bytes[at..];
        rest.starts_with(b"</")
            && rest
                .get(2..2 + name.len())
                .is_some_and(|found| found.eq_ignore_ascii_case(name.as_bytes()))
            && rest.get(2 + name.len()) == Some(&b'>')
    }

    fn imply_closes(&mut self, name: &str, at: usize) {
        let local = local_name(name);
        if let Some(scope) = in_scope_of_list(local) {
            let siblings = implied_siblings(local);
            let boundary = self
                .stack
                .iter()
                .rposition(|open| {
                    scope
                        .iter()
                        .any(|stop| stop.eq_ignore_ascii_case(local_name(open)))
                })
                .map_or(0, |depth| depth + 1);
            let sibling = self.stack[boundary..].iter().position(|open| {
                siblings
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(local_name(open)))
            });
            if let Some(found) = sibling {
                self.close_down_to(boundary + found, at);
            }
        }
        if closes_open_paragraph(local) {
            let paragraph = self.stack.iter().rposition(|open| {
                let open = local_name(open);
                open.eq_ignore_ascii_case("p")
                    || !matches!(class_of(open), ElementClass::Inline | ElementClass::Unknown)
            });
            if let Some(depth) =
                paragraph.filter(|&depth| local_name(self.stack[depth]).eq_ignore_ascii_case("p"))
            {
                self.close_down_to(depth, at);
            }
        }
    }

    fn raw_text(&mut self, name: &str) {
        let start = self.pos;
        let mut at = start;
        let end = loop {
            let Some(found) = memmem::find(&self.bytes[at..], b"</") else {
                break self.bytes.len();
            };
            at += found;
            let closes = self.bytes[at + 2..]
                .get(..name.len())
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes()));
            if closes {
                break at;
            }
            at += 2;
        };
        if end > start {
            let content = &self.bytes[start..end];
            if content.contains(&b'<')
                || content.contains(&b'&')
                || memmem::find(content, b"]]>").is_some()
            {
                self.fixes.structural = true;
            }
            self.check_markup_chars(start, end);
            self.events.push(Event::Text {
                raw: &self.src[start..end],
                span: Span::new(start, end),
            });
        }
        self.pos = end;
    }

    fn parse_attrs(&mut self, start: usize, mut at: usize) -> Result<Option<ParsedTag<'s>>, Stop> {
        let mut attrs: Vec<Attr<'s>> = Vec::new();
        loop {
            let space_start = at;
            while self.bytes.get(at).copied().is_some_and(is_space) {
                at += 1;
            }
            let had_space = at > space_start;
            if at - start > self.limits.max_tag_bytes as usize {
                return Err(self.tag_too_long());
            }
            match self.bytes.get(at) {
                None => return Ok(None),
                Some(b'>') => {
                    return Ok(Some(ParsedTag {
                        attrs,
                        end: at + 1,
                        self_closed: false,
                    }));
                }
                Some(b'/') => {
                    if self.bytes.get(at + 1) == Some(&b'>') {
                        return Ok(Some(ParsedTag {
                            attrs,
                            end: at + 2,
                            self_closed: true,
                        }));
                    }
                    self.fixes.structural = true;
                    at += 1;
                    continue;
                }
                Some(b'=') => {
                    self.fixes.structural = true;
                    at += 1;
                    continue;
                }
                Some(_) => {}
            }
            if !had_space {
                self.edit(at, at, " ");
            }
            let name_start = at;
            while let Some(&byte) = self.bytes.get(at) {
                if is_space(byte) || matches!(byte, b'=' | b'/' | b'>') {
                    break;
                }
                at += 1;
            }
            let name = &self.src[name_start..at];
            if !is_valid_name(name) || attrs.iter().any(|attr| attr.name == name) {
                self.fixes.structural = true;
            }
            let name_end = at;
            while self.bytes.get(at).copied().is_some_and(is_space) {
                at += 1;
            }
            let (raw_value, value_span);
            if self.bytes.get(at) == Some(&b'=') {
                at += 1;
                while self.bytes.get(at).copied().is_some_and(is_space) {
                    at += 1;
                }
                match self.bytes.get(at) {
                    None => return Ok(None),
                    Some(&quote @ (b'"' | b'\'')) => {
                        let body = at + 1;
                        let Some(found) = memchr(quote, &self.bytes[body..]) else {
                            return Ok(None);
                        };
                        let end = body + found;
                        if end - start > self.limits.max_tag_bytes as usize {
                            return Err(self.tag_too_long());
                        }
                        self.check_chars(body, end, true);
                        raw_value = &self.src[body..end];
                        value_span = Span::new(body, end);
                        at = end + 1;
                    }
                    Some(_) => {
                        let body = at;
                        while let Some(&byte) = self.bytes.get(at) {
                            if is_space(byte) || byte == b'>' {
                                break;
                            }
                            at += 1;
                        }
                        raw_value = &self.src[body..at];
                        value_span = Span::new(body, at);
                        if body == at {
                            self.fixes.structural = true;
                        } else {
                            self.edit(body, at, quoted(raw_value));
                        }
                    }
                }
            } else {
                raw_value = "";
                value_span = Span::new(name_end, name_end);
                self.edit(name_end, name_end, format!("=\"{name}\""));
            }
            attrs.push(Attr {
                name,
                raw_value,
                value_span,
            });
            if attrs.len() > self.limits.max_attrs_per_tag as usize {
                return Err(Stop::Failed(Error::Limit(format!(
                    "A tag in this document has more than the limit of {} attributes.",
                    self.limits.max_attrs_per_tag
                ))));
            }
        }
    }
}

struct ParsedTag<'s> {
    attrs: Vec<Attr<'s>>,
    end: usize,
    self_closed: bool,
}

fn quoted(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    let mut rest = raw;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '&' => match char_ref(rest) {
                Some(reference) if reference.needs_numeric_form => {
                    out.push_str(&format!("&#{};", reference.ch as u32));
                    rest = &rest[reference.len..];
                }
                Some(reference) => {
                    out.push_str(&rest[..reference.len]);
                    rest = &rest[reference.len..];
                }
                None => {
                    out.push_str("&amp;");
                    rest = &rest[1..];
                }
            },
            '<' => {
                out.push_str("&lt;");
                rest = &rest[1..];
            }
            '"' => {
                out.push_str("&quot;");
                rest = &rest[1..];
            }
            other => {
                if is_xml_char(other as u32) {
                    out.push(other);
                }
                rest = &rest[other.len_utf8()..];
            }
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;
