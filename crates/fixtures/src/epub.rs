use crate::plants::{Counts, HEADING_TERM};
use crate::prose::{self, ChapterSpec};
use crate::rng::derive;
use crate::truth::{SpineRow, TocRow};
use crate::zip::{DEFLATE, STORED, ZipWriter};
use std::io;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spine {
    No,
    Linear,
    NonLinear,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
    Empty,
    Nav,
    Ncx,
    Resource,
    Missing,
    Unresolved,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Empty => "empty",
            Kind::Nav => "nav",
            Kind::Ncx => "ncx",
            Kind::Resource => "resource",
            Kind::Missing => "missing-from-zip",
            Kind::Unresolved => "unresolved-idref",
        }
    }
}

pub enum Body {
    Prose(Box<ChapterSpec>),
    Bytes(Vec<u8>),
    Generated,
    WrittenByCaller,
}

pub struct Item {
    pub id: String,
    pub href: String,
    pub media_type: &'static str,
    pub properties: Option<&'static str>,
    pub spine: Spine,
    pub kind: Kind,
    pub title: Option<String>,
    pub encoding: &'static str,
    pub well_formed: bool,
    pub defect: Option<&'static str>,
    pub zip_name: Option<Vec<u8>>,
    pub body: Body,
}

pub const XHTML: &str = "application/xhtml+xml";

impl Item {
    pub fn resource(id: &str, href: &str, media_type: &'static str, bytes: Vec<u8>) -> Self {
        Item {
            id: id.to_owned(),
            href: href.to_owned(),
            media_type,
            properties: None,
            spine: Spine::No,
            kind: Kind::Resource,
            title: None,
            encoding: "binary",
            well_formed: true,
            defect: None,
            zip_name: None,
            body: Body::Bytes(bytes),
        }
    }

    pub fn page(id: &str, href: &str, kind: Kind, title: Option<&str>, bytes: Vec<u8>) -> Self {
        Item {
            id: id.to_owned(),
            href: href.to_owned(),
            media_type: XHTML,
            properties: None,
            spine: Spine::Linear,
            kind,
            title: title.map(str::to_owned),
            encoding: "utf-8",
            well_formed: true,
            defect: None,
            zip_name: None,
            body: Body::Bytes(bytes),
        }
    }

    pub fn prose(id: &str, href: &str, spec: ChapterSpec) -> Self {
        Item {
            id: id.to_owned(),
            href: href.to_owned(),
            media_type: XHTML,
            properties: None,
            spine: Spine::Linear,
            kind: Kind::Text,
            title: spec.heading(),
            encoding: spec.decl.label(),
            well_formed: spec.defect.is_none(),
            defect: spec.defect.map(|d| d.label()),
            zip_name: None,
            body: Body::Prose(Box::new(spec)),
        }
    }

    pub fn nav(in_spine: bool) -> Self {
        Item {
            id: "nav".to_owned(),
            href: "nav.xhtml".to_owned(),
            media_type: XHTML,
            properties: Some("nav"),
            spine: if in_spine { Spine::Linear } else { Spine::No },
            kind: Kind::Nav,
            title: None,
            encoding: "utf-8",
            well_formed: true,
            defect: None,
            zip_name: None,
            body: Body::Generated,
        }
    }

    pub fn ncx() -> Self {
        Item {
            id: "ncx".to_owned(),
            href: "toc.ncx".to_owned(),
            media_type: "application/x-dtbncx+xml",
            properties: None,
            spine: Spine::No,
            kind: Kind::Ncx,
            title: None,
            encoding: "utf-8",
            well_formed: true,
            defect: None,
            zip_name: None,
            body: Body::Generated,
        }
    }

    pub fn missing(id: &str, href: &str) -> Self {
        let mut item = Item::page(id, href, Kind::Missing, None, Vec::new());
        item.body = Body::WrittenByCaller;
        item
    }

    pub fn unresolved(id: &str) -> Self {
        let mut item = Item::page(id, "", Kind::Unresolved, None, Vec::new());
        item.body = Body::WrittenByCaller;
        item
    }
}

pub struct TocNode {
    pub label: String,
    pub target: Option<String>,
    pub children: Vec<TocNode>,
}

impl TocNode {
    pub fn link(label: impl Into<String>, target: impl Into<String>) -> Self {
        TocNode {
            label: label.into(),
            target: Some(target.into()),
            children: Vec::new(),
        }
    }

    pub fn group(label: impl Into<String>, children: Vec<TocNode>) -> Self {
        TocNode {
            label: label.into(),
            target: None,
            children,
        }
    }
}

pub struct Package {
    pub name: &'static str,
    pub title: String,
    pub version: &'static str,
    pub language: &'static str,
    pub items: Vec<Item>,
    pub toc: Vec<TocNode>,
    pub extra_metadata: String,
    pub container_target: Option<String>,
    pub mimetype_not_first: bool,
}

impl Package {
    pub fn new(name: &'static str, title: impl Into<String>) -> Self {
        Package {
            name,
            title: title.into(),
            version: "3.0",
            language: "en",
            items: Vec::new(),
            toc: Vec::new(),
            extra_metadata: String::new(),
            container_target: None,
            mimetype_not_first: false,
        }
    }

    fn has(&self, kind: Kind) -> bool {
        self.items.iter().any(|i| i.kind == kind)
    }

    fn opf(&self) -> String {
        let uuid_a = derive(self.name, 1);
        let uuid_b = derive(self.name, 2);
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"{}\" unique-identifier=\"uid\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"uid\">urn:uuid:{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:creator>Jadeslip Fixtures</dc:creator>\n<dc:language>{}</dc:language>\n",
            self.version,
            uuid_a >> 32,
            (uuid_a >> 16) & 0xFFFF,
            uuid_a & 0xFFF,
            (uuid_b >> 48) & 0xFFF,
            uuid_b & 0xFFFF_FFFF_FFFF,
            escape(&self.title),
            self.language,
        );
        if self.version.starts_with('3') {
            out.push_str("<meta property=\"dcterms:modified\">2026-01-01T00:00:00Z</meta>\n");
        }
        out.push_str(&self.extra_metadata);
        out.push_str("</metadata>\n<manifest>\n");
        for item in self.items.iter().filter(|i| i.kind != Kind::Unresolved) {
            out.push_str(&format!(
                "<item id=\"{}\" href=\"{}\" media-type=\"{}\"",
                item.id, item.href, item.media_type
            ));
            if let Some(properties) = item.properties {
                out.push_str(&format!(" properties=\"{properties}\""));
            }
            out.push_str("/>\n");
        }
        out.push_str("</manifest>\n<spine");
        if self.has(Kind::Ncx) {
            out.push_str(" toc=\"ncx\"");
        }
        out.push_str(">\n");
        for item in self.items.iter().filter(|i| i.spine != Spine::No) {
            out.push_str(&format!("<itemref idref=\"{}\"", item.id));
            if item.spine == Spine::NonLinear {
                out.push_str(" linear=\"no\"");
            }
            out.push_str("/>\n");
        }
        out.push_str("</spine>\n</package>\n");
        out
    }

    fn nav_xhtml(&self) -> String {
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head>\n<body>\n<nav epub:type=\"toc\"><h1>Contents zephyrquill {HEADING_TERM}</h1>\n"
        );
        nav_list(&self.toc, &mut out);
        out.push_str("</nav>\n</body></html>\n");
        out
    }

    fn ncx_xml(&self) -> String {
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{}\"/></head><docTitle><text>{}</text></docTitle><navMap>\n",
            self.name,
            escape(&self.title)
        );
        let mut order = 0;
        ncx_points(&self.toc, &mut order, &mut out);
        out.push_str("</navMap></ncx>\n");
        out
    }

    pub fn toc_rows(&self) -> Vec<TocRow> {
        let mut rows = Vec::new();
        flatten(&self.toc, 0, &mut rows);
        rows
    }

    pub fn spine_rows(&self) -> Vec<SpineRow> {
        self.items
            .iter()
            .filter(|i| i.spine != Spine::No)
            .enumerate()
            .map(|(ix, item)| SpineRow {
                ix: ix as u32,
                href: (item.kind != Kind::Unresolved).then(|| item.href.clone()),
                title: item.title.clone(),
                kind: item.kind.label(),
                linear: item.spine == Spine::Linear,
                encoding: item.encoding,
                well_formed: item.well_formed,
                defect: item.defect,
            })
            .collect()
    }

    pub fn linear_chapters(&self) -> u32 {
        self.items
            .iter()
            .filter(|i| i.spine == Spine::Linear && i.kind != Kind::Nav)
            .count() as u32
    }

    pub fn write_front(&self, zip: &mut ZipWriter) -> io::Result<()> {
        let container = format!(
            "<?xml version=\"1.0\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"{}\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>\n",
            self.container_target
                .as_deref()
                .unwrap_or("OEBPS/content.opf")
        );
        if self.mimetype_not_first {
            zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)?;
            zip.add(b"mimetype", b"application/epub+zip", DEFLATE)?;
        } else {
            zip.add(b"mimetype", b"application/epub+zip", STORED)?;
            zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)?;
        }
        zip.add(b"OEBPS/content.opf", self.opf().as_bytes(), DEFLATE)
    }

    pub fn entry_name(item: &Item) -> Vec<u8> {
        item.zip_name
            .clone()
            .unwrap_or_else(|| format!("OEBPS/{}", item.href).into_bytes())
    }

    pub fn write_all(&self, zip: &mut ZipWriter, counts: &mut Counts) -> io::Result<()> {
        self.write_front(zip)?;
        let mut spine_ix = 0;
        for item in &self.items {
            let name = Package::entry_name(item);
            let method = if item.media_type.starts_with("image/") {
                STORED
            } else {
                DEFLATE
            };
            match &item.body {
                Body::Prose(spec) => {
                    zip.add(&name, &prose::render(spec, spine_ix, counts), method)?
                }
                Body::Bytes(bytes) => zip.add(&name, bytes, method)?,
                Body::Generated => {
                    let text = match item.kind {
                        Kind::Nav => self.nav_xhtml(),
                        _ => self.ncx_xml(),
                    };
                    zip.add(&name, text.as_bytes(), method)?
                }
                Body::WrittenByCaller => {}
            }
            if item.spine != Spine::No {
                spine_ix += 1;
            }
        }
        Ok(())
    }
}

fn nav_list(nodes: &[TocNode], out: &mut String) {
    out.push_str("<ol>\n");
    for node in nodes {
        match &node.target {
            Some(target) => out.push_str(&format!(
                "<li><a href=\"{}\">{}</a>",
                target,
                escape(&node.label)
            )),
            None => out.push_str(&format!("<li><span>{}</span>", escape(&node.label))),
        }
        if !node.children.is_empty() {
            nav_list(&node.children, out);
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ol>\n");
}

fn ncx_points(nodes: &[TocNode], order: &mut u32, out: &mut String) {
    for node in nodes {
        *order += 1;
        out.push_str(&format!(
            "<navPoint id=\"np{0}\" playOrder=\"{0}\"><navLabel><text>{1}</text></navLabel>",
            order,
            escape(&node.label)
        ));
        if let Some(target) = &node.target {
            out.push_str(&format!("<content src=\"{target}\"/>"));
        }
        ncx_points(&node.children, order, out);
        out.push_str("</navPoint>\n");
    }
}

fn flatten(nodes: &[TocNode], depth: u16, rows: &mut Vec<TocRow>) {
    for node in nodes {
        rows.push(TocRow {
            depth,
            label: node.label.clone(),
            target: node.target.clone(),
        });
        flatten(&node.children, depth + 1, rows);
    }
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
