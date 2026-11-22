use crate::CancelToken;
use crate::canon::Traced;
use crate::cover::{CoverRef, find_cover, has_property};
use crate::chapter::{BlockLayout, ChapterModel, VisibleText, decode_and_model, decode_source};
use crate::css::sanitize_stylesheet;
use crate::edit::{BlockEdits, ChapterPlan, plan_chapter};
use crate::manage::{Doc, Package, check_package, refusal_for};
use crate::markup::scan;
use crate::package::{
    OpfDoc, Toc, TocDoc, TocDocEntry, TocEntry, TocSource, find_opf, percent_decode, resolve_href,
};
use crate::sanitize::{
    ChapterKind, Context, Href, Layout, ResourceUrls, Sanitized, Sheet, UrlMap, classify_href,
    entry_of_scheme_url, raw_image_page, sanitize, sanitize_svg_document,
};
use crate::zip::{EntryName, ReadAt, ZipEntry, ZipIndex};
use crate::{ChapterIx, ChapterRev, DomRange, Error, Fingerprint, Limits, VisRange};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::ops::Range;
use std::sync::OnceLock;

const MAX_CHAPTER_CSS_BYTES: u64 = 32 << 20;
const MAX_TOC_FOR_START: u64 = 2 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Served {
    Raw(&'static str),
    Svg,
    Stylesheet,
}

fn served_as(media_type: &str) -> Option<Served> {
    let lowered = media_type.trim().to_ascii_lowercase();
    let raw = match lowered.as_str() {
        "image/svg+xml" => return Some(Served::Svg),
        "text/css" => return Some(Served::Stylesheet),
        "image/png" => "image/png",
        "image/jpeg" | "image/jpg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/avif" => "image/avif",
        "image/bmp" => "image/bmp",
        "font/otf" | "application/vnd.ms-opentype" | "application/x-font-opentype" => "font/otf",
        "font/ttf"
        | "application/x-font-ttf"
        | "application/x-font-truetype"
        | "application/font-sfnt" => "font/ttf",
        "font/woff" | "application/font-woff" => "font/woff",
        "font/woff2" | "application/font-woff2" => "font/woff2",
        "audio/mpeg" | "audio/mp3" => "audio/mpeg",
        "audio/mp4" => "audio/mp4",
        "audio/ogg" => "audio/ogg",
        "video/mp4" => "video/mp4",
        "video/webm" => "video/webm",
        _ => return None,
    };
    Some(Served::Raw(raw))
}

pub type PlannedBytes = (Vec<u8>, Vec<String>);

pub struct Reader<R: ReadAt = File> {
    source: R,
    limits: Limits,
    zip: ZipIndex,
    opf_name: EntryName,
    opf: OpfDoc,
    rows: Vec<ChapterRow>,
    spine_items: Vec<Option<u32>>,
    summary: BookSummary,
    session: u64,
    resources: HashMap<EntryName, Served>,
    nav_entry: Option<EntryName>,
    ncx_entry: Option<EntryName>,
    spine_by_entry: OnceLock<HashMap<EntryName, u32>>,
    toc: OnceLock<Toc>,
}

#[derive(Clone)]
pub struct BookSummary {
    pub title: String,
    pub author: Option<String>,
    pub fingerprint: Fingerprint,
    pub linear_chapters: u32,
    pub inflated_spine_bytes: u64,
    pub cover: Option<CoverRef>,
}

pub struct ChapterRow {
    pub ix: ChapterIx,
    pub href: Option<EntryName>,
    pub linear_ordinal: Option<u32>,
    pub is_nav: bool,
    pub rev: Option<ChapterRev>,
}

pub enum Extracted {
    Document(Vec<u8>),
    Image { media_type: String, bytes: Vec<u8> },
}

pub struct RenderedChapter {
    pub rev: ChapterRev,
    pub title: Option<String>,
    pub kind: ChapterKind,
    pub xhtml: String,
    pub css: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(PartialEq, Eq, Debug)]
pub enum LinkTarget {
    Chapter {
        ix: ChapterIx,
        fragment: Option<String>,
    },
    External {
        url: String,
    },
    Blocked,
}

pub struct Resource {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl<R: ReadAt> Reader<R> {
    pub fn open(source: R, limits: Limits) -> Result<Self, Error> {
        let zip = ZipIndex::parse(&source, &limits)?;
        let opf_name = find_opf(&zip, &source, &limits)?;
        let opf_entry = zip
            .entry(&opf_name)
            .expect("find_opf returns an existing entry");
        let opf_bytes = zip.read_entry(&source, opf_entry, &limits)?;
        let mut hasher = Sha256::new();
        hasher.update(&opf_bytes);
        let opf = OpfDoc::parse(opf_bytes, &limits)?;
        hasher.update((opf.spine.len() as u64).to_le_bytes());
        let fingerprint = Fingerprint(hasher.finalize().into());

        let mut hrefs = Vec::with_capacity(opf.manifest.len());
        let mut by_id = HashMap::with_capacity(opf.manifest.len());
        for (ix, item) in opf.manifest.iter().enumerate() {
            hrefs.push(resolve_href(&opf_name, &item.href)?);
            by_id.entry(item.id.as_str()).or_insert(ix);
        }

        let mut rows = Vec::with_capacity(opf.spine.len());
        let mut spine_items = Vec::with_capacity(opf.spine.len());
        let mut next_ordinal = 0u32;
        let mut inflated_spine_bytes = 0u64;
        for (ix, spine_ref) in opf.spine.iter().enumerate() {
            let item = by_id.get(spine_ref.idref.as_str()).copied();
            let is_nav =
                item.is_some_and(|item| has_property(&opf.manifest[item].properties, "nav"));
            let href = item.and_then(|item| hrefs[item].clone());
            let entry = href.as_ref().and_then(|name| zip.entry(name));
            if let Some(entry) = entry {
                inflated_spine_bytes = inflated_spine_bytes.saturating_add(entry.inflated);
            }
            let linear_ordinal = (spine_ref.linear && !is_nav).then(|| {
                next_ordinal += 1;
                next_ordinal
            });
            rows.push(ChapterRow {
                ix: ChapterIx(ix as u32),
                href,
                linear_ordinal,
                is_nav,
                rev: entry.map(|entry| ChapterRev {
                    crc32: entry.crc32,
                    inflated_len: u32::try_from(entry.inflated).unwrap_or(u32::MAX),
                }),
            });
            spine_items.push(item.map(|item| item as u32));
        }

        let cover = find_cover(&opf, &hrefs, &by_id, &zip);
        let mut resources = HashMap::new();
        let mut nav_entry = None;
        let mut ncx_entry = None;
        for (item, href) in opf.manifest.iter().zip(&hrefs) {
            let Some(href) = href else { continue };
            if zip.entry(href).is_none() {
                continue;
            }
            if let Some(served) = served_as(&item.media_type) {
                resources.entry(href.clone()).or_insert(served);
            }
            if nav_entry.is_none() && has_property(&item.properties, "nav") {
                nav_entry = Some(href.clone());
            }
            let named_ncx = opf.ncx_id.as_deref() == Some(item.id.as_str());
            if named_ncx
                || (ncx_entry.is_none()
                    && item
                        .media_type
                        .trim()
                        .eq_ignore_ascii_case("application/x-dtbncx+xml"))
            {
                ncx_entry = Some(href.clone());
            }
        }
        let summary = BookSummary {
            title: opf
                .title
                .clone()
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "Untitled".to_owned()),
            author: opf.author.clone().filter(|author| !author.is_empty()),
            fingerprint,
            linear_chapters: next_ordinal,
            inflated_spine_bytes,
            cover,
        };
        Ok(Reader {
            source,
            limits,
            zip,
            opf_name,
            opf,
            rows,
            spine_items,
            summary,
            session: RandomState::new().build_hasher().finish(),
            resources,
            nav_entry,
            ncx_entry,
            spine_by_entry: OnceLock::new(),
            toc: OnceLock::new(),
        })
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    pub(crate) fn zip(&self) -> &ZipIndex {
        &self.zip
    }

    pub(crate) fn source(&self) -> &R {
        &self.source
    }

    pub fn language(&self) -> Option<&str> {
        self.opf.language.as_deref()
    }

    pub fn summary(&self) -> &BookSummary {
        &self.summary
    }

    pub fn rows(&self) -> &[ChapterRow] {
        &self.rows
    }

    pub(crate) fn entry_names(&self) -> BTreeSet<EntryName> {
        self.zip
            .entries()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub(crate) fn package<'r>(
        &'r self,
        docs: &'r [(EntryName, TocDoc)],
        names: &'r BTreeSet<EntryName>,
        reserved_names: Vec<EntryName>,
    ) -> Package<'r> {
        let mut built: Vec<Doc<'r>> = docs
            .iter()
            .map(|(name, toc)| Doc::new(name, toc, self.spine_map()))
            .collect();
        let primary = built.iter().position(Doc::has_rows);
        if let Some(at) = primary {
            built.swap(0, at);
        }
        Package {
            opf_name: &self.opf_name,
            opf: &self.opf,
            rows: &self.rows,
            spine_items: &self.spine_items,
            item_files: self
                .opf
                .manifest
                .iter()
                .map(|item| resolve_href(&self.opf_name, &item.href).ok().flatten())
                .collect(),
            docs: built,
            has_primary: primary.is_some(),
            names,
            reserved_names,
            limits: self.limits,
        }
    }

    pub(crate) fn opf_entry(&self) -> &EntryName {
        &self.opf_name
    }

    pub(crate) fn nav_name(&self) -> Option<&EntryName> {
        self.nav_entry.as_ref()
    }

    pub(crate) fn ncx_name(&self) -> Option<&EntryName> {
        self.ncx_entry.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn manifest_count(&self) -> usize {
        self.opf.manifest.len()
    }

    #[cfg(test)]
    pub(crate) fn problems_for_test(&self) -> BTreeSet<crate::manage::Problem> {
        let docs: Vec<_> = self.toc_docs().unwrap().into_iter().flatten().collect();
        check_package(
            &self.opf_name,
            self.opf.source.as_bytes().to_vec(),
            docs.iter()
                .map(|(name, doc)| (name, doc.kind, doc.source.as_bytes().to_vec())),
            &self.entry_names(),
            &self.limits,
        )
    }

    #[cfg(test)]
    pub(crate) fn check_no_new_problems(&self, before: &Reader<R>, context: &str) {
        let (old, new) = (before.problems_for_test(), self.problems_for_test());
        let fresh: Vec<_> = new.difference(&old).map(|problem| problem.kind).collect();
        assert!(fresh.is_empty(), "{context}: check_package found {fresh:?}");
    }

    #[cfg(test)]
    pub(crate) fn check_whole_package_for_test(&self) {
        let problems = self.problems_for_test();
        assert!(
            problems.iter().all(|problem| problem.kind == "nav-count"),
            "{:?}",
            problems
                .iter()
                .map(|problem| problem.kind)
                .collect::<Vec<_>>()
        );
    }

    pub(crate) fn check_whole_package(&self) -> Result<(), Error> {
        let docs: Vec<_> = self.toc_docs()?.into_iter().flatten().collect();
        let problems = check_package(
            &self.opf_name,
            self.opf.source.as_bytes().to_vec(),
            docs.iter()
                .map(|(name, doc)| (name, doc.kind, doc.source.as_bytes().to_vec())),
            &self.entry_names(),
            &self.limits,
        );
        match problems.first() {
            Some(problem) => Err(Error::WouldBreak(refusal_for(problem))),
            None => Ok(()),
        }
    }

    pub(crate) fn entry_of(&self, ix: ChapterIx) -> Result<&ZipEntry, Error> {
        let row = self.rows.get(ix.0 as usize).ok_or_else(|| {
            Error::Book(format!("This book has no chapter {}.", u64::from(ix.0) + 1))
        })?;
        let number = u64::from(ix.0) + 1;
        let href = row.href.as_ref().ok_or_else(|| {
            Error::Book(format!(
                "Chapter {number} is listed in the book but points to nothing."
            ))
        })?;
        self.zip.entry(href).ok_or_else(|| {
            Error::Book(format!(
                "Chapter {number} is listed in the book but its file is missing."
            ))
        })
    }

    fn media_type(&self, ix: ChapterIx) -> Option<&str> {
        let item = (*self.spine_items.get(ix.0 as usize)?)?;
        Some(self.opf.manifest[item as usize].media_type.as_str())
    }

    pub fn extract(&self, ix: ChapterIx) -> Result<Extracted, Error> {
        let entry = self.entry_of(ix)?;
        let bytes = self.zip.read_entry(&self.source, entry, &self.limits)?;
        match self.media_type(ix) {
            Some(media_type) if media_type.starts_with("image/") => Ok(Extracted::Image {
                media_type: media_type.to_owned(),
                bytes,
            }),
            _ => Ok(Extracted::Document(bytes)),
        }
    }

    pub(crate) fn model(&self, ix: ChapterIx) -> Result<Option<ChapterModel>, Error> {
        match self.extract(ix)? {
            Extracted::Document(bytes) => decode_and_model(bytes, &self.limits).map(Some),
            Extracted::Image { .. } => Ok(None),
        }
    }

    pub fn visible_text(&self, ix: ChapterIx) -> Result<VisibleText, Error> {
        Ok(self
            .model(ix)?
            .map_or_else(VisibleText::empty, |model| model.text))
    }

    pub fn locate(&self, ix: ChapterIx, range: VisRange) -> Result<DomRange, Error> {
        Ok(self.visible_text(ix)?.to_dom(range))
    }

    pub fn locate_many(&self, ix: ChapterIx, ranges: &[VisRange]) -> Result<Vec<DomRange>, Error> {
        let visible = self.visible_text(ix)?;
        Ok(ranges.iter().map(|range| visible.to_dom(*range)).collect())
    }

    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }

    fn spine_map(&self) -> &HashMap<EntryName, u32> {
        self.spine_by_entry.get_or_init(|| {
            let mut map = HashMap::with_capacity(self.rows.len());
            for row in &self.rows {
                if let Some(href) = &row.href {
                    map.entry(href.clone()).or_insert(row.ix.0);
                }
            }
            map
        })
    }

    pub fn chapter_of_entry(&self, entry: &str) -> Option<ChapterIx> {
        let name = EntryName::parse(entry).ok()?;
        self.spine_map().get(&name).map(|ix| ChapterIx(*ix))
    }

    fn is_serveable(&self, entry: &EntryName) -> bool {
        matches!(
            self.resources.get(entry),
            Some(Served::Raw(_) | Served::Svg)
        )
    }

    fn is_stylesheet(&self, entry: &EntryName) -> bool {
        self.resources.get(entry) == Some(&Served::Stylesheet)
    }

    fn sanitized(&self, ix: ChapterIx) -> Result<(Sanitized, ChapterRev, bool), Error> {
        let entry = self.entry_of(ix)?;
        let rev = ChapterRev {
            crc32: entry.crc32,
            inflated_len: u32::try_from(entry.inflated).unwrap_or(u32::MAX),
        };
        let map = UrlMap::new(self.session, entry.name.clone());
        let is_image = self
            .media_type(ix)
            .is_some_and(|media_type| media_type.trim().starts_with("image/"));
        if is_image {
            let url = self
                .is_serveable(&entry.name)
                .then(|| map.entry_url(&entry.name));
            return Ok((raw_image_page(url.as_deref()), rev, false));
        }
        let raw = self.zip.read_entry(&self.source, entry, &self.limits)?;
        let (source, _) = decode_source(raw);
        let scanned = scan(&source, &self.limits)?;
        let (layout, ranges) = BlockLayout::build(&scanned.events, &source);
        let serveable = |entry: &EntryName| self.is_serveable(entry);
        let is_stylesheet = |entry: &EntryName| self.is_stylesheet(entry);
        let context = Context {
            urls: ResourceUrls {
                map,
                serveable: &serveable,
            },
            is_stylesheet: &is_stylesheet,
        };
        let sanitized = sanitize(
            &scanned.events,
            source.len(),
            &Layout {
                blocks: &layout.blocks,
                ranges: &ranges,
                body_inner: layout.body_inner,
            },
            &context,
        );
        Ok((sanitized, rev, scanned.fixes.structural))
    }

    pub fn render(&self, ix: ChapterIx) -> Result<RenderedChapter, Error> {
        let (sanitized, rev, repaired) = self.sanitized(ix)?;
        let mut warnings = Vec::new();
        if repaired {
            warnings
                .push("This chapter's markup is damaged and was repaired for display.".to_owned());
        }
        let serveable = |entry: &EntryName| self.is_serveable(entry);
        let mut css = Vec::with_capacity(sanitized.sheets.len());
        let mut linked: HashSet<EntryName> = HashSet::new();
        let mut budget = MAX_CHAPTER_CSS_BYTES;
        let mut dropped_sheets = 0u32;
        for sheet in sanitized.sheets {
            match sheet {
                Sheet::Inline(text) => css.push(text),
                Sheet::Linked(entry) => {
                    if !linked.insert(entry.clone()) {
                        continue;
                    }
                    let urls = ResourceUrls {
                        map: UrlMap::new(self.session, entry.clone()),
                        serveable: &serveable,
                    };
                    match self.read_sheet(&entry, &mut budget) {
                        Some(text) => {
                            let clean = sanitize_stylesheet(&text, &urls);
                            if !clean.is_empty() {
                                css.push(clean);
                            }
                        }
                        None => dropped_sheets += 1,
                    }
                }
            }
        }
        if dropped_sheets > 0 {
            warnings.push(format!(
                "{dropped_sheets} of this chapter's stylesheets could not be loaded."
            ));
        }
        Ok(RenderedChapter {
            rev,
            title: sanitized.heading.or(sanitized.head_title),
            kind: sanitized.kind,
            xhtml: sanitized.xhtml,
            css,
            warnings,
        })
    }

    fn read_sheet(&self, name: &EntryName, budget: &mut u64) -> Option<String> {
        let entry = self.zip.entry(name)?;
        if entry.inflated > *budget {
            return None;
        }
        let raw = self
            .zip
            .read_entry(&self.source, entry, &self.limits)
            .ok()?;
        *budget -= raw.len() as u64;
        let text = String::from_utf8_lossy(&raw);
        Some(text.strip_prefix('\u{FEFF}').unwrap_or(&text).to_owned())
    }

    pub fn resource(&self, entry: &str) -> Result<Resource, Error> {
        let unavailable = || Error::Book("This book does not offer that resource.".to_owned());
        let name = EntryName::parse(entry).map_err(|_| unavailable())?;
        let served = *self.resources.get(&name).ok_or_else(unavailable)?;
        let zip_entry = self.zip.entry(&name).ok_or_else(unavailable)?;
        match served {
            Served::Stylesheet => Err(unavailable()),
            Served::Raw(media_type) => Ok(Resource {
                media_type: media_type.to_owned(),
                bytes: self.zip.read_entry(&self.source, zip_entry, &self.limits)?,
            }),
            Served::Svg => {
                let raw = self.zip.read_entry(&self.source, zip_entry, &self.limits)?;
                let (source, _) = decode_source(raw);
                let scanned = scan(&source, &self.limits)?;
                let serveable = |entry: &EntryName| self.is_serveable(entry);
                let refuse_sheets = |_: &EntryName| false;
                let context = Context {
                    urls: ResourceUrls {
                        map: UrlMap::new(self.session, name.clone()),
                        serveable: &serveable,
                    },
                    is_stylesheet: &refuse_sheets,
                };
                let clean = sanitize_svg_document(&scanned.events, source.len(), &context)
                    .ok_or_else(unavailable)?;
                Ok(Resource {
                    media_type: "image/svg+xml".to_owned(),
                    bytes: clean.into_bytes(),
                })
            }
        }
    }

    pub fn entry_of_url(&self, url: &str) -> Option<String> {
        entry_of_scheme_url(self.session, url).map(|entry| entry.as_str().to_owned())
    }

    pub fn link(&self, from: ChapterIx, href: &str) -> LinkTarget {
        let relative = match classify_href(href) {
            Href::External(url) => return LinkTarget::External { url },
            Href::Blocked => return LinkTarget::Blocked,
            Href::Relative(relative) => relative,
        };
        let Some(base) = self
            .rows
            .get(from.0 as usize)
            .and_then(|row| row.href.as_ref())
        else {
            return LinkTarget::Blocked;
        };
        self.resolve_target(base, &relative)
            .map_or(LinkTarget::Blocked, |(ix, fragment)| LinkTarget::Chapter {
                ix,
                fragment,
            })
    }

    fn resolve_target(&self, base: &EntryName, href: &str) -> Option<(ChapterIx, Option<String>)> {
        let (path, fragment) = match href.split_once('#') {
            Some((path, fragment)) => (path, Some(percent_decode(fragment))),
            None => (href, None),
        };
        let path = path.split('?').next().unwrap_or_default();
        let fragment = fragment.filter(|fragment| !fragment.is_empty());
        let entry = if path.is_empty() {
            base.clone()
        } else {
            resolve_href(base, path).ok()??
        };
        let ix = *self.spine_map().get(&entry)?;
        Some((ChapterIx(ix), fragment))
    }

    pub fn toc(&self) -> Result<&Toc, Error> {
        if let Some(toc) = self.toc.get() {
            return Ok(toc);
        }
        let toc = self.build_toc()?;
        Ok(self.toc.get_or_init(|| toc))
    }

    #[doc(hidden)]
    pub fn toc_is_parsed(&self) -> bool {
        self.toc.get().is_some()
    }

    pub(crate) fn toc_docs(&self) -> Result<[Option<(EntryName, TocDoc)>; 2], Error> {
        type Parse = fn(Vec<u8>, &Limits) -> Result<TocDoc, Error>;
        let documents: [(Option<&EntryName>, Parse); 2] = [
            (self.nav_entry.as_ref(), TocDoc::parse_nav),
            (self.ncx_entry.as_ref(), TocDoc::parse_ncx),
        ];
        let mut docs = [None, None];
        for (slot, (name, parse)) in docs.iter_mut().zip(documents) {
            let Some(name) = name else { continue };
            let Some(entry) = self.zip.entry(name) else {
                continue;
            };
            let raw = match self.zip.read_entry(&self.source, entry, &self.limits) {
                Ok(raw) => raw,
                Err(error @ Error::Io(_)) => return Err(error),
                Err(_) => continue,
            };
            match parse(raw, &self.limits) {
                Ok(doc) => *slot = Some((name.clone(), doc)),
                Err(error @ Error::Io(_)) => return Err(error),
                Err(_) => {}
            }
        }
        Ok(docs)
    }

    fn build_toc(&self) -> Result<Toc, Error> {
        for (name, doc) in self.toc_docs()?.iter().flatten() {
            let entries: Vec<TocEntry> = doc
                .entries
                .iter()
                .map(|entry| self.toc_entry(name, entry))
                .collect();
            if entries.iter().any(|entry| entry.target.is_some()) {
                return Ok(Toc {
                    source: doc.kind,
                    entries,
                });
            }
        }
        let entries = self
            .rows
            .iter()
            .filter_map(|row| {
                let ordinal = row.linear_ordinal?;
                Some(TocEntry {
                    label: format!("Chapter {ordinal}"),
                    target: Some(row.ix),
                    fragment: None,
                    depth: 0,
                })
            })
            .collect();
        Ok(Toc {
            source: TocSource::HeadingsPending,
            entries,
        })
    }

    fn toc_entry(&self, document: &EntryName, raw: &TocDocEntry) -> TocEntry {
        let resolved = raw
            .href
            .as_deref()
            .and_then(|href| match classify_href(href) {
                Href::Relative(relative) => self.resolve_target(document, &relative),
                Href::External(_) | Href::Blocked => None,
            });
        let (target, fragment) =
            resolved.map_or((None, None), |(ix, fragment)| (Some(ix), fragment));
        TocEntry {
            label: raw.label.clone(),
            target,
            fragment,
            depth: raw.depth,
        }
    }

    pub(crate) fn keep_session(&mut self, session: u64) {
        self.session = session;
    }

    pub(crate) fn rev_of(&self, ix: ChapterIx) -> Result<ChapterRev, Error> {
        let entry = self.entry_of(ix)?;
        Ok(ChapterRev {
            crc32: entry.crc32,
            inflated_len: u32::try_from(entry.inflated).unwrap_or(u32::MAX),
        })
    }

    pub(crate) fn plan_save(
        &self,
        ix: ChapterIx,
        edits: &BlockEdits,
    ) -> Result<(EntryName, ChapterPlan), Error> {
        let entry = self.entry_of(ix)?;
        if self.rev_of(ix)? != edits.base {
            return Err(Error::PlanStale);
        }
        let model = self.model(ix)?.ok_or_else(|| {
            Error::NotEditable("This chapter is an image, so it has no text to edit.".to_owned())
        })?;
        let plan = self.with_context(&entry.name, |context| {
            plan_chapter(&model, edits, context, &self.limits)
        })?;
        Ok((entry.name.clone(), plan))
    }

    #[doc(hidden)]
    pub fn plan_preview(
        &self,
        ix: ChapterIx,
        edits: &BlockEdits,
    ) -> Result<Option<PlannedBytes>, Error> {
        Ok(match self.plan_save(ix, edits)?.1 {
            ChapterPlan::Unchanged { .. } => None,
            ChapterPlan::Write(planned) => Some((planned.bytes, planned.warnings)),
        })
    }

    pub(crate) fn display_blocks(&self, ix: ChapterIx) -> Result<Vec<String>, Error> {
        let entry = self.entry_of(ix)?;
        let Some(model) = self.model(ix)? else {
            return Ok(Vec::new());
        };
        let scanned = scan(&model.source, &self.limits)?;
        let (layout, ranges) = BlockLayout::build(&scanned.events, &model.source);
        Ok(self.with_context(&entry.name, |context| {
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
            ranges
                .iter()
                .map(|range| traced.display(range).to_owned())
                .collect()
        }))
    }

    fn with_context<T>(&self, base: &EntryName, work: impl FnOnce(&Context<'_>) -> T) -> T {
        let serveable = |entry: &EntryName| self.is_serveable(entry);
        let is_stylesheet = |entry: &EntryName| self.is_stylesheet(entry);
        work(&Context {
            urls: ResourceUrls {
                map: UrlMap::new(self.session, base.clone()),
                serveable: &serveable,
            },
            is_stylesheet: &is_stylesheet,
        })
    }

    pub fn start_chapter(&self) -> Result<ChapterIx, Error> {
        let toc_size = self
            .nav_entry
            .as_ref()
            .or(self.ncx_entry.as_ref())
            .and_then(|name| self.zip.entry(name))
            .map_or(0, |entry| entry.inflated);
        if toc_size <= MAX_TOC_FOR_START {
            let target = self.toc()?.entries.iter().find_map(|entry| {
                entry
                    .target
                    .filter(|target| !self.rows[target.0 as usize].is_nav)
            });
            if let Some(target) = target {
                return Ok(target);
            }
        }
        Ok(self
            .rows
            .iter()
            .find(|row| row.linear_ordinal.is_some())
            .map_or(ChapterIx(0), |row| row.ix))
    }

    pub fn heading_titles(&self, cancel: &CancelToken, sink: &mut dyn FnMut(ChapterIx, String)) {
        for row in &self.rows {
            if cancel.is_cancelled() {
                return;
            }
            if row.linear_ordinal.is_none() {
                continue;
            }
            if let Ok((sanitized, ..)) = self.sanitized(row.ix)
                && let Some(heading) = sanitized.heading
            {
                sink(row.ix, heading);
            }
        }
    }

    #[doc(hidden)]
    pub fn entry_record_range(&self, ix: ChapterIx) -> Range<u64> {
        self.entry_of(ix).map_or(0..0, |entry| entry.record.clone())
    }
}
