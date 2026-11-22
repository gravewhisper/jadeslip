use crate::markup::{
    Attr, ElementClass, Event, Fixes, SourceEdit, Span, char_ref, class_of, is_invisible_char,
    is_space, local_name, scan,
};
use crate::{BlockIx, DomPoint, DomRange, Error, Limits, VisRange};
use encoding_rs::Encoding;
use memchr::{memchr, memmem};
use std::ops::Range;
use std::sync::OnceLock;

pub(crate) enum SourceEncoding {
    Utf8 {
        bom: bool,
    },
    Other {
        encoding: &'static str,
        raw: Vec<u8>,
    },
}

pub(crate) struct ChapterModel {
    pub(crate) source: String,
    pub(crate) encoding: SourceEncoding,
    pub(crate) layout: BlockLayout,
    pub(crate) text: VisibleText,
    pub(crate) fixes: Fixes,
}

pub(crate) struct BlockLayout {
    pub(crate) blocks: Vec<Block>,
    pub(crate) body_inner: Span,
    pub(crate) declarations: Vec<Span>,
}

pub(crate) struct Block {
    pub(crate) span: Span,
    pub(crate) kind: BlockKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BlockKind {
    Element,
    BodyInner,
}

const SNIFF_WINDOW: usize = 64 << 10;

fn xml_declared_label(raw: &[u8]) -> Option<&[u8]> {
    if !raw.starts_with(b"<?xml") {
        return None;
    }
    let declaration = &raw[..memmem::find(&raw[..raw.len().min(256)], b"?>")?];
    let at = memmem::find(declaration, b"encoding")? + "encoding".len();
    let after = &declaration[at..];
    let equals = memchr(b'=', after)?;
    let value = &after[equals + 1..];
    let open = value.iter().position(|&byte| !is_space(byte))?;
    let quote = *value
        .get(open)
        .filter(|byte| matches!(byte, b'"' | b'\''))?;
    let body = &value[open + 1..];
    Some(&body[..memchr(quote, body)?])
}

fn meta_declared_label(raw: &[u8]) -> Option<&[u8]> {
    let window = &raw[..raw.len().min(SNIFF_WINDOW)];
    let mut at = 0;
    while let Some(found) = memchr(b'<', &window[at..]) {
        at += found;
        let rest = &window[at..];
        if rest.starts_with(b"<!--") {
            at += memmem::find(rest, b"-->")? + 3;
            continue;
        }
        let name_is = |name: &[u8]| {
            rest.get(1..1 + name.len())
                .is_some_and(|got| got.eq_ignore_ascii_case(name))
                && rest
                    .get(1 + name.len())
                    .is_some_and(|&next| is_space(next) || matches!(next, b'/' | b'>'))
        };
        if name_is(b"body") {
            return None;
        }
        if name_is(b"meta") {
            let tag = &rest[..memchr(b'>', rest)?];
            if let Some(label) = charset_in_tag(tag) {
                return Some(label);
            }
        }
        at += 1;
    }
    None
}

fn charset_in_tag(tag: &[u8]) -> Option<&[u8]> {
    const NAME: &[u8] = b"charset";
    let at = tag
        .windows(NAME.len())
        .position(|window| window.eq_ignore_ascii_case(NAME))?;
    if tag[at - 1.min(at)].is_ascii_alphanumeric() || tag[at - 1.min(at)] == b'-' {
        return None;
    }
    let mut rest = &tag[at + NAME.len()..];
    let skip = rest.iter().position(|&byte| !is_space(byte))?;
    rest = rest[skip..].strip_prefix(b"=")?;
    let skip = rest.iter().position(|&byte| !is_space(byte))?;
    rest = &rest[skip..];
    if let Some(&quote) = rest.first().filter(|byte| matches!(byte, b'"' | b'\'')) {
        rest = &rest[1..];
        let end = rest
            .iter()
            .position(|&byte| byte == quote || byte == b';')?;
        return Some(&rest[..end]);
    }
    let end = rest
        .iter()
        .position(|&byte| is_space(byte) || matches!(byte, b';' | b'/' | b'"' | b'\''))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn declared_encoding(raw: &[u8]) -> Option<&'static Encoding> {
    let label = xml_declared_label(raw).or_else(|| meta_declared_label(raw))?;
    let encoding = Encoding::for_label_no_replacement(label)?;
    let self_describing = encoding == encoding_rs::UTF_8
        || encoding == encoding_rs::UTF_16LE
        || encoding == encoding_rs::UTF_16BE;
    (!self_describing).then_some(encoding)
}

pub(crate) fn decode_source(raw: Vec<u8>) -> (String, SourceEncoding) {
    if raw.starts_with(b"\xEF\xBB\xBF") {
        return decode_utf8(raw, true);
    }
    if raw.starts_with(b"\xFF\xFE") {
        return decode_with(encoding_rs::UTF_16LE, 2, raw);
    }
    if raw.starts_with(b"\xFE\xFF") {
        return decode_with(encoding_rs::UTF_16BE, 2, raw);
    }
    match declared_encoding(&raw) {
        Some(encoding) => {
            let strict = encoding
                .decode_without_bom_handling_and_without_replacement(&raw)
                .map(|decoded| decoded.into_owned());
            match strict {
                Some(source) => (
                    source,
                    SourceEncoding::Other {
                        encoding: encoding.name(),
                        raw,
                    },
                ),
                None if std::str::from_utf8(&raw).is_ok() => decode_utf8(raw, false),
                None => decode_with(encoding, 0, raw),
            }
        }
        None => decode_utf8(raw, false),
    }
}

fn decode_utf8(raw: Vec<u8>, bom: bool) -> (String, SourceEncoding) {
    let skip = if bom { 3 } else { 0 };
    if std::str::from_utf8(&raw[skip..]).is_ok() {
        let mut bytes = raw;
        bytes.drain(..skip);
        let source = String::from_utf8(bytes).expect("validated just above");
        return (source, SourceEncoding::Utf8 { bom });
    }
    decode_with(encoding_rs::UTF_8, skip, raw)
}

fn decode_with(encoding: &'static Encoding, skip: usize, raw: Vec<u8>) -> (String, SourceEncoding) {
    let (decoded, _) = encoding.decode_without_bom_handling(&raw[skip..]);
    let source = decoded.into_owned();
    (
        source,
        SourceEncoding::Other {
            encoding: encoding.name(),
            raw,
        },
    )
}

pub(crate) fn decode_and_model(raw: Vec<u8>, limits: &Limits) -> Result<ChapterModel, Error> {
    let (source, encoding) = decode_source(raw);
    model_of_source(source, encoding, limits)
}

pub(crate) fn model_of_source(
    source: String,
    encoding: SourceEncoding,
    limits: &Limits,
) -> Result<ChapterModel, Error> {
    let scanned = scan(&source, limits)?;
    let (layout, block_events) = BlockLayout::build(&scanned.events, &source);
    let text = VisibleText::fold(&scanned.events, &source, &layout.blocks, &block_events);
    let mut fixes = scanned.fixes;
    fixes
        .point
        .extend(declaration_fix(&source, &encoding, &layout));
    Ok(ChapterModel {
        source,
        encoding,
        layout,
        text,
        fixes,
    })
}

fn declaration_fix(
    source: &str,
    encoding: &SourceEncoding,
    layout: &BlockLayout,
) -> Option<SourceEdit> {
    let decoded_as_utf8 = match encoding {
        SourceEncoding::Utf8 { .. } => true,
        SourceEncoding::Other { encoding, .. } => *encoding == "UTF-8",
    };
    let span = *layout.declarations.first()?;
    let in_xml_declaration =
        source.starts_with("<?xml") && span.end as usize <= source.find("?>")?;
    let label = &source[span.range()];
    (decoded_as_utf8 && in_xml_declaration && !label.eq_ignore_ascii_case("utf-8")).then(|| {
        SourceEdit {
            span,
            text: "UTF-8".to_owned(),
        }
    })
}

enum Frame {
    Container {
        open: u32,
        start: u32,
        inner: u32,
        first_block: u32,
        has_inline: bool,
        is_root: bool,
    },
    Transparent,
}

struct LayoutWalk {
    frames: Vec<Frame>,
    blocks: Vec<Block>,
    ranges: Vec<Range<u32>>,
    skip: u32,
    leaf_open: Option<u32>,
    leaf_start: u32,
}

impl LayoutWalk {
    fn mark_inline(&mut self) {
        for frame in self.frames.iter_mut().rev() {
            if let Frame::Container { has_inline, .. } = frame {
                *has_inline = true;
                return;
            }
        }
    }
}

impl BlockLayout {
    pub(crate) fn build(events: &[Event<'_>], source: &str) -> (BlockLayout, Vec<Range<u32>>) {
        let mut walk = LayoutWalk {
            frames: vec![Frame::Container {
                open: 0,
                start: 0,
                inner: 0,
                first_block: 0,
                has_inline: false,
                is_root: true,
            }],
            blocks: Vec::new(),
            ranges: Vec::new(),
            skip: 0,
            leaf_open: None,
            leaf_start: 0,
        };
        let mut body_inner = Span::new(0, source.len());
        let mut body_open = None;
        let mut declarations = Vec::new();
        let mut in_head = true;

        for (ix, event) in events.iter().enumerate() {
            let ix = ix as u32;
            match event {
                Event::Other { span } if ix == 0 => {
                    if let Some(value) = xml_encoding_span(source, *span) {
                        declarations.push(value);
                    }
                }
                Event::Open { name, attrs, .. } if in_head => {
                    let local = local_name(name);
                    if local.eq_ignore_ascii_case("body") {
                        in_head = false;
                    } else if local.eq_ignore_ascii_case("meta") {
                        declarations.extend(meta_charset_span(attrs, source));
                    }
                }
                _ => {}
            }

            if walk.skip > 0 {
                match event {
                    Event::Open {
                        self_closed: false, ..
                    } => walk.skip += 1,
                    Event::Close { span, .. } => {
                        walk.skip -= 1;
                        if walk.skip == 0
                            && let Some(open) = walk.leaf_open.take()
                        {
                            walk.blocks.push(Block {
                                span: Span {
                                    start: walk.leaf_start,
                                    end: span.end,
                                },
                                kind: BlockKind::Element,
                            });
                            walk.ranges.push(open..ix + 1);
                        }
                    }
                    _ => {}
                }
                continue;
            }

            match event {
                Event::Text { raw, .. } | Event::CData { raw, .. } => {
                    if !raw.bytes().all(is_space) {
                        walk.mark_inline();
                    }
                }
                Event::Other { .. } => {}
                Event::Open {
                    name,
                    span,
                    self_closed,
                    ..
                } => {
                    let local = local_name(name);
                    match class_of(local) {
                        ElementClass::Container if !self_closed => {
                            let is_body = local.eq_ignore_ascii_case("body");
                            if is_body {
                                body_open = Some(span.end);
                            }
                            walk.frames.push(Frame::Container {
                                open: ix,
                                start: span.start,
                                inner: span.end,
                                first_block: walk.blocks.len() as u32,
                                has_inline: false,
                                is_root: is_body || local.eq_ignore_ascii_case("html"),
                            });
                        }
                        ElementClass::Container => {}
                        ElementClass::Unknown if !self_closed => {
                            walk.frames.push(Frame::Transparent)
                        }
                        ElementClass::Unknown => {}
                        ElementClass::Inline => {
                            walk.mark_inline();
                            if !self_closed {
                                walk.skip = 1;
                            }
                        }
                        ElementClass::Block | ElementClass::Replaced => {
                            if *self_closed {
                                walk.blocks.push(Block {
                                    span: *span,
                                    kind: BlockKind::Element,
                                });
                                walk.ranges.push(ix..ix + 1);
                            } else {
                                walk.skip = 1;
                                walk.leaf_open = Some(ix);
                                walk.leaf_start = span.start;
                            }
                        }
                        ElementClass::NonRendered | ElementClass::Forbidden => {
                            if !self_closed {
                                walk.skip = 1;
                            }
                        }
                    }
                }
                Event::Close { span, .. } => {
                    let Some(Frame::Container {
                        open,
                        start,
                        inner,
                        first_block,
                        has_inline,
                        is_root,
                    }) = walk.frames.pop()
                    else {
                        continue;
                    };
                    if body_open == Some(inner) {
                        body_inner = Span {
                            start: inner,
                            end: span.start,
                        };
                    }
                    if has_inline {
                        walk.blocks.truncate(first_block as usize);
                        walk.ranges.truncate(first_block as usize);
                        if is_root {
                            walk.blocks.push(Block {
                                span: Span {
                                    start: inner,
                                    end: span.start,
                                },
                                kind: BlockKind::BodyInner,
                            });
                            walk.ranges.push(open + 1..ix);
                        } else {
                            walk.blocks.push(Block {
                                span: Span {
                                    start,
                                    end: span.end,
                                },
                                kind: BlockKind::Element,
                            });
                            walk.ranges.push(open..ix + 1);
                        }
                    }
                }
            }
        }

        if let Some(Frame::Container {
            has_inline: true,
            first_block,
            ..
        }) = walk.frames.first()
        {
            let first_block = *first_block as usize;
            walk.blocks.truncate(first_block);
            walk.ranges.truncate(first_block);
            walk.blocks.push(Block {
                span: Span::new(0, source.len()),
                kind: BlockKind::BodyInner,
            });
            walk.ranges.push(0..events.len() as u32);
        }

        (
            BlockLayout {
                blocks: walk.blocks,
                body_inner,
                declarations,
            },
            walk.ranges,
        )
    }
}

fn xml_encoding_span(source: &str, span: Span) -> Option<Span> {
    let declaration = &source[span.range()];
    declaration.strip_prefix("<?xml")?;
    let at = declaration.find("encoding")? + "encoding".len();
    let after = &declaration[at..];
    let open = after.find(['"', '\''])?;
    let quote = after.as_bytes()[open];
    let end = memchr(quote, &after.as_bytes()[open + 1..])?;
    let value_start = span.start as usize + at + open + 1;
    Some(Span::new(value_start, value_start + end))
}

fn meta_charset_span(attrs: &[Attr<'_>], source: &str) -> Option<Span> {
    for attr in attrs {
        if attr.name.eq_ignore_ascii_case("charset") {
            return Some(attr.value_span);
        }
        if attr.name.eq_ignore_ascii_case("content") {
            let value = &source[attr.value_span.range()];
            let lowered = value.to_ascii_lowercase();
            let at = lowered.find("charset=")? + "charset=".len();
            let end = value[at..]
                .find([';', ' ', '"', '\''])
                .map_or(value.len(), |found| at + found);
            let start = attr.value_span.start as usize;
            return Some(Span::new(start + at, start + end));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SegKind {
    Literal,
    Entity,
    Space { dom_units: u32 },
    HiddenSpace { dom_units: u32 },
    Break,
    BlockEnd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub(crate) vis: u32,
    pub(crate) src: u32,
    pub(crate) src_len: u32,
    pub(crate) kind: SegKind,
}

pub const BREAK_UNIT: u32 = 0x11_0000;

pub struct VisibleText {
    text: String,
    segs: Vec<Seg>,
    block_first_seg: Vec<u32>,
    units_upto: OnceLock<Vec<u32>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Start,
    End,
}

struct Fold {
    text: String,
    segs: Vec<Seg>,
    line_start: bool,
    pending_space: Option<usize>,
    pending_boundary: bool,
    pre_depth: u32,
}

impl Fold {
    fn visible_starts(&mut self) {
        if self.pending_boundary {
            self.pending_boundary = false;
            let at = self.segs.last().map_or(0, |seg| seg.src + seg.src_len);
            self.push_seg(at, 0, SegKind::BlockEnd);
            self.text.push('\n');
            self.line_start = true;
        }
        if let Some(pending) = self.pending_space.take() {
            let seg = &mut self.segs[pending];
            if let SegKind::HiddenSpace { dom_units } = seg.kind {
                seg.kind = SegKind::Space { dom_units };
            }
            self.text.push(' ');
            for later in &mut self.segs[pending + 1..] {
                later.vis += 1;
            }
        }
        self.line_start = false;
    }

    fn push_seg(&mut self, src: u32, src_len: u32, kind: SegKind) {
        self.segs.push(Seg {
            vis: self.text.len() as u32,
            src,
            src_len,
            kind,
        });
    }

    fn whitespace(&mut self, src: u32, src_len: u32, dom_units: u32) {
        let hidden = self.line_start || self.pending_boundary || self.pending_space.is_some();
        self.push_seg(src, src_len, SegKind::HiddenSpace { dom_units });
        if !hidden {
            self.pending_space = Some(self.segs.len() - 1);
        }
    }

    fn boundary(&mut self) {
        self.pending_space = None;
        if !self.line_start {
            self.pending_boundary = true;
        }
    }

    fn line_break(&mut self, span: Span) {
        if self.pending_boundary {
            self.visible_starts();
        }
        self.pending_space = None;
        self.push_seg(span.start, span.end - span.start, SegKind::Break);
        self.text.push('\n');
        self.line_start = true;
    }

    fn end_block(&mut self, at: u32) {
        self.pending_space = None;
        if self.pending_boundary || !self.line_start {
            self.pending_boundary = false;
            self.push_seg(at, 0, SegKind::BlockEnd);
            self.text.push('\n');
            self.line_start = true;
        }
    }

    fn entity(&mut self, src: u32, src_len: u32, ch: char) {
        self.visible_starts();
        self.push_seg(src, src_len, SegKind::Entity);
        self.text.push(ch);
    }

    fn begin_literal(&mut self, src: u32) {
        self.visible_starts();
        self.push_seg(src, 0, SegKind::Literal);
    }

    fn end_literal(&mut self, raw: &str, literal_from: &mut Option<usize>, at: usize) {
        if let Some(from) = literal_from.take() {
            self.text.push_str(&raw[from..at]);
            let last = self.segs.len() - 1;
            self.segs[last].src_len = (at - from) as u32;
        }
    }

    fn run(&mut self, raw: &str, base: u32, entities: bool) {
        let bytes = raw.as_bytes();
        let mut literal: Option<usize> = None;
        let mut at = 0;
        while at < bytes.len() {
            let byte = bytes[at];
            let in_pre = self.pre_depth > 0;
            if in_pre && byte == 0x0C {
                self.end_literal(raw, &mut literal, at);
                self.entity(base + at as u32, 1, ' ');
                at += 1;
            } else if in_pre && byte == b'\r' {
                self.end_literal(raw, &mut literal, at);
                let len = if bytes.get(at + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                self.entity(base + at as u32, len, '\n');
                at += len as usize;
            } else if is_space(byte) && !in_pre {
                let stays_in_literal = byte == b' '
                    && literal.is_some()
                    && bytes
                        .get(at + 1)
                        .is_some_and(|&next| !is_space(next) && next != b'&')
                    && !is_invisible_char(&bytes[at + 1..]);
                if stays_in_literal {
                    at += 1;
                    continue;
                }
                self.end_literal(raw, &mut literal, at);
                let start = at;
                let mut dom_units = 0;
                while at < bytes.len() && is_space(bytes[at]) {
                    if !(bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n')) {
                        dom_units += 1;
                    }
                    at += 1;
                }
                self.whitespace(base + start as u32, (at - start) as u32, dom_units);
            } else if byte == b'&'
                && entities
                && let Some(found) = char_ref(&raw[at..])
            {
                self.end_literal(raw, &mut literal, at);
                let src = base + at as u32;
                if matches!(found.ch, ' ' | '\t' | '\n' | '\r') && !in_pre {
                    self.whitespace(src, found.len as u32, 1);
                } else {
                    self.entity(src, found.len as u32, found.ch);
                }
                at += found.len;
            } else if is_invisible_char(&bytes[at..]) {
                self.end_literal(raw, &mut literal, at);
                let len = if byte == 0xEF { 3 } else { 1 };
                self.push_seg(base + at as u32, len, SegKind::HiddenSpace { dom_units: 0 });
                at += len as usize;
            } else {
                if literal.is_none() {
                    self.begin_literal(base + at as u32);
                    literal = Some(at);
                }
                at += 1;
            }
        }
        self.end_literal(raw, &mut literal, at);
    }
}

impl VisibleText {
    pub(crate) fn empty() -> VisibleText {
        VisibleText {
            text: String::new(),
            segs: Vec::new(),
            block_first_seg: Vec::new(),
            units_upto: OnceLock::new(),
        }
    }

    pub(crate) fn fold(
        events: &[Event<'_>],
        source: &str,
        blocks: &[Block],
        ranges: &[Range<u32>],
    ) -> VisibleText {
        let mut fold = Fold {
            text: String::with_capacity(source.len() / 2),
            segs: Vec::with_capacity(source.len() / 256 + 8),
            line_start: true,
            pending_space: None,
            pending_boundary: false,
            pre_depth: 0,
        };
        let mut block_first_seg = Vec::with_capacity(blocks.len());
        for (block, range) in blocks.iter().zip(ranges) {
            block_first_seg.push(fold.segs.len() as u32);
            fold_block(&mut fold, &events[range.start as usize..range.end as usize]);
            fold.end_block(block.span.end);
            fold.pre_depth = 0;
        }
        if let Some(last) = fold
            .segs
            .iter()
            .rposition(|seg| seg.kind == SegKind::BlockEnd)
            && fold.segs[last].vis as usize + 1 == fold.text.len()
        {
            fold.text.pop();
            fold.segs.remove(last);
            for seg in &mut fold.segs[last..] {
                seg.vis -= 1;
            }
            for first in &mut block_first_seg {
                if *first as usize > last {
                    *first -= 1;
                }
            }
        }
        VisibleText {
            text: fold.text,
            segs: fold.segs,
            block_first_seg,
            units_upto: OnceLock::new(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[doc(hidden)]
    pub fn check_dom_blocks(&self, blocks: &[Vec<u32>]) -> Result<(u64, u64), String> {
        let (visible, segs, firsts) = (&self.text, &self.segs, &self.block_first_seg);
        if firsts.is_empty() {
            if blocks.iter().any(|units| !units.is_empty()) {
                return Err("the DOM has text and the model has none".to_owned());
            }
            return Ok((blocks.len() as u64, 0));
        }
        if blocks.len() != firsts.len() {
            return Err(format!(
                "{} DOM blocks against {} model blocks",
                blocks.len(),
                firsts.len()
            ));
        }
        let mut total = 0;
        for (block, units) in blocks.iter().enumerate() {
            let start = firsts[block] as usize;
            let end = firsts
                .get(block + 1)
                .map_or(segs.len(), |&next| next as usize);
            let mut at = 0usize;
            for ix in start..end {
                let seg = segs[ix];
                let seg_end = segs
                    .get(ix + 1)
                    .map_or(visible.len(), |next| next.vis as usize);
                match seg.kind {
                    SegKind::Literal | SegKind::Entity => {
                        for unit in visible[seg.vis as usize..seg_end].encode_utf16() {
                            if units.get(at) != Some(&u32::from(unit)) {
                                return Err(format!("block {block}: text differs at unit {at}"));
                            }
                            at += 1;
                        }
                    }
                    SegKind::Space { dom_units } | SegKind::HiddenSpace { dom_units } => {
                        for _ in 0..dom_units {
                            let is_space = units
                                .get(at)
                                .is_some_and(|&unit| matches!(unit, 0x20 | 0x9 | 0xA | 0xD));
                            if !is_space {
                                return Err(format!("block {block}: space differs at unit {at}"));
                            }
                            at += 1;
                        }
                    }
                    SegKind::Break => {
                        if units.get(at) != Some(&BREAK_UNIT) {
                            return Err(format!("block {block}: break differs at unit {at}"));
                        }
                        at += 1;
                    }
                    SegKind::BlockEnd => {}
                }
            }
            if at != units.len() {
                return Err(format!(
                    "block {block}: the DOM has {} units and the model {at}",
                    units.len()
                ));
            }
            total += at as u64;
        }
        Ok((blocks.len() as u64, total))
    }

    pub(crate) fn visible_within(&self, start: u32, end: u32) -> Option<VisRange> {
        let from = self.segs.partition_point(|seg| seg.src < start);
        let to = self.segs.partition_point(|seg| seg.src < end);
        let shown = |ix: &usize| matches!(self.segs[*ix].kind, SegKind::Literal | SegKind::Entity);
        let first = (from..to).find(shown)?;
        let last = (from..to).rev().find(shown)?;
        Some(VisRange {
            start: self.segs[first].vis,
            end: self.seg_end(last) as u32,
        })
    }

    #[cfg(test)]
    pub(crate) fn parts(&self) -> (&str, &[Seg], &[u32]) {
        (&self.text, &self.segs, &self.block_first_seg)
    }

    fn seg_end(&self, ix: usize) -> usize {
        self.segs
            .get(ix + 1)
            .map_or(self.text.len(), |next| next.vis as usize)
    }

    fn seg_units(&self, ix: usize) -> u32 {
        let seg = &self.segs[ix];
        match seg.kind {
            SegKind::Literal | SegKind::Entity => self.text[seg.vis as usize..self.seg_end(ix)]
                .encode_utf16()
                .count() as u32,
            SegKind::Space { dom_units } | SegKind::HiddenSpace { dom_units } => dom_units,
            SegKind::Break => 1,
            SegKind::BlockEnd => 0,
        }
    }

    fn block_of_seg(&self, seg: usize) -> usize {
        self.block_first_seg
            .partition_point(|&first| first as usize <= seg)
            - 1
    }

    fn block_segs(&self, block: usize) -> Range<usize> {
        let start = self.block_first_seg[block] as usize;
        let end = self
            .block_first_seg
            .get(block + 1)
            .map_or(self.segs.len(), |&next| next as usize);
        start..end
    }

    fn clamp(&self, range: VisRange) -> (usize, usize) {
        let len = self.text.len();
        let mut start = (range.start.min(range.end) as usize).min(len);
        let mut end = (range.start.max(range.end) as usize).min(len);
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        while !self.text.is_char_boundary(end) {
            end += 1;
        }
        (start, end)
    }

    fn units_before(&self, block_start: usize, seg: usize) -> u32 {
        let upto = self.units_upto.get_or_init(|| {
            let mut total = 0;
            let mut upto = Vec::with_capacity(self.segs.len() + 1);
            upto.push(0);
            for ix in 0..self.segs.len() {
                total += self.seg_units(ix);
                upto.push(total);
            }
            upto
        });
        upto[seg] - upto[block_start]
    }

    pub(crate) fn crosses_block(&self, start: usize, end: usize) -> bool {
        let first = self.segs.partition_point(|seg| (seg.vis as usize) < start);
        self.segs[first..]
            .iter()
            .take_while(|seg| (seg.vis as usize) < end)
            .any(|seg| seg.kind == SegKind::BlockEnd)
    }

    pub(crate) fn spans_tags(&self, start: usize, end: usize) -> bool {
        let first = self
            .segs
            .partition_point(|seg| seg.vis as usize <= start)
            .saturating_sub(1);
        let mut previous: Option<Seg> = None;
        for seg in self.segs[first..]
            .iter()
            .take_while(|seg| (seg.vis as usize) < end)
        {
            if matches!(seg.kind, SegKind::Break | SegKind::BlockEnd) {
                return true;
            }
            if previous.is_some_and(|before| before.src + before.src_len != seg.src) {
                return true;
            }
            previous = Some(*seg);
        }
        false
    }

    fn dom_point(&self, pos: usize, side: Side) -> DomPoint {
        if self.segs.is_empty() {
            return DomPoint {
                block: BlockIx(0),
                offset_u16: 0,
            };
        }
        let seg = match side {
            Side::Start => self
                .segs
                .partition_point(|seg| seg.vis as usize <= pos)
                .saturating_sub(1),
            Side::End if pos == 0 => {
                return DomPoint {
                    block: BlockIx(0),
                    offset_u16: 0,
                };
            }
            Side::End => self.segs.partition_point(|seg| (seg.vis as usize) < pos) - 1,
        };
        let block = self.block_of_seg(seg);
        let block_start = self.block_first_seg[block] as usize;
        if side == Side::End
            && self.segs[seg].kind == SegKind::BlockEnd
            && self.is_last_in_block(seg, block)
        {
            return DomPoint {
                block: BlockIx(block as u32 + 1),
                offset_u16: 0,
            };
        }
        if side == Side::Start && pos >= self.text.len() && self.seg_end(seg) <= pos {
            let all = self.block_segs(block);
            return DomPoint {
                block: BlockIx(block as u32),
                offset_u16: self.units_before(block_start, all.end),
            };
        }
        let seg_vis = self.segs[seg].vis as usize;
        let inner = match self.segs[seg].kind {
            SegKind::Literal | SegKind::Entity => {
                self.text[seg_vis..pos].encode_utf16().count() as u32
            }
            SegKind::Space { dom_units } if pos > seg_vis => dom_units,
            SegKind::Break if pos > seg_vis => 1,
            _ => 0,
        };
        DomPoint {
            block: BlockIx(block as u32),
            offset_u16: self.units_before(block_start, seg) + inner,
        }
    }

    fn is_last_in_block(&self, seg: usize, block: usize) -> bool {
        seg + 1 == self.block_segs(block).end
    }

    fn vis_at(&self, point: DomPoint, side: Side) -> usize {
        if point.block.0 as usize >= self.block_first_seg.len() {
            return self.text.len();
        }
        let segs = self.block_segs(point.block.0 as usize);
        let mut remaining = point.offset_u16;
        for ix in segs.clone() {
            if side == Side::End && remaining == 0 {
                return self.segs[ix].vis as usize;
            }
            if side == Side::Start
                && remaining == 0
                && self.segs[ix].kind == SegKind::BlockEnd
                && ix + 1 == segs.end
            {
                return self.segs[ix].vis as usize;
            }
            let units = self.seg_units(ix);
            if remaining < units {
                return self.vis_inside(ix, remaining, side);
            }
            remaining -= units;
        }
        match segs.end.checked_sub(1).filter(|_| !segs.is_empty()) {
            Some(last) => self.seg_end(last),
            None => self
                .segs
                .get(segs.start)
                .map_or(self.text.len(), |seg| seg.vis as usize),
        }
    }

    fn vis_inside(&self, ix: usize, units: u32, side: Side) -> usize {
        let seg = &self.segs[ix];
        let start = seg.vis as usize;
        match seg.kind {
            SegKind::Literal | SegKind::Entity => {
                let mut walked = 0;
                for (offset, ch) in self.text[start..self.seg_end(ix)].char_indices() {
                    let width = ch.len_utf16() as u32;
                    if walked + width > units {
                        let inside_surrogates = walked != units;
                        return start
                            + offset
                            + if side == Side::End && inside_surrogates {
                                ch.len_utf8()
                            } else {
                                0
                            };
                    }
                    walked += width;
                }
                self.seg_end(ix)
            }
            SegKind::Space { .. } if side == Side::End && units > 0 => self.seg_end(ix),
            _ => start,
        }
    }

    pub fn to_dom(&self, range: VisRange) -> DomRange {
        let (start, end) = self.clamp(range);
        let start_point = self.dom_point(start, Side::Start);
        if start == end {
            return DomRange {
                start: start_point,
                end: start_point,
            };
        }
        DomRange {
            start: start_point,
            end: self.dom_point(end, Side::End),
        }
    }

    pub fn from_dom(&self, range: DomRange) -> VisRange {
        let start = self.vis_at(range.start, Side::Start);
        if range.start == range.end {
            return VisRange {
                start: start as u32,
                end: start as u32,
            };
        }
        let end = self.vis_at(range.end, Side::End);
        VisRange {
            start: start.min(end) as u32,
            end: start.max(end) as u32,
        }
    }

    pub(crate) fn source_edits(&self, range: VisRange, replacement: &str) -> Vec<SourceEdit> {
        let (start, end) = self.clamp(range);
        let escaped = escape_text(replacement);
        let mut edits = Vec::new();
        let mut placed = false;
        let first = self
            .segs
            .partition_point(|seg| (seg.vis as usize) < start.max(1) - 1)
            .saturating_sub(1);
        let mut removed_a_space = false;
        for ix in first..self.segs.len() {
            let seg = self.segs[ix];
            let (vis_start, vis_end) = (seg.vis as usize, self.seg_end(ix));
            if removed_a_space && matches!(seg.kind, SegKind::HiddenSpace { .. }) {
                edits.push(SourceEdit {
                    span: Span {
                        start: seg.src,
                        end: seg.src + seg.src_len,
                    },
                    text: String::new(),
                });
                continue;
            }
            removed_a_space = false;
            if vis_start >= end {
                break;
            }
            if vis_end <= start {
                continue;
            }
            removed_a_space = matches!(seg.kind, SegKind::Space { .. });
            let span = match seg.kind {
                SegKind::Literal => {
                    let low = start.max(vis_start) - vis_start;
                    let high = end.min(vis_end) - vis_start;
                    Span {
                        start: seg.src + low as u32,
                        end: seg.src + high as u32,
                    }
                }
                SegKind::Entity | SegKind::Space { .. } => Span {
                    start: seg.src,
                    end: seg.src + seg.src_len,
                },
                SegKind::HiddenSpace { .. } | SegKind::Break | SegKind::BlockEnd => continue,
            };
            edits.push(SourceEdit {
                span,
                text: if placed {
                    String::new()
                } else {
                    escaped.clone()
                },
            });
            placed = true;
        }
        if !placed
            && !escaped.is_empty()
            && let Some(at) = self.insertion_point(start)
        {
            edits.push(SourceEdit {
                span: Span { start: at, end: at },
                text: escaped,
            });
        }
        edits
    }

    fn insertion_point(&self, pos: usize) -> Option<u32> {
        let at = self
            .segs
            .partition_point(|seg| seg.vis as usize <= pos)
            .checked_sub(1)?;
        let seg = self.segs[at];
        let inside = (seg.vis as usize) < pos && pos < self.seg_end(at);
        if inside && seg.kind == SegKind::Literal {
            return Some(seg.src + (pos - seg.vis as usize) as u32);
        }
        let starts_here = seg.vis as usize == pos && self.seg_end(at) > pos;
        if starts_here
            && matches!(
                seg.kind,
                SegKind::Literal | SegKind::Entity | SegKind::Space { .. }
            )
        {
            return Some(seg.src);
        }
        let before = self
            .segs
            .partition_point(|seg| (seg.vis as usize) < pos)
            .checked_sub(1);
        match before.map(|ix| self.segs[ix]) {
            Some(previous)
                if starts_here && matches!(previous.kind, SegKind::Break | SegKind::BlockEnd) =>
            {
                Some(seg.src)
            }
            Some(previous) => Some(previous.src + previous.src_len),
            None => Some(seg.src),
        }
    }
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

fn fold_block(fold: &mut Fold, events: &[Event<'_>]) {
    let mut skip = 0u32;
    for event in events {
        if skip > 0 {
            match event {
                Event::Open {
                    self_closed: false, ..
                } => skip += 1,
                Event::Close { .. } => skip -= 1,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Text { raw, span } => fold.run(raw, span.start, true),
            Event::CData { raw, span } => fold.run(raw, span.start, false),
            Event::Other { .. } => {}
            Event::Open {
                name,
                span,
                self_closed,
                ..
            } => {
                let local = local_name(name);
                match class_of(local) {
                    ElementClass::NonRendered
                    | ElementClass::Forbidden
                    | ElementClass::Replaced => {
                        if !self_closed {
                            skip = 1;
                        }
                    }
                    ElementClass::Container | ElementClass::Block => {
                        fold.boundary();
                        if local.eq_ignore_ascii_case("pre") && !self_closed {
                            fold.pre_depth += 1;
                        }
                    }
                    ElementClass::Inline | ElementClass::Unknown => {
                        if local.eq_ignore_ascii_case("br") {
                            fold.line_break(*span);
                        }
                    }
                }
            }
            Event::Close { name, .. } => {
                let local = local_name(name);
                if matches!(
                    class_of(local),
                    ElementClass::Container | ElementClass::Block
                ) {
                    fold.boundary();
                    if local.eq_ignore_ascii_case("pre") {
                        fold.pre_depth = fold.pre_depth.saturating_sub(1);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
