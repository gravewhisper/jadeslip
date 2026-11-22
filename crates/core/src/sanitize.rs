use crate::chapter::{Block, BlockKind};
use crate::css::{sanitize_inline, sanitize_root_style, sanitize_stylesheet};
use crate::markup::{
    Attr, ElementClass, Event, Span, class_of, decode_attr, decode_text, is_space, local_name,
};
use crate::package::{percent_encode_path, resolve_href};
use crate::zip::EntryName;
use std::ops::Range;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATH_NS: &str = "http://www.w3.org/1998/Math/MathML";
const SCHEME_PREFIX: &str = "jadeslip-book://";

#[derive(Clone)]
pub(crate) struct UrlMap {
    session: u64,
    base: EntryName,
    from_display: bool,
}

fn split_suffix(href: &str) -> (&str, &str) {
    let at = href.find(['#', '?']).unwrap_or(href.len());
    href.split_at(at)
}

impl UrlMap {
    pub(crate) fn new(session: u64, base: EntryName) -> UrlMap {
        UrlMap {
            session,
            base,
            from_display: false,
        }
    }

    pub(crate) fn reading_display(mut self) -> UrlMap {
        self.from_display = true;
        self
    }

    pub(crate) fn entry_url(&self, entry: &EntryName) -> String {
        format!(
            "{SCHEME_PREFIX}{:016x}/{}",
            self.session,
            percent_encode_path(entry.as_str())
        )
    }

    pub(crate) fn entry_of(&self, href: &str) -> Option<EntryName> {
        resolve_href(&self.base, href).ok().flatten()
    }

    pub(crate) fn to_scheme(&self, source_href: &str) -> Option<String> {
        let (_, suffix) = split_suffix(source_href);
        let entry = self.entry_of(source_href)?;
        Some(format!("{}{suffix}", self.entry_url(&entry)))
    }

    pub(crate) fn to_source_href(&self, scheme_url: &str) -> Option<String> {
        let (url, suffix) = split_suffix(scheme_url);
        let entry = entry_of_scheme_url(self.session, url)?;
        let base = self
            .base
            .as_str()
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir);
        let base_parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
        let entry_parts: Vec<&str> = entry.as_str().split('/').collect();
        let (dirs, file) = entry_parts.split_at(entry_parts.len() - 1);
        let shared = base_parts
            .iter()
            .zip(dirs)
            .take_while(|(a, b)| a == b)
            .count();
        let mut relative = "../".repeat(base_parts.len() - shared);
        for part in dirs[shared..].iter().chain(file) {
            relative.push_str(&percent_encode_path(part));
            relative.push('/');
        }
        relative.pop();
        relative.push_str(suffix);
        Some(relative)
    }
}

pub(crate) fn entry_of_scheme_url(session: u64, url: &str) -> Option<EntryName> {
    let rest = url.strip_prefix(SCHEME_PREFIX)?;
    let (host, path) = rest.split_once('/')?;
    if host != format!("{session:016x}") {
        return None;
    }
    let (path, _) = split_suffix(path);
    let root = EntryName::parse("root").ok()?;
    resolve_href(&root, path).ok().flatten()
}

pub(crate) struct ResourceUrls<'a> {
    pub(crate) map: UrlMap,
    pub(crate) serveable: &'a dyn Fn(&EntryName) -> bool,
}

impl ResourceUrls<'_> {
    pub(crate) fn resolve(&self, href: &str) -> Option<String> {
        let inverted = self
            .map
            .from_display
            .then(|| self.map.to_source_href(href))
            .flatten();
        let Href::Relative(cleaned) = classify_href(inverted.as_deref().unwrap_or(href)) else {
            return None;
        };
        let entry = self.map.entry_of(&cleaned)?;
        (self.serveable)(&entry)
            .then(|| self.map.to_scheme(&cleaned))
            .flatten()
    }
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) enum Href {
    Relative(String),
    External(String),
    Blocked,
}

pub(crate) fn classify_href(raw: &str) -> Href {
    let cleaned: String = raw
        .trim_matches(|ch: char| ch <= ' ')
        .chars()
        .filter(|ch| !matches!(ch, '\t' | '\n' | '\r'))
        .collect();
    if cleaned.contains('\\') || cleaned.starts_with('/') {
        return Href::Blocked;
    }
    let first_delimiter = cleaned.find([':', '/', '?', '#']);
    if first_delimiter.is_none_or(|at| cleaned.as_bytes()[at] != b':') {
        return Href::Relative(cleaned);
    }
    let lowered = cleaned.to_ascii_lowercase();
    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        Href::External(cleaned)
    } else if lowered.starts_with("mailto:") {
        Href::External(mail_link(&cleaned["mailto:".len()..]))
    } else {
        Href::Blocked
    }
}

const MAIL_FIELDS: [&str; 4] = ["subject", "body", "cc", "bcc"];

fn mail_link(rest: &str) -> String {
    let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
    let fields: Vec<&str> = query
        .split('&')
        .filter(|field| {
            let key = field.split_once('=').map_or(*field, |(key, _)| key);
            MAIL_FIELDS
                .iter()
                .any(|kept| key.eq_ignore_ascii_case(kept))
        })
        .collect();
    if fields.is_empty() {
        format!("mailto:{to}")
    } else {
        format!("mailto:{to}?{}", fields.join("&"))
    }
}

pub(crate) enum Sheet {
    Inline(String),
    Linked(EntryName),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChapterKind {
    Text,
    ImagePage,
    Empty,
}

pub(crate) struct Sanitized {
    pub(crate) xhtml: String,
    pub(crate) sheets: Vec<Sheet>,
    pub(crate) heading: Option<String>,
    pub(crate) head_title: Option<String>,
    pub(crate) kind: ChapterKind,
}

pub(crate) struct Context<'a> {
    pub(crate) urls: ResourceUrls<'a>,
    pub(crate) is_stylesheet: &'a dyn Fn(&EntryName) -> bool,
}

pub(crate) struct Layout<'a> {
    pub(crate) blocks: &'a [Block],
    pub(crate) ranges: &'a [Range<u32>],
    pub(crate) body_inner: Span,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Html,
    Head,
    Svg,
    Math,
    Picture,
    Raw,
}

#[derive(Clone, Copy)]
enum Closer<'s> {
    Silent,
    Lowered(&'s str),
    Exact(&'static str),
}

struct Frame<'s> {
    closer: Closer<'s>,
    parent_mode: Mode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Style,
    Title,
    Heading,
}

struct Capture {
    kind: CaptureKind,
    depth: usize,
    text: String,
}

struct Walk<'e, 's, 'c> {
    out: String,
    frames: Vec<Frame<'s>>,
    mode: Mode,
    drop_depth: u32,
    layout: &'c Layout<'c>,
    next_block: usize,
    in_block: bool,
    synthetic_block: bool,
    root_is_block: bool,
    root_block_end: usize,
    pending_mark: Option<u32>,
    capture: Option<Capture>,
    context: &'c Context<'c>,
    html_attrs: Option<&'e [Attr<'s>]>,
    body_attrs: Option<&'e [Attr<'s>]>,
    sheets: Vec<Sheet>,
    heading: Option<String>,
    head_title: Option<String>,
    saw_text: bool,
    saw_image: bool,
}

const fn special_bytes() -> [bool; 256] {
    let mut table = [false; 256];
    let mut byte = 0;
    while byte < 0x20 {
        table[byte] = !matches!(byte as u8, b'\t' | b'\n');
        byte += 1;
    }
    table[b'&' as usize] = true;
    table[b'<' as usize] = true;
    table[b'>' as usize] = true;
    table[0xEF] = true;
    table
}

const SPECIAL: [bool; 256] = special_bytes();

fn push_escaped_text(out: &mut String, text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut flushed = 0;
    let mut at = 0;
    let mut visible = false;
    while at < bytes.len() {
        let byte = bytes[at];
        if !SPECIAL[byte as usize] {
            visible = visible || !is_space(byte);
            at += 1;
            continue;
        }
        let (replacement, len) = match byte {
            b'&' => ("&amp;", 1),
            b'<' => ("&lt;", 1),
            b'>' => ("&gt;", 1),
            b'\r' => ("&#13;", 1),
            0x0C => (" ", 1),
            0xEF if matches!(bytes[at..], [0xEF, 0xBF, 0xBE | 0xBF, ..]) => ("", 3),
            0xEF => {
                visible = true;
                at += 1;
                continue;
            }
            _ => ("", 1),
        };
        visible = visible || byte == b'&' || byte == b'<' || byte == b'>';
        out.push_str(&text[flushed..at]);
        out.push_str(replacement);
        at += len;
        flushed = at;
    }
    out.push_str(&text[flushed..]);
    visible
}

fn is_invisible(ch: char) -> bool {
    matches!(ch, '\u{0}'..='\u{8}' | '\u{B}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
}

pub(crate) fn push_escaped_attr(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\u{C}' => out.push(' '),
            other if is_invisible(other) => {}
            other => out.push(other),
        }
    }
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_escaped_attr(out, value);
    out.push('"');
}

fn push_lower(out: &mut String, name: &str) {
    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        out.push_str(&name.to_ascii_lowercase());
    } else {
        out.push_str(name);
    }
}

fn normalized_cdata(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

const HTML_ATTRS: &[&str] = &[
    "align",
    "alt",
    "aria-hidden",
    "aria-label",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "class",
    "clear",
    "color",
    "colspan",
    "controls",
    "datetime",
    "dir",
    "face",
    "height",
    "href",
    "hspace",
    "id",
    "lang",
    "name",
    "nowrap",
    "open",
    "poster",
    "reversed",
    "role",
    "rowspan",
    "size",
    "span",
    "src",
    "start",
    "style",
    "title",
    "type",
    "valign",
    "value",
    "vspace",
    "width",
    "xml:lang",
];

const SVG_ELEMENTS: &[&str] = &[
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "g",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
    "text",
    "textPath",
    "tspan",
];

const SVG_TRANSPARENT: &[&str] = &["a", "switch"];

const SVG_ATTRS: &[&str] = &[
    "class",
    "clip-path",
    "clip-rule",
    "clipPathUnits",
    "color",
    "cx",
    "cy",
    "d",
    "display",
    "dominant-baseline",
    "dx",
    "dy",
    "fill",
    "fill-opacity",
    "fill-rule",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "fx",
    "fy",
    "gradientTransform",
    "gradientUnits",
    "height",
    "id",
    "lang",
    "lengthAdjust",
    "letter-spacing",
    "marker-end",
    "marker-mid",
    "marker-start",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "mask",
    "maskContentUnits",
    "maskUnits",
    "offset",
    "opacity",
    "orient",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "points",
    "preserveAspectRatio",
    "r",
    "refX",
    "refY",
    "rotate",
    "rx",
    "ry",
    "spreadMethod",
    "startOffset",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "style",
    "text-anchor",
    "text-decoration",
    "textLength",
    "transform",
    "viewBox",
    "visibility",
    "width",
    "word-spacing",
    "x",
    "x1",
    "x2",
    "xml:lang",
    "y",
    "y1",
    "y2",
];

const SVG_FUNCTIONS: &[&str] = &[
    "hsl",
    "hsla",
    "matrix",
    "rgb",
    "rgba",
    "rotate",
    "scale",
    "skewx",
    "skewy",
    "translate",
    "translatex",
    "translatey",
];

const MATH_ELEMENTS: &[&str] = &[
    "maligngroup",
    "malignmark",
    "math",
    "menclose",
    "merror",
    "mfenced",
    "mfrac",
    "mi",
    "mlabeledtr",
    "mmultiscripts",
    "mn",
    "mo",
    "mover",
    "mpadded",
    "mphantom",
    "mprescripts",
    "mroot",
    "mrow",
    "ms",
    "mspace",
    "msqrt",
    "mstyle",
    "msub",
    "msubsup",
    "msup",
    "mtable",
    "mtd",
    "mtext",
    "mtr",
    "munder",
    "munderover",
    "none",
    "semantics",
];

const MATH_ATTRS: &[&str] = &[
    "accent",
    "accentunder",
    "alttext",
    "class",
    "close",
    "columnalign",
    "columnspan",
    "denomalign",
    "depth",
    "dir",
    "display",
    "displaystyle",
    "fence",
    "form",
    "height",
    "id",
    "largeop",
    "linethickness",
    "lspace",
    "mathbackground",
    "mathcolor",
    "mathsize",
    "mathvariant",
    "movablelimits",
    "notation",
    "numalign",
    "open",
    "rowalign",
    "rowspan",
    "rspace",
    "scriptlevel",
    "separators",
    "stretchy",
    "style",
    "voffset",
    "width",
];

fn svg_value_is_inert(value: &str) -> bool {
    if value.contains('\\') {
        return false;
    }
    let mut rest = value;
    while let Some(open) = rest.find('(') {
        let name_start = rest[..open]
            .rfind(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
            .map_or(0, |at| at + 1);
        let name = rest[name_start..open].to_ascii_lowercase();
        let after = &rest[open + 1..];
        let fragment_url = name == "url"
            && after.starts_with('#')
            && after
                .find(')')
                .is_some_and(|close| !after[..close].contains(['(', ':', '/']));
        if !fragment_url && !SVG_FUNCTIONS.contains(&name.as_str()) {
            return false;
        }
        rest = after;
    }
    true
}

fn lookup_lowered(table: &[&str], name: &str) -> Option<usize> {
    const LONGEST: usize = 16;
    let mut lowered = [0u8; LONGEST];
    let buffer = lowered.get_mut(..name.len())?;
    buffer.copy_from_slice(name.as_bytes());
    buffer.make_ascii_lowercase();
    table
        .binary_search_by(|probe| probe.as_bytes().cmp(buffer))
        .ok()
}

impl<'e, 's, 'c> Walk<'e, 's, 'c> {
    fn new(capacity: usize, layout: &'c Layout<'c>, context: &'c Context<'c>) -> Self {
        Walk {
            out: String::with_capacity(capacity),
            frames: Vec::with_capacity(32),
            mode: Mode::Html,
            drop_depth: 0,
            layout,
            next_block: 0,
            in_block: false,
            synthetic_block: false,
            root_is_block: false,
            root_block_end: 0,
            pending_mark: None,
            capture: None,
            context,
            html_attrs: None,
            body_attrs: None,
            sheets: Vec::new(),
            heading: None,
            head_title: None,
            saw_text: false,
            saw_image: false,
        }
    }

    fn block_edges(&mut self, ix: u32) {
        if self.in_block
            && self
                .layout
                .ranges
                .get(self.next_block)
                .is_some_and(|range| range.end == ix)
        {
            if self.synthetic_block {
                self.out.push_str("</div>");
                self.synthetic_block = false;
            }
            self.root_block_end = self.out.len();
            self.in_block = false;
            self.next_block += 1;
        }
        if self.in_block {
            return;
        }
        let Some(range) = self.layout.ranges.get(self.next_block) else {
            return;
        };
        if range.start != ix || range.start == range.end {
            return;
        }
        self.in_block = true;
        match self.layout.blocks[self.next_block].kind {
            BlockKind::Element => self.pending_mark = Some(self.next_block as u32),
            BlockKind::BodyInner => {
                if self.layout.blocks.len() == 1 && self.out.is_empty() {
                    self.root_is_block = true;
                } else {
                    self.out.push_str("<div data-ss-b=\"");
                    self.out.push_str(&self.next_block.to_string());
                    self.out.push_str("\">");
                    self.synthetic_block = true;
                }
            }
        }
    }

    fn event(&mut self, event: &'e Event<'s>) {
        match event {
            Event::Open {
                name,
                attrs,
                self_closed,
                ..
            } => self.open(name, attrs, *self_closed),
            Event::Close { .. } => self.close(),
            Event::Text { raw, span } => {
                if self.drop_depth == 0 {
                    self.text(&decode_text(raw), *span);
                }
            }
            Event::CData { raw, span } => {
                if self.drop_depth == 0 {
                    self.text(&normalized_cdata(raw), *span);
                }
            }
            Event::Other { .. } => {}
        }
    }

    fn text(&mut self, text: &str, span: Span) {
        if let Some(capture) = &mut self.capture {
            if capture.kind != CaptureKind::Heading {
                capture.text.push_str(text);
                return;
            }
            if self.mode == Mode::Html {
                capture.text.push_str(text);
            }
        }
        match self.mode {
            Mode::Html if self.in_block => {
                if push_escaped_text(&mut self.out, text) {
                    self.saw_text = true;
                }
            }
            Mode::Html => {
                let inner = self.layout.body_inner;
                if span.start >= inner.start && span.end <= inner.end {
                    push_escaped_text(&mut self.out, text);
                }
            }
            Mode::Svg | Mode::Math => {
                push_escaped_text(&mut self.out, text);
            }
            Mode::Head | Mode::Picture | Mode::Raw => {}
        }
    }

    fn open(&mut self, name: &'s str, attrs: &'e [Attr<'s>], self_closed: bool) {
        if self.drop_depth > 0 {
            if !self_closed {
                self.drop_depth += 1;
            }
            return;
        }
        let local = local_name(name);
        match self.mode {
            Mode::Html => self.open_html(local, attrs, self_closed),
            Mode::Head => self.open_non_rendered(local, attrs, self_closed),
            Mode::Svg => self.open_svg(local, attrs, self_closed),
            Mode::Math => self.open_math(local, attrs, self_closed),
            Mode::Picture => {
                if local.eq_ignore_ascii_case("img") {
                    self.open_image(attrs, self_closed);
                } else {
                    self.drop_element(self_closed);
                }
            }
            Mode::Raw => self.drop_element(self_closed),
        }
    }

    fn drop_element(&mut self, self_closed: bool) {
        if !self_closed {
            self.drop_depth = 1;
        }
    }

    fn push_frame(&mut self, closer: Closer<'s>, mode: Mode) {
        self.frames.push(Frame {
            closer,
            parent_mode: self.mode,
        });
        self.mode = mode;
    }

    fn close(&mut self) {
        if self.drop_depth > 0 {
            self.drop_depth -= 1;
            return;
        }
        let Some(frame) = self.frames.pop() else {
            return;
        };
        match frame.closer {
            Closer::Silent => {}
            Closer::Lowered(name) => {
                self.out.push_str("</");
                push_lower(&mut self.out, name);
                self.out.push('>');
            }
            Closer::Exact(name) => {
                self.out.push_str("</");
                self.out.push_str(name);
                self.out.push('>');
            }
        }
        self.mode = frame.parent_mode;
        if self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.depth == self.frames.len())
        {
            self.finish_capture();
        }
    }

    fn finish_capture(&mut self) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        let collapsed = || {
            let joined = capture
                .text
                .replace(is_invisible, "")
                .split_ascii_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!joined.is_empty()).then_some(joined)
        };
        match capture.kind {
            CaptureKind::Style => {
                let sheet = sanitize_stylesheet(&capture.text, &self.context.urls);
                if !sheet.is_empty() {
                    self.sheets.push(Sheet::Inline(sheet));
                }
            }
            CaptureKind::Title => {
                if self.head_title.is_none() {
                    self.head_title = collapsed().map(|title| decode_text(&title).into_owned());
                }
            }
            CaptureKind::Heading => {
                if self.heading.is_none() {
                    self.heading = collapsed();
                }
            }
        }
    }

    fn start_capture(&mut self, kind: CaptureKind) {
        self.capture = Some(Capture {
            kind,
            depth: self.frames.len() - 1,
            text: String::new(),
        });
    }

    fn open_html(&mut self, local: &'s str, attrs: &'e [Attr<'s>], self_closed: bool) {
        match class_of(local) {
            ElementClass::Container => {
                let is_html = local.eq_ignore_ascii_case("html");
                let is_body = local.eq_ignore_ascii_case("body");
                if is_html && self.html_attrs.is_none() {
                    self.html_attrs = Some(attrs);
                }
                if is_body && self.body_attrs.is_none() {
                    self.body_attrs = Some(attrs);
                }
                if is_html || is_body {
                    if self.in_block {
                        self.out.push_str("<div");
                        self.finish_open(Closer::Exact("div"), self_closed, Mode::Html);
                    } else if !self_closed {
                        self.push_frame(Closer::Silent, Mode::Html);
                    }
                } else {
                    self.write_html_tag(local, attrs, self_closed, true);
                }
            }
            ElementClass::Block | ElementClass::Inline => {
                let may_start_heading = self.in_block
                    && !self_closed
                    && self.heading.is_none()
                    && self.capture.is_none()
                    && is_heading(local);
                self.write_html_tag(local, attrs, self_closed, false);
                if may_start_heading {
                    self.start_capture(CaptureKind::Heading);
                }
            }
            ElementClass::Replaced => self.open_replaced(local, attrs, self_closed),
            ElementClass::NonRendered => self.open_non_rendered(local, attrs, self_closed),
            ElementClass::Forbidden => self.drop_element(self_closed),
            ElementClass::Unknown => {
                if !self_closed {
                    self.push_frame(Closer::Silent, Mode::Html);
                }
            }
        }
    }

    fn open_non_rendered(&mut self, local: &'s str, attrs: &'e [Attr<'s>], self_closed: bool) {
        if self.mode == Mode::Html && local.eq_ignore_ascii_case("head") {
            if !self_closed {
                self.push_frame(Closer::Silent, Mode::Head);
            }
        } else if local.eq_ignore_ascii_case("style") {
            if !self_closed && self.capture.is_none() {
                self.push_frame(Closer::Silent, Mode::Raw);
                self.start_capture(CaptureKind::Style);
            } else {
                self.drop_element(self_closed);
            }
        } else if local.eq_ignore_ascii_case("title") && self.mode == Mode::Head {
            if !self_closed && self.capture.is_none() {
                self.push_frame(Closer::Silent, Mode::Raw);
                self.start_capture(CaptureKind::Title);
            } else {
                self.drop_element(self_closed);
            }
        } else {
            if local.eq_ignore_ascii_case("link") {
                self.note_stylesheet(attrs);
            }
            self.drop_element(self_closed);
        }
    }

    fn note_stylesheet(&mut self, attrs: &[Attr<'_>]) {
        let mut rel = None;
        let mut href = None;
        for attr in attrs {
            if attr.name.eq_ignore_ascii_case("rel") {
                rel.get_or_insert_with(|| decode_attr(attr.raw_value));
            } else if attr.name.eq_ignore_ascii_case("href") {
                href.get_or_insert_with(|| decode_attr(attr.raw_value));
            }
        }
        let is_sheet = rel.is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|word| word.eq_ignore_ascii_case("stylesheet"))
        });
        if !is_sheet {
            return;
        }
        let Some(Href::Relative(cleaned)) = href.as_deref().map(classify_href) else {
            return;
        };
        if let Some(entry) = self.context.urls.map.entry_of(&cleaned)
            && (self.context.is_stylesheet)(&entry)
        {
            self.sheets.push(Sheet::Linked(entry));
        }
    }

    fn push_marks(&mut self, wrapper: bool) {
        if let Some(block) = self.pending_mark.take() {
            self.out.push_str(" data-ss-b=\"");
            self.out.push_str(&block.to_string());
            self.out.push('"');
        } else if wrapper && !self.in_block {
            self.out.push_str(" data-ss-w=\"\"");
        }
    }

    fn write_html_tag(
        &mut self,
        local: &'s str,
        attrs: &[Attr<'_>],
        self_closed: bool,
        wrapper: bool,
    ) {
        self.out.push('<');
        push_lower(&mut self.out, local);
        self.push_html_attrs(local, attrs);
        self.push_marks(wrapper);
        self.finish_open(Closer::Lowered(local), self_closed, Mode::Html);
    }

    fn finish_open(&mut self, closer: Closer<'s>, self_closed: bool, mode: Mode) {
        if self_closed {
            self.out.push_str("/>");
        } else {
            self.out.push('>');
            self.push_frame(closer, mode);
        }
    }

    fn push_html_attrs(&mut self, element: &str, attrs: &[Attr<'_>]) {
        let mut seen = 0u64;
        for attr in attrs {
            let Some(ix) = lookup_lowered(HTML_ATTRS, attr.name) else {
                continue;
            };
            if seen & (1 << ix) != 0 {
                continue;
            }
            seen |= 1 << ix;
            let canonical = HTML_ATTRS[ix];
            let value = decode_attr(attr.raw_value);
            match canonical {
                "href" => {
                    if element.eq_ignore_ascii_case("a") {
                        match classify_href(&value) {
                            Href::Relative(url) | Href::External(url) => {
                                push_attr(&mut self.out, "href", &url);
                            }
                            Href::Blocked => {}
                        }
                    }
                }
                "src" | "poster" => {
                    let loads = (canonical == "src"
                        && (element.eq_ignore_ascii_case("img")
                            || element.eq_ignore_ascii_case("image")
                            || element.eq_ignore_ascii_case("video")
                            || element.eq_ignore_ascii_case("audio")))
                        || (canonical == "poster" && element.eq_ignore_ascii_case("video"));
                    if loads && let Some(url) = self.context.urls.resolve(&value) {
                        push_attr(&mut self.out, canonical, &url);
                        if canonical == "src" {
                            self.saw_image = true;
                        }
                    }
                }
                "name" => {
                    if element.eq_ignore_ascii_case("a") {
                        push_attr(&mut self.out, "name", &value);
                    }
                }
                "style" => self.push_style(&value),
                other => push_attr(&mut self.out, other, &value),
            }
        }
    }

    fn push_style(&mut self, value: &str) {
        let style = sanitize_inline(value, &self.context.urls);
        if !style.is_empty() {
            push_attr(&mut self.out, "style", &style);
        }
    }

    fn open_replaced(&mut self, local: &'s str, attrs: &'e [Attr<'s>], self_closed: bool) {
        let lowered = local.to_ascii_lowercase();
        match lowered.as_str() {
            "img" | "image" => self.open_image(attrs, self_closed),
            "svg" => {
                self.saw_image = true;
                self.out.push_str("<svg xmlns=\"");
                self.out.push_str(SVG_NS);
                self.out.push('"');
                self.push_svg_attrs("svg", attrs);
                self.push_marks(false);
                self.finish_open(Closer::Exact("svg"), self_closed, Mode::Svg);
            }
            "math" => {
                self.saw_image = true;
                self.out.push_str("<math xmlns=\"");
                self.out.push_str(MATH_NS);
                self.out.push('"');
                self.push_math_attrs(attrs);
                self.push_marks(false);
                self.finish_open(Closer::Exact("math"), self_closed, Mode::Math);
            }
            "picture" => {
                self.out.push_str("<picture");
                self.push_html_attrs("picture", attrs);
                self.push_marks(false);
                self.finish_open(Closer::Exact("picture"), self_closed, Mode::Picture);
            }
            _ => {
                self.out.push('<');
                push_lower(&mut self.out, local);
                self.push_html_attrs(local, attrs);
                self.push_marks(false);
                self.out.push_str("/>");
                self.drop_element(self_closed);
            }
        }
    }

    fn open_image(&mut self, attrs: &[Attr<'_>], self_closed: bool) {
        self.out.push_str("<img");
        self.push_html_attrs("img", attrs);
        self.push_marks(false);
        self.out.push_str("/>");
        self.drop_element(self_closed);
    }

    fn open_svg(&mut self, local: &'s str, attrs: &[Attr<'_>], self_closed: bool) {
        if let Ok(ix) = SVG_ELEMENTS.binary_search(&local) {
            let canonical = SVG_ELEMENTS[ix];
            self.out.push('<');
            self.out.push_str(canonical);
            self.push_svg_attrs(canonical, attrs);
            self.finish_open(Closer::Exact(canonical), self_closed, Mode::Svg);
        } else if SVG_TRANSPARENT.contains(&local) {
            if !self_closed {
                self.push_frame(Closer::Silent, Mode::Svg);
            }
        } else {
            self.drop_element(self_closed);
        }
    }

    fn push_svg_attrs(&mut self, element: &str, attrs: &[Attr<'_>]) {
        let mut seen = 0u128;
        let mut wrote_href = false;
        for attr in attrs {
            if local_name(attr.name) == "href" && (attr.name == "href" || attr.name == "xlink:href")
            {
                if !wrote_href {
                    wrote_href = self.push_svg_href(element, attr);
                }
                continue;
            }
            let Ok(ix) = SVG_ATTRS.binary_search(&attr.name) else {
                continue;
            };
            if seen & (1 << ix) != 0 {
                continue;
            }
            seen |= 1 << ix;
            let value = decode_attr(attr.raw_value);
            if SVG_ATTRS[ix] == "style" {
                self.push_style(&value);
            } else if svg_value_is_inert(&value) {
                push_attr(&mut self.out, SVG_ATTRS[ix], &value);
            }
        }
    }

    fn push_svg_href(&mut self, element: &str, attr: &Attr<'_>) -> bool {
        let value = decode_attr(attr.raw_value);
        let target = if element == "image" {
            self.context.urls.resolve(&value)
        } else if matches!(
            element,
            "linearGradient" | "radialGradient" | "pattern" | "textPath"
        ) && value.starts_with('#')
        {
            Some(value)
        } else {
            None
        };
        match target {
            Some(target) => {
                push_attr(&mut self.out, "href", &target);
                true
            }
            None => false,
        }
    }

    fn open_math(&mut self, local: &'s str, attrs: &[Attr<'_>], self_closed: bool) {
        if let Ok(ix) = MATH_ELEMENTS.binary_search(&local) {
            self.out.push('<');
            self.out.push_str(MATH_ELEMENTS[ix]);
            self.push_math_attrs(attrs);
            self.finish_open(Closer::Exact(MATH_ELEMENTS[ix]), self_closed, Mode::Math);
        } else {
            self.drop_element(self_closed);
        }
    }

    fn push_math_attrs(&mut self, attrs: &[Attr<'_>]) {
        let mut seen = 0u64;
        for attr in attrs {
            let Ok(ix) = MATH_ATTRS.binary_search(&attr.name) else {
                continue;
            };
            if seen & (1 << ix) != 0 {
                continue;
            }
            seen |= 1 << ix;
            let value = decode_attr(attr.raw_value);
            if MATH_ATTRS[ix] == "style" {
                self.push_style(&value);
            } else {
                push_attr(&mut self.out, MATH_ATTRS[ix], &value);
            }
        }
    }

    fn wrap_root_block_if_content_followed(&mut self) {
        if self.root_is_block && self.out.len() != self.root_block_end {
            self.root_is_block = false;
            self.out.insert_str(self.root_block_end, "</div>");
            self.out.insert_str(0, "<div data-ss-b=\"0\">");
        }
    }

    fn root_open(&self) -> String {
        let mut root = String::with_capacity(128);
        root.push_str("<div xmlns=\"");
        root.push_str(XHTML_NS);
        root.push_str("\" class=\"ss-body");
        let sources = [self.body_attrs, self.html_attrs];
        for attrs in sources.into_iter().flatten() {
            if let Some(class) = attrs
                .iter()
                .find(|attr| attr.name.eq_ignore_ascii_case("class"))
            {
                root.push(' ');
                push_escaped_attr(&mut root, &decode_attr(class.raw_value));
            }
        }
        root.push('"');
        let mut seen = 0u8;
        const COPIED: [&str; 5] = ["id", "lang", "xml:lang", "dir", "style"];
        for attrs in sources.into_iter().flatten() {
            for attr in attrs {
                let Some(ix) = COPIED
                    .iter()
                    .position(|name| attr.name.eq_ignore_ascii_case(name))
                else {
                    continue;
                };
                if seen & (1 << ix) != 0 {
                    continue;
                }
                seen |= 1 << ix;
                let value = decode_attr(attr.raw_value);
                if COPIED[ix] == "style" {
                    let style = sanitize_root_style(&value, &self.context.urls);
                    if !style.is_empty() {
                        push_attr(&mut root, "style", &style);
                    }
                } else {
                    push_attr(&mut root, COPIED[ix], &value);
                }
            }
        }
        if self.root_is_block {
            root.push_str(" data-ss-b=\"0\"");
        }
        root.push('>');
        root
    }
}

fn is_heading(local: &str) -> bool {
    let bytes = local.as_bytes();
    bytes.len() == 2 && bytes[0].eq_ignore_ascii_case(&b'h') && matches!(bytes[1], b'1'..=b'6')
}

pub(crate) fn sanitize<'e, 's>(
    events: &'e [Event<'s>],
    capacity: usize,
    layout: &Layout<'_>,
    context: &Context<'_>,
) -> Sanitized {
    let mut walk = Walk::new(capacity + capacity / 8 + 256, layout, context);
    for (ix, event) in events.iter().enumerate() {
        walk.block_edges(ix as u32);
        walk.event(event);
    }
    walk.block_edges(events.len() as u32);
    walk.wrap_root_block_if_content_followed();
    let root = walk.root_open();
    let mut xhtml = String::with_capacity(root.len() + walk.out.len() + 6);
    xhtml.push_str(&root);
    xhtml.push_str(&walk.out);
    xhtml.push_str("</div>");
    let kind = if walk.saw_text {
        ChapterKind::Text
    } else if walk.saw_image {
        ChapterKind::ImagePage
    } else {
        ChapterKind::Empty
    };
    Sanitized {
        xhtml,
        sheets: walk.sheets,
        heading: walk.heading,
        head_title: walk.head_title,
        kind,
    }
}

pub(crate) fn trace_output<'e, 's>(
    events: &'e [Event<'s>],
    capacity: usize,
    layout: &Layout<'_>,
    context: &Context<'_>,
) -> (String, Vec<Range<u32>>) {
    let mut walk = Walk::new(capacity + capacity / 8 + 256, layout, context);
    let mut ranges = Vec::with_capacity(events.len());
    for (ix, event) in events.iter().enumerate() {
        walk.block_edges(ix as u32);
        let start = walk.out.len() as u32;
        walk.event(event);
        ranges.push(start..walk.out.len() as u32);
    }
    walk.block_edges(events.len() as u32);
    (walk.out, ranges)
}

pub(crate) fn sanitize_svg_document(
    events: &[Event<'_>],
    capacity: usize,
    context: &Context<'_>,
) -> Option<String> {
    let layout = Layout {
        blocks: &[],
        ranges: &[],
        body_inner: Span::new(0, 0),
    };
    let mut walk = Walk::new(capacity, &layout, context);
    for event in events {
        walk.event(event);
    }
    walk.out.starts_with("<svg ").then_some(walk.out)
}

pub(crate) fn raw_image_page(url: Option<&str>) -> Sanitized {
    let mut xhtml = String::with_capacity(160);
    xhtml.push_str("<div xmlns=\"");
    xhtml.push_str(XHTML_NS);
    xhtml.push_str("\" class=\"ss-body\"><img data-ss-b=\"0\"");
    if let Some(url) = url {
        push_attr(&mut xhtml, "src", url);
    }
    xhtml.push_str(" alt=\"\"/></div>");
    Sanitized {
        xhtml,
        sheets: Vec::new(),
        heading: None,
        head_title: None,
        kind: if url.is_some() {
            ChapterKind::ImagePage
        } else {
            ChapterKind::Empty
        },
    }
}

#[cfg(test)]
mod tests;
