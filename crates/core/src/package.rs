use crate::chapter::{SourceEncoding, decode_source};
use crate::markup::{Event, Span, attr_value, decode_text, local_name, scan};
use crate::zip::{EntryName, ReadAt, ZipIndex};
use crate::{ChapterIx, Error, Limits};
use std::collections::HashSet;

const CONTAINER_PATH: &str = "META-INF/container.xml";

pub(crate) struct OpfDoc {
    pub(crate) source: String,
    pub(crate) text_form: TextForm,
    pub(crate) manifest: Vec<ManifestItem>,
    pub(crate) spine: Vec<SpineRef>,
    pub(crate) guide: Vec<GuideRef>,
    pub(crate) guide_span: Option<Span>,
    pub(crate) repaired: bool,
    pub(crate) manifest_end: u32,
    pub(crate) spine_end: u32,
    pub(crate) version: String,
    pub(crate) language: Option<String>,
    pub(crate) prefix: String,
    pub(crate) ids: Vec<String>,
    pub(crate) refines: Vec<String>,
    pub(crate) title: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) cover_id: Option<String>,
    pub(crate) ncx_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TextForm {
    Utf8 { bom: bool },
    Other,
}

impl TextForm {
    pub(crate) fn of(encoding: &SourceEncoding) -> TextForm {
        match encoding {
            SourceEncoding::Utf8 { bom } => TextForm::Utf8 { bom: *bom },
            SourceEncoding::Other { .. } => TextForm::Other,
        }
    }
}

pub(crate) struct ManifestItem {
    pub(crate) id: String,
    pub(crate) href: String,
    pub(crate) media_type: String,
    pub(crate) properties: String,
    pub(crate) fallback: Option<String>,
    pub(crate) span: Span,
}

pub(crate) struct SpineRef {
    pub(crate) idref: String,
    pub(crate) linear: bool,
    pub(crate) span: Span,
}

pub(crate) struct GuideRef {
    pub(crate) href: String,
    pub(crate) span: Span,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Metadata,
    Manifest,
    Spine,
    Guide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    None,
    Title,
    Creator,
    Language,
}

impl OpfDoc {
    pub(crate) fn parse(raw: Vec<u8>, limits: &Limits) -> Result<OpfDoc, Error> {
        let (source, encoding) = decode_source(raw);
        let mut doc = OpfDoc {
            text_form: TextForm::of(&encoding),
            manifest: Vec::new(),
            spine: Vec::new(),
            guide: Vec::new(),
            guide_span: None,
            repaired: false,
            manifest_end: 0,
            spine_end: 0,
            version: String::new(),
            language: None,
            prefix: String::new(),
            ids: Vec::new(),
            refines: Vec::new(),
            title: None,
            author: None,
            cover_id: None,
            ncx_id: None,
            source: String::new(),
        };
        let scanned = scan(&source, limits)?;
        let mut section = Section::None;
        let mut capture = Capture::None;
        let mut captured = String::new();
        let mut saw_spine = false;
        for event in &scanned.events {
            match event {
                Event::Open {
                    name,
                    attrs,
                    span,
                    self_closed,
                } => {
                    let local = local_name(name);
                    if let Some(id) = attr_value(attrs, "id") {
                        doc.ids.push(id);
                    }
                    if let Some(target) = attr_value(attrs, "refines") {
                        doc.refines.push(target.trim_start_matches('#').to_owned());
                    }
                    match (section, local) {
                        (Section::None, "package") => {
                            doc.version = attr_value(attrs, "version").unwrap_or_default();
                        }
                        (Section::None, "metadata") => section = Section::Metadata,
                        (Section::None, "manifest") if !self_closed => {
                            section = Section::Manifest;
                            doc.prefix = name[..name.len() - local.len()].to_owned();
                        }
                        (Section::None, "spine") => {
                            saw_spine = true;
                            doc.ncx_id = attr_value(attrs, "toc");
                            if !self_closed {
                                section = Section::Spine;
                            }
                        }
                        (Section::None, "guide") if !self_closed => {
                            section = Section::Guide;
                            doc.guide_span = Some(*span);
                        }
                        (Section::Metadata, "title") if doc.title.is_none() && !self_closed => {
                            capture = Capture::Title;
                        }
                        (Section::Metadata, "creator") if doc.author.is_none() && !self_closed => {
                            capture = Capture::Creator;
                        }
                        (Section::Metadata, "language")
                            if doc.language.is_none() && !self_closed =>
                        {
                            capture = Capture::Language;
                        }
                        (Section::Metadata, "meta")
                            if attr_value(attrs, "name").as_deref() == Some("cover") =>
                        {
                            doc.cover_id = attr_value(attrs, "content");
                        }
                        (Section::Manifest, "item") => {
                            let id = attr_value(attrs, "id");
                            let href = attr_value(attrs, "href");
                            if let (Some(id), Some(href)) = (id, href) {
                                doc.manifest.push(ManifestItem {
                                    id,
                                    href,
                                    media_type: attr_value(attrs, "media-type").unwrap_or_default(),
                                    properties: attr_value(attrs, "properties").unwrap_or_default(),
                                    fallback: attr_value(attrs, "fallback"),
                                    span: *span,
                                });
                            }
                        }
                        (Section::Spine, "itemref") => {
                            doc.spine.push(SpineRef {
                                idref: attr_value(attrs, "idref").unwrap_or_default(),
                                linear: attr_value(attrs, "linear").as_deref() != Some("no"),
                                span: *span,
                            });
                        }
                        (Section::Guide, "reference") => {
                            if let Some(href) = attr_value(attrs, "href") {
                                doc.guide.push(GuideRef { href, span: *span });
                            }
                        }
                        _ => {}
                    }
                }
                Event::Text { raw, .. } | Event::CData { raw, .. } => {
                    if capture != Capture::None {
                        captured.push_str(&decode_text(raw));
                    }
                }
                Event::Close { name, span } => {
                    let local = local_name(name);
                    match (section, local) {
                        (Section::Metadata, "metadata") => section = Section::None,
                        (Section::Guide, "guide") => {
                            section = Section::None;
                            if let Some(whole) = &mut doc.guide_span {
                                whole.end = span.end;
                            }
                        }
                        (Section::Manifest, "manifest") => {
                            section = Section::None;
                            doc.manifest_end = span.start;
                        }
                        (Section::Spine, "spine") => {
                            section = Section::None;
                            doc.spine_end = span.start;
                        }
                        (Section::Metadata, "title") if capture == Capture::Title => {
                            doc.title = Some(collapse_spaces(&captured));
                            capture = Capture::None;
                            captured.clear();
                        }
                        (Section::Metadata, "creator") if capture == Capture::Creator => {
                            doc.author = Some(collapse_spaces(&captured));
                            capture = Capture::None;
                            captured.clear();
                        }
                        (Section::Metadata, "language") if capture == Capture::Language => {
                            doc.language = Some(collapse_spaces(&captured));
                            capture = Capture::None;
                            captured.clear();
                        }
                        (Section::Manifest, "item") => {
                            if let Some(item) = doc.manifest.last_mut() {
                                item.span.end = span.end;
                            }
                        }
                        (Section::Spine, "itemref") => {
                            if let Some(row) = doc.spine.last_mut() {
                                row.span.end = span.end;
                            }
                        }
                        (Section::Guide, "reference") => {
                            if let Some(row) = doc.guide.last_mut() {
                                row.span.end = span.end;
                            }
                        }
                        _ => {}
                    }
                }
                Event::Other { .. } => {}
            }
        }
        if !saw_spine || doc.spine.is_empty() {
            return Err(Error::Book(
                "This book's package lists no chapters.".to_owned(),
            ));
        }
        doc.repaired = scanned.fixes.structural;
        drop(scanned);
        doc.source = source;
        Ok(doc)
    }
}

fn collapse_spaces(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn find_opf<R: ReadAt>(
    zip: &ZipIndex,
    source: &R,
    limits: &Limits,
) -> Result<EntryName, Error> {
    let container_name = EntryName::parse(CONTAINER_PATH)?;
    if let Some(entry) = zip.entry(&container_name) {
        let raw = zip.read_entry(source, entry, limits)?;
        let (text, _) = decode_source(raw);
        let scanned = scan(&text, limits)?;
        for event in &scanned.events {
            if let Event::Open { name, attrs, .. } = event
                && local_name(name) == "rootfile"
                && let Some(path) = attr_value(attrs, "full-path")
            {
                let target = EntryName::parse(&path)?;
                if zip.entry(&target).is_some() {
                    return Ok(target);
                }
                break;
            }
        }
    }
    sole_opf(zip)
}

fn sole_opf(zip: &ZipIndex) -> Result<EntryName, Error> {
    let mut candidates = zip
        .entries()
        .iter()
        .filter(|entry| entry.name.as_str().to_ascii_lowercase().ends_with(".opf"));
    match (candidates.next(), candidates.next()) {
        (Some(only), None) => Ok(only.name.clone()),
        _ => Err(Error::Book(
            "This book's container does not point to a package file.".to_owned(),
        )),
    }
}

pub(crate) fn resolve_href(opf_name: &EntryName, href: &str) -> Result<Option<EntryName>, Error> {
    let without_fragment = href.split(['#', '?']).next().unwrap_or_default();
    if without_fragment.is_empty() || has_url_scheme(without_fragment) {
        return Ok(None);
    }
    let decoded = percent_decode(without_fragment);
    let unsafe_href = || Error::Book("This book's package points outside the book.".to_owned());
    if decoded.starts_with('/') || decoded.contains('\\') {
        return Err(unsafe_href());
    }
    let base = opf_name
        .as_str()
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir);
    let mut segments: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop().ok_or_else(unsafe_href)?;
            }
            other => segments.push(other),
        }
    }
    EntryName::parse(&segments.join("/"))
        .map(Some)
        .map_err(|_| unsafe_href())
}

pub(crate) fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &byte in path.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub(crate) fn has_url_scheme(href: &str) -> bool {
    let Some((scheme, _)) = href.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

pub(crate) fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .filter(|_| bytes[at] == b'%')
            .and_then(|pair| {
                let high = (pair[0] as char).to_digit(16)?;
                let low = (pair[1] as char).to_digit(16)?;
                Some((high * 16 + low) as u8)
            });
        match hex {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

pub struct Toc {
    pub source: TocSource,
    pub entries: Vec<TocEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TocSource {
    Nav,
    Ncx,
    HeadingsPending,
}

pub struct TocEntry {
    pub label: String,
    pub target: Option<ChapterIx>,
    pub fragment: Option<String>,
    pub depth: u16,
}

fn tidy_label(label: &str) -> String {
    let tidy = collapse_spaces(label);
    if tidy.is_empty() {
        "Untitled".to_owned()
    } else {
        tidy
    }
}

fn nav_is_toc(attrs: &[crate::markup::Attr<'_>]) -> bool {
    attr_value(attrs, "type").is_some_and(|kinds| {
        kinds
            .split_ascii_whitespace()
            .any(|kind| kind.eq_ignore_ascii_case("toc"))
    })
}

pub(crate) struct TocDocEntry {
    pub(crate) label: String,
    pub(crate) href: Option<String>,
    pub(crate) depth: u16,
    pub(crate) parent: Option<u32>,
    pub(crate) kids: Vec<u32>,
    pub(crate) whole: Span,
    pub(crate) label_span: Option<Span>,
    pub(crate) nested: Option<Span>,
    pub(crate) play_order: Option<Span>,
}

pub(crate) struct OtherLink {
    pub(crate) href: String,
    pub(crate) whole: Span,
}

pub(crate) struct OtherList {
    pub(crate) whole: Span,
    pub(crate) links: Vec<OtherLink>,
}

pub(crate) struct TocDoc {
    pub(crate) kind: TocSource,
    pub(crate) source: String,
    pub(crate) text_form: TextForm,
    pub(crate) repaired: bool,
    pub(crate) entries: Vec<TocDocEntry>,
    pub(crate) list: Option<Span>,
    pub(crate) others: Vec<OtherList>,
    pub(crate) prefix: String,
    pub(crate) ids: HashSet<String>,
    pub(crate) depth_meta: Option<Span>,
}

#[derive(Default)]
struct Walk {
    entries: Vec<TocDocEntry>,
    labels: Vec<String>,
    list: Option<Span>,
    prefix: String,
}

fn nav_walk(events: &[Event<'_>], start: usize) -> (Walk, usize) {
    let mut walk = Walk::default();
    let mut items: Vec<usize> = Vec::new();
    let mut lists: Vec<(Option<usize>, u32, u32)> = Vec::new();
    let mut label_open: Option<(usize, u32, u32)> = None;
    let mut depth = 1u32;
    let mut at = start;
    while at < events.len() && depth > 0 {
        match &events[at] {
            Event::Open {
                name,
                attrs,
                self_closed,
                span,
            } => {
                let local = local_name(name).to_ascii_lowercase();
                if !self_closed {
                    depth += 1;
                }
                match local.as_str() {
                    "ol" | "ul" if !self_closed => {
                        if lists.is_empty() {
                            walk.prefix = name[..name.len() - local.len()].to_owned();
                        }
                        lists.push((items.last().copied(), span.start, span.end));
                    }
                    "li" if !self_closed => {
                        let parent = items.last().copied();
                        items.push(walk.entries.len());
                        walk.labels.push(String::new());
                        if let Some(parent) = parent {
                            let kid = walk.entries.len() as u32;
                            walk.entries[parent].kids.push(kid);
                        }
                        walk.entries.push(TocDocEntry {
                            label: String::new(),
                            href: None,
                            depth: (lists.len() as u16).saturating_sub(1),
                            parent: parent.map(|parent| parent as u32),
                            kids: Vec::new(),
                            whole: *span,
                            label_span: None,
                            nested: None,
                            play_order: None,
                        });
                    }
                    "a" => {
                        if let Some(&item) = items.last()
                            && walk.entries[item].href.is_none()
                        {
                            walk.entries[item].href =
                                attr_value(attrs, "href").filter(|href| !href.is_empty());
                        }
                    }
                    _ => {}
                }
                if matches!(local.as_str(), "a" | "span")
                    && !self_closed
                    && let Some(&item) = items.last()
                    && walk.entries[item].label_span.is_none()
                    && label_open.is_none()
                {
                    label_open = Some((item, depth, span.end));
                }
            }
            Event::Close { name, span } => {
                if let Some((item, opened, inner)) = label_open
                    && opened == depth
                {
                    walk.entries[item].label_span =
                        Some(Span::new(inner as usize, span.start as usize));
                    label_open = None;
                }
                depth -= 1;
                let local = local_name(name).to_ascii_lowercase();
                match local.as_str() {
                    "ol" | "ul" => {
                        if let Some((owner, start, inner)) = lists.pop() {
                            match owner {
                                Some(owner) => {
                                    walk.entries[owner].nested.get_or_insert(Span::new(
                                        start as usize,
                                        span.end as usize,
                                    ));
                                }
                                None => {
                                    walk.list.get_or_insert(Span::new(
                                        inner as usize,
                                        span.start as usize,
                                    ));
                                }
                            }
                        }
                    }
                    "li" => {
                        if let Some(item) = items.pop() {
                            walk.entries[item].whole.end = span.end;
                        }
                    }
                    _ => {}
                }
            }
            Event::Text { raw, .. } => {
                if let Some(&item) = items.last() {
                    walk.labels[item].push_str(&decode_text(raw));
                }
            }
            Event::CData { raw, .. } => {
                if let Some(&item) = items.last() {
                    walk.labels[item].push_str(raw);
                }
            }
            Event::Other { .. } => {}
        }
        at += 1;
    }
    (walk, at)
}

fn other_list_of(walk: Walk, whole: Span) -> OtherList {
    OtherList {
        whole,
        links: walk
            .entries
            .into_iter()
            .filter_map(|entry| {
                Some(OtherLink {
                    href: entry.href?,
                    whole: entry.whole,
                })
            })
            .collect(),
    }
}

fn nav_elements<'e>(events: &'e [Event<'e>]) -> impl Iterator<Item = (usize, bool, u32)> + use<'e> {
    events.iter().enumerate().filter_map(|(ix, event)| {
        if let Event::Open {
            name,
            attrs,
            self_closed: false,
            span,
        } = event
            && local_name(name).eq_ignore_ascii_case("nav")
        {
            Some((ix + 1, nav_is_toc(attrs), span.start))
        } else {
            None
        }
    })
}

fn close_span(events: &[Event<'_>], walked_to: usize) -> Option<Span> {
    match events.get(walked_to.checked_sub(1)?)? {
        Event::Close { span, .. } => Some(*span),
        _ => None,
    }
}

impl TocDoc {
    pub(crate) fn parse_nav(raw: Vec<u8>, limits: &Limits) -> Result<TocDoc, Error> {
        let (source, encoding) = decode_source(raw);
        let scanned = scan(&source, limits)?;
        let navs: Vec<(usize, bool, u32)> = nav_elements(&scanned.events).collect();
        let chosen =
            navs.iter()
                .position(|nav| nav.1)
                .or(if navs.is_empty() { None } else { Some(0) });
        let mut main = Walk::default();
        let mut others = Vec::new();
        let mut walked_to = 0;
        for (position, &(start, _, nav_start)) in navs.iter().enumerate() {
            let is_chosen = Some(position) == chosen;
            if start <= walked_to && !is_chosen {
                continue;
            }
            let (walk, end) = nav_walk(&scanned.events, start);
            walked_to = walked_to.max(end);
            if is_chosen {
                main = walk;
            } else {
                let close = close_span(&scanned.events, end).map_or(nav_start, |span| span.end);
                others.push(other_list_of(
                    walk,
                    Span::new(nav_start as usize, close as usize),
                ));
            }
        }
        let doc = finish_walk(
            main,
            TocSource::Nav,
            others,
            HashSet::new(),
            scanned.fixes.structural,
        );
        drop(scanned);
        Ok(TocDoc {
            source,
            text_form: TextForm::of(&encoding),
            ..doc
        })
    }

    pub(crate) fn parse_ncx(raw: Vec<u8>, limits: &Limits) -> Result<TocDoc, Error> {
        let (source, encoding) = decode_source(raw);
        let scanned = scan(&source, limits)?;
        let mut walk = Walk::default();
        let mut points: Vec<usize> = Vec::new();
        let mut in_map = false;
        let mut label_depth = 0u32;
        let mut text_open: Option<u32> = None;
        let mut ids = HashSet::new();
        let mut others: Vec<OtherList> = Vec::new();
        let mut other_open: Option<OtherList> = None;
        let mut target_open: Option<(Option<String>, Span)> = None;
        let mut depth_meta: Option<Span> = None;
        for event in &scanned.events {
            match event {
                Event::Open {
                    name,
                    attrs,
                    self_closed,
                    span,
                } => {
                    if let Some(id) = attr_value(attrs, "id") {
                        ids.insert(id);
                    }
                    match local_name(name) {
                        "navMap" if !self_closed => {
                            in_map = true;
                            walk.prefix = name[..name.len() - "navMap".len()].to_owned();
                            walk.list
                                .get_or_insert(Span::new(span.end as usize, span.end as usize));
                        }
                        "navPoint" if in_map && !self_closed => {
                            let parent = points.last().copied();
                            walk.labels.push(String::new());
                            if let Some(parent) = parent {
                                let kid = walk.entries.len() as u32;
                                walk.entries[parent].kids.push(kid);
                            }
                            walk.entries.push(TocDocEntry {
                                label: String::new(),
                                href: None,
                                depth: u16::try_from(points.len()).unwrap_or(u16::MAX),
                                parent: parent.map(|parent| parent as u32),
                                kids: Vec::new(),
                                whole: *span,
                                label_span: None,
                                nested: None,
                                play_order: attrs
                                    .iter()
                                    .find(|attr| local_name(attr.name) == "playOrder")
                                    .map(|attr| attr.value_span),
                            });
                            points.push(walk.entries.len() - 1);
                        }
                        "meta"
                            if !in_map
                                && depth_meta.is_none()
                                && attr_value(attrs, "name").as_deref() == Some("dtb:depth") =>
                        {
                            depth_meta = attrs
                                .iter()
                                .find(|attr| local_name(attr.name) == "content")
                                .map(|attr| attr.value_span);
                        }
                        "navLabel" if in_map && !self_closed => label_depth += 1,
                        "text" if in_map && label_depth > 0 && !self_closed => {
                            if let Some(&point) = points.last()
                                && walk.entries[point].label_span.is_none()
                            {
                                text_open = Some(span.end);
                            }
                        }
                        "content" if in_map => {
                            if let Some(&point) = points.last()
                                && walk.entries[point].href.is_none()
                            {
                                walk.entries[point].href =
                                    attr_value(attrs, "src").filter(|src| !src.is_empty());
                            }
                        }
                        "content" => {
                            if let Some((href, _)) = &mut target_open {
                                *href = attr_value(attrs, "src");
                            }
                        }
                        "pageList" | "navList" if !self_closed => {
                            other_open = Some(OtherList {
                                whole: *span,
                                links: Vec::new(),
                            });
                        }
                        "pageTarget" | "navTarget" if !self_closed && other_open.is_some() => {
                            target_open = Some((None, *span));
                        }
                        _ => {}
                    }
                }
                Event::Close { name, span } => match local_name(name) {
                    "navMap" => {
                        in_map = false;
                        if let Some(list) = &mut walk.list {
                            list.end = span.start;
                        }
                    }
                    "navPoint" if in_map => {
                        if let Some(point) = points.pop() {
                            walk.entries[point].whole.end = span.end;
                        }
                    }
                    "navLabel" if in_map => label_depth = label_depth.saturating_sub(1),
                    "text" if in_map => {
                        if let (Some(inner), Some(&point)) = (text_open.take(), points.last()) {
                            walk.entries[point].label_span =
                                Some(Span::new(inner as usize, span.start as usize));
                        }
                    }
                    "pageTarget" | "navTarget" => {
                        if let (Some((href, start)), Some(open)) =
                            (target_open.take(), &mut other_open)
                            && let Some(href) = href
                        {
                            open.links.push(OtherLink {
                                href,
                                whole: Span::new(start.start as usize, span.end as usize),
                            });
                        }
                    }
                    "pageList" | "navList" => {
                        if let Some(mut open) = other_open.take() {
                            open.whole.end = span.end;
                            others.push(open);
                        }
                    }
                    _ => {}
                },
                Event::Text { raw, .. } if label_depth > 0 => {
                    if let Some(&point) = points.last() {
                        walk.labels[point].push_str(&decode_text(raw));
                    }
                }
                Event::CData { raw, .. } if label_depth > 0 => {
                    if let Some(&point) = points.last() {
                        walk.labels[point].push_str(raw);
                    }
                }
                _ => {}
            }
        }
        let doc = finish_walk(walk, TocSource::Ncx, others, ids, scanned.fixes.structural);
        drop(scanned);
        Ok(TocDoc {
            source,
            text_form: TextForm::of(&encoding),
            depth_meta,
            ..doc
        })
    }
}

fn finish_walk(
    mut walk: Walk,
    kind: TocSource,
    others: Vec<OtherList>,
    ids: HashSet<String>,
    repaired: bool,
) -> TocDoc {
    for (entry, label) in walk.entries.iter_mut().zip(&walk.labels) {
        entry.label = tidy_label(label);
    }
    TocDoc {
        kind,
        source: String::new(),
        text_form: TextForm::Utf8 { bom: false },
        repaired,
        entries: walk.entries,
        list: walk.list,
        others,
        prefix: walk.prefix,
        ids,
        depth_meta: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn parse(doc: &str) -> TocDoc {
        TocDoc::parse_nav(doc.as_bytes().to_vec(), &crate::DEFAULT_LIMITS).unwrap()
    }

    fn fastest_parse(navs: usize, links: usize) -> Duration {
        let mut doc = "<html><body>".to_owned();
        doc.push_str(&"<nav>".repeat(navs));
        doc.push_str(&"<a/>".repeat(links));
        (0..3)
            .map(|_| {
                let started = Instant::now();
                parse(&doc);
                started.elapsed()
            })
            .min()
            .unwrap()
    }

    #[test]
    fn nested_nav_elements_do_not_multiply_the_table_of_contents_walk() {
        let one = fastest_parse(1, 100_000);
        let nested = fastest_parse(120, 100_000);
        assert!(
            nested < one * 4,
            "120 nested navs took {nested:?} against {one:?} for one"
        );
    }

    #[test]
    fn a_contents_nav_nested_in_another_nav_is_still_the_one_read() {
        let toc = parse(concat!(
            "<html xmlns:epub=\"http://www.idpf.org/2007/ops\"><body>",
            "<nav><ol><li><a href=\"cover.xhtml\">Cover</a></li>",
            "<nav epub:type=\"toc\"><ol><li><a href=\"one.xhtml\">One</a></li></ol></nav>",
            "</ol></nav>",
            "<nav epub:type=\"landmarks\"><ol><li><a href=\"two.xhtml\">Two</a></li></ol></nav>",
            "</body></html>",
        ));
        let hrefs: Vec<_> = toc
            .entries
            .iter()
            .map(|entry| entry.href.as_deref())
            .collect();
        assert_eq!(hrefs, [Some("one.xhtml")]);
        assert_eq!(toc.entries[0].label, "One");
        let other_hrefs: Vec<Vec<&str>> = toc
            .others
            .iter()
            .map(|list| list.links.iter().map(|link| link.href.as_str()).collect())
            .collect();
        assert_eq!(
            other_hrefs,
            [vec!["cover.xhtml", "one.xhtml"], vec!["two.xhtml"]]
        );
    }
}
