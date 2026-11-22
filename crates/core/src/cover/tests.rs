use super::*;
use crate::DEFAULT_LIMITS;
use crate::common::fixture;
use image::{GenericImageView, RgbImage};
use jadeslip_fixtures::png;
use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Manifest<'a> {
    id: &'a str,
    href: &'a str,
    media_type: &'a str,
    properties: &'a str,
}

const fn item<'a>(id: &'a str, href: &'a str, media_type: &'a str) -> Manifest<'a> {
    Manifest {
        id,
        href,
        media_type,
        properties: "",
    }
}

const fn marked<'a>(id: &'a str, href: &'a str, media_type: &'a str) -> Manifest<'a> {
    Manifest {
        id,
        href,
        media_type,
        properties: "nav cover-image",
    }
}

fn book(items: &[Manifest], meta_cover: Option<&str>, files: &[(&str, Vec<u8>)]) -> Reader {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf =
        jadeslip_fixtures::scratch::scratch(&format!("cover-{n}")).join("book.epub");
    let mut manifest =
        String::from("<item id=\"c0\" href=\"c0.xhtml\" media-type=\"application/xhtml+xml\"/>");
    for item in items {
        manifest.push_str(&format!(
            "<item id=\"{}\" href=\"{}\" media-type=\"{}\" properties=\"{}\"/>",
            item.id, item.href, item.media_type, item.properties
        ));
    }
    let meta = meta_cover.map_or(String::new(), |id| {
        format!("<meta name=\"cover\" content=\"{id}\"/>")
    });
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">Cover</dc:title>{meta}</metadata><manifest>{manifest}</manifest><spine><itemref idref=\"c0\"/></spine></package>"
    );
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"content.opf", opf.as_bytes(), DEFLATE).unwrap();
    zip.add(
        b"c0.xhtml",
        b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p>x</p></body></html>",
        DEFLATE,
    )
    .unwrap();
    for (name, bytes) in files {
        zip.add(name.as_bytes(), bytes, DEFLATE).unwrap();
    }
    zip.finish().unwrap();
    Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap()
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let picture = RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 120])
    });
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&picture)
        .unwrap();
    out
}

fn cover_entry(reader: &Reader) -> Option<String> {
    reader
        .summary()
        .cover
        .as_ref()
        .map(|cover| cover.entry.as_str().to_owned())
}

fn thumbnail_size(reader: &Reader) -> Option<(u32, u32)> {
    let bytes = reader.cover_thumbnail()?;
    let picture = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
    Some(picture.dimensions())
}

#[test]
fn the_epub3_cover_image_property_wins() {
    let reader = book(
        &[
            item("cover", "cover.jpg", "image/jpeg"),
            marked("art", "art.png", "image/png"),
        ],
        Some("cover"),
        &[
            ("cover.jpg", jpeg(100, 150)),
            ("art.png", png::cover(1, 600, 900)),
        ],
    );
    assert_eq!(cover_entry(&reader).as_deref(), Some("art.png"));
    assert_eq!(thumbnail_size(&reader), Some((400, 600)));
}

#[test]
fn the_epub2_meta_names_the_cover() {
    let reader = book(
        &[
            item("plate", "plate.jpg", "image/jpeg"),
            item("front", "front.jpg", "image/jpeg"),
        ],
        Some("front"),
        &[("plate.jpg", jpeg(80, 80)), ("front.jpg", jpeg(1200, 1600))],
    );
    assert_eq!(cover_entry(&reader).as_deref(), Some("front.jpg"));
    assert_eq!(thumbnail_size(&reader), Some((400, 533)));
}

#[test]
fn an_image_named_cover_is_the_last_resort() {
    let reader = book(
        &[
            item("plate", "images/plate.jpg", "image/jpeg"),
            item("pic", "images/Cover-Front.JPG", "image/jpeg"),
        ],
        None,
        &[
            ("images/plate.jpg", jpeg(80, 80)),
            ("images/Cover-Front.JPG", jpeg(300, 200)),
        ],
    );
    assert_eq!(
        cover_entry(&reader).as_deref(),
        Some("images/Cover-Front.JPG")
    );
    assert_eq!(thumbnail_size(&reader), Some((300, 200)), "never enlarged");
}

#[test]
fn a_book_without_any_cover_has_none() {
    let reader = book(
        &[item("plate", "plate.png", "image/png")],
        None,
        &[("plate.png", png::cover(2, 100, 100))],
    );
    assert_eq!(cover_entry(&reader), None);
    assert_eq!(reader.cover_thumbnail(), None);
}

#[test]
fn a_broken_reference_falls_through_to_the_next_rule() {
    let missing_entry = book(
        &[
            marked("art", "art.png", "image/png"),
            item("front", "front.jpg", "image/jpeg"),
        ],
        Some("front"),
        &[("front.jpg", jpeg(200, 300))],
    );
    assert_eq!(cover_entry(&missing_entry).as_deref(), Some("front.jpg"));
    assert_eq!(thumbnail_size(&missing_entry), Some((200, 300)));

    let missing_item = book(
        &[item("cover-pic", "pic.png", "image/png")],
        Some("nothing-here"),
        &[("pic.png", png::cover(3, 120, 180))],
    );
    assert_eq!(cover_entry(&missing_item).as_deref(), Some("pic.png"));
    assert_eq!(thumbnail_size(&missing_item), Some((120, 180)));

    let nothing_left = book(
        &[marked("art", "art.png", "image/png")],
        Some("nothing-here"),
        &[],
    );
    assert_eq!(cover_entry(&nothing_left), None);
    assert_eq!(nothing_left.cover_thumbnail(), None);
}

#[test]
fn svg_gif_and_webp_covers_count_as_none() {
    for (href, media_type) in [
        ("cover.svg", "image/svg+xml"),
        ("cover.gif", "image/gif"),
        ("cover.webp", "image/webp"),
    ] {
        let reader = book(
            &[marked("cover", href, media_type)],
            Some("cover"),
            &[(href, b"GIF89a not really".to_vec())],
        );
        assert_eq!(cover_entry(&reader), None, "{media_type}");
        assert_eq!(reader.cover_thumbnail(), None, "{media_type}");
    }
}

#[test]
fn a_header_claiming_a_huge_picture_is_refused() {
    let reader = book(
        &[marked("art", "art.png", "image/png")],
        None,
        &[("art.png", png::claiming(50_000, 50_000))],
    );
    assert_eq!(cover_entry(&reader).as_deref(), Some("art.png"));
    assert_eq!(reader.cover_thumbnail(), None);
    let wide = |width| {
        book(
            &[marked("art", "art.png", "image/png")],
            None,
            &[("art.png", png::cover(6, width, 20))],
        )
    };
    assert!(
        wide(6000).cover_thumbnail().is_some(),
        "the control decodes"
    );
    assert_eq!(wide(6001).cover_thumbnail(), None);
}

#[test]
fn a_cover_entry_over_the_cap_is_refused() {
    let padded = |extra: u64| {
        let mut bytes = png::cover(4, 60, 90);
        bytes.resize(bytes.len() + extra as usize, 0);
        bytes
    };
    let under = book(
        &[marked("art", "art.png", "image/png")],
        None,
        &[("art.png", padded(MAX_COVER_BYTES - (1 << 20)))],
    );
    assert_eq!(
        thumbnail_size(&under),
        Some((60, 90)),
        "the control decodes"
    );
    let over = book(
        &[marked("art", "art.png", "image/png")],
        None,
        &[("art.png", padded(MAX_COVER_BYTES))],
    );
    assert_eq!(cover_entry(&over).as_deref(), Some("art.png"));
    assert_eq!(over.cover_thumbnail(), None);
}

#[test]
fn corrupt_or_mislabelled_bytes_give_none() {
    let mut truncated = jpeg(400, 600);
    truncated.truncate(200);
    for bytes in [
        b"not a picture at all".to_vec(),
        truncated,
        png::cover(5, 50, 50),
    ] {
        let reader = book(
            &[marked("art", "art.jpg", "image/jpeg")],
            None,
            &[("art.jpg", bytes)],
        );
        assert_eq!(reader.cover_thumbnail(), None);
    }
}

#[test]
fn the_generated_cover_books_are_found_by_their_own_rule() {
    for (name, entry) in [
        ("cover-epub3", "OEBPS/Images/art.png"),
        ("cover-epub2", "OEBPS/Images/front.png"),
    ] {
        let reader = Reader::open(File::open(fixture(name)).unwrap(), DEFAULT_LIMITS).unwrap();
        assert_eq!(cover_entry(&reader).as_deref(), Some(entry), "{name}");
        let (width, height) = thumbnail_size(&reader).unwrap();
        assert!(width <= 400 && height <= 600 && width > 0, "{name}");
    }
    let plain = Reader::open(File::open(fixture("toc-nav-only")).unwrap(), DEFAULT_LIMITS).unwrap();
    assert_eq!(plain.cover_thumbnail(), None);
}

#[test]
#[ignore = "real books, counts only: cargo test --release -p jadeslip-core --lib real_book_covers -- --ignored --nocapture"]
fn real_book_covers_are_found_and_shrunk() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "epub")
    });
    let (mut found, mut shrunk, mut slowest) = (0, 0, 0.0f64);
    for path in &paths {
        let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
        found += usize::from(reader.summary().cover.is_some());
        let started = std::time::Instant::now();
        if let Some(size) = thumbnail_size(&reader) {
            assert!(size.0 <= 400 && size.1 <= 600);
            shrunk += 1;
        }
        slowest = slowest.max(started.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "real books {}, covers found {found}, thumbnails {shrunk}, slowest {slowest:.0} ms",
        paths.len()
    );
}
