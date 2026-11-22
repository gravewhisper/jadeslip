use crate::epub::{Body, Item, Kind, Package};
use crate::plants::Counts;
use crate::rng::Rng;
use crate::truth::{BombTruth, Truth};
use crate::zip::{DEFLATE, Record, STORED, ZipWriter, deflate, deflate_max, local_header};
use std::collections::BTreeMap;
use std::io;

const MIB: usize = 1 << 20;
const JUST_UNDER_ENTRY_LIMIT: usize = 63 * MIB;
const OVER_ENTRY_LIMIT: usize = 65 * MIB;
const ENTRIES_OVER_BOOK_LIMIT: usize = 66;
const ENTRIES_OVER_COUNT_LIMIT: usize = 200_001;
const XML_DEPTH: usize = 100_000;
const OPF_DEPTH: usize = 1_000;

fn small_chapter(label: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{label}</title></head>\n<body>\n<h1>{label}</h1>\n<p>Readable control text for {label}.</p>\n</body></html>\n"
    )
    .into_bytes()
}

fn wrap_chapter(filler: Vec<u8>) -> Vec<u8> {
    let mut bytes = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>bomb</title></head><body><p>".to_vec();
    bytes.extend_from_slice(&filler);
    bytes.extend_from_slice(b"</p></body></html>\n");
    bytes
}

fn single_byte_chapter(len: usize) -> Vec<u8> {
    wrap_chapter(vec![b'a'; len])
}

fn speckled_chapter(book: &str, len: usize) -> Vec<u8> {
    let mut rng = Rng::for_item(book, 0);
    let mut filler = vec![b'a'; len];
    for chunk in filler.chunks_mut(8192) {
        for byte in chunk.iter_mut().take(16) {
            *byte = b'a' + rng.below(26) as u8;
        }
    }
    wrap_chapter(filler)
}

fn caller_item(id: &str, href: &str) -> Item {
    let mut item = Item::page(id, href, Kind::Text, Some(id), Vec::new());
    item.body = Body::WrittenByCaller;
    item
}

fn truth(
    package: &Package,
    trips: &'static str,
    stage: &'static str,
    detail: &'static str,
) -> Truth {
    Truth {
        book: package.name.to_owned(),
        category: "bomb".to_owned(),
        meant_valid: false,
        title: package.title.clone(),
        linear_chapters: package.linear_chapters(),
        spine: package.spine_rows(),
        toc: package.toc_rows(),
        terms: BTreeMap::new(),
        bomb: Some(BombTruth {
            trips,
            stage,
            detail,
        }),
        attack: None,
        clean: None,
    }
}

fn deflated_entry(zip: &mut ZipWriter, name: &[u8], data: &[u8], max: bool) -> io::Result<()> {
    let body = if max {
        deflate_max(data)
    } else {
        deflate(data)
    };
    let record = Record::honest(
        name,
        DEFLATE,
        crc32fast::hash(data),
        body.len() as u64,
        data.len() as u64,
    );
    let record = zip.write_local(record, &body)?;
    zip.push_central(record);
    Ok(())
}

pub fn ratio(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-ratio", "Bomb Ratio");
    package.items.push(caller_item("c0", "c0.xhtml"));
    package.write_front(zip)?;
    deflated_entry(
        zip,
        b"OEBPS/c0.xhtml",
        &single_byte_chapter(JUST_UNDER_ENTRY_LIMIT),
        true,
    )?;
    Ok(truth(
        &package,
        "none",
        "loads",
        "One chapter under max_entry_inflated whose honest sizes exceed 1000:1. No ratio limit exists, so it opens and loads, bounded by max_entry_inflated.",
    ))
}

pub fn oversize_entry(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-oversize-entry", "Bomb Oversize Entry");
    package.items.push(caller_item("c0", "c0.xhtml"));
    package.write_front(zip)?;
    deflated_entry(
        zip,
        b"OEBPS/c0.xhtml",
        &speckled_chapter(package.name, OVER_ENTRY_LIMIT),
        false,
    )?;
    Ok(truth(
        &package,
        "max_entry_inflated",
        "chapter",
        "One chapter over max_entry_inflated with honest sizes. The book opens and loading that chapter fails.",
    ))
}

pub fn book_total(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-book-total", "Bomb Book Total");
    for ix in 0..ENTRIES_OVER_BOOK_LIMIT {
        package
            .items
            .push(caller_item(&format!("c{ix}"), &format!("c{ix}.xhtml")));
    }
    package.write_front(zip)?;
    let data = speckled_chapter(package.name, JUST_UNDER_ENTRY_LIMIT);
    let body = deflate(&data);
    let crc = crc32fast::hash(&data);
    for ix in 0..ENTRIES_OVER_BOOK_LIMIT {
        let name = format!("OEBPS/c{ix}.xhtml");
        let record = Record::honest(
            name.as_bytes(),
            DEFLATE,
            crc,
            body.len() as u64,
            data.len() as u64,
        );
        let record = zip.write_local(record, &body)?;
        zip.push_central(record);
    }
    Ok(truth(
        &package,
        "max_book_inflated",
        "open",
        "66 entries of 63 MiB, each under the entry limit, summing past 4 GiB.",
    ))
}

pub fn overlap_shared(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-overlap-shared", "Bomb Overlap Shared");
    let copies = 1000;
    for ix in 0..copies {
        package
            .items
            .push(caller_item(&format!("c{ix}"), &format!("c{ix}.xhtml")));
    }
    package.write_front(zip)?;
    let data = speckled_chapter(package.name, MIB);
    let body = deflate(&data);
    let record = Record::honest(
        b"OEBPS/c0.xhtml",
        DEFLATE,
        crc32fast::hash(&data),
        body.len() as u64,
        data.len() as u64,
    );
    let record = zip.write_local(record, &body)?;
    for ix in 0..copies {
        let mut alias = record.clone();
        alias.name = format!("OEBPS/c{ix}.xhtml").into_bytes();
        zip.push_central(alias);
    }
    Ok(truth(
        &package,
        "overlap",
        "open",
        "1000 central records share one local record offset.",
    ))
}

pub fn overlap_nested(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-overlap-nested", "Bomb Overlap Nested");
    package.items.push(caller_item("c0", "c0.xhtml"));
    package.items.push(caller_item("c1", "c1.xhtml"));
    package.write_front(zip)?;
    let inner_data = small_chapter("inner");
    let mut inner = Record::honest(
        b"OEBPS/c1.xhtml",
        STORED,
        crc32fast::hash(&inner_data),
        inner_data.len() as u64,
        inner_data.len() as u64,
    );
    let inner_header = local_header(&inner);
    let mut outer_data = inner_header.clone();
    outer_data.extend_from_slice(&inner_data);
    let outer = Record::honest(
        b"OEBPS/c0.xhtml",
        STORED,
        crc32fast::hash(&outer_data),
        outer_data.len() as u64,
        outer_data.len() as u64,
    );
    let outer = zip.write_local(outer, &outer_data)?;
    inner.offset = outer.offset + 30 + outer.name.len() as u64;
    zip.push_central(outer);
    zip.push_central(inner);
    Ok(truth(
        &package,
        "overlap",
        "open",
        "Stored c0 contains the whole local record of c1, so sorted offsets alone show no overlap.",
    ))
}

pub fn lying_sizes(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-lying-sizes", "Bomb Lying Sizes");
    let cases = [
        "honest",
        "declares-small",
        "declares-large",
        "local-differs",
        "wrong-crc",
    ];
    for (ix, case) in cases.iter().enumerate() {
        let mut item = caller_item(&format!("c{ix}"), &format!("c{ix}.xhtml"));
        item.defect = Some(case);
        package.items.push(item);
    }
    package.write_front(zip)?;

    deflated_entry(zip, b"OEBPS/c0.xhtml", &small_chapter("honest"), false)?;

    let big = speckled_chapter(package.name, 10 * MIB);
    let body = deflate(&big);
    let lie = Record::honest(
        b"OEBPS/c1.xhtml",
        DEFLATE,
        crc32fast::hash(&big),
        body.len() as u64,
        1024,
    );
    let lie = zip.write_local(lie, &body)?;
    zip.push_central(lie);

    let small = small_chapter("declares large");
    let body = deflate(&small);
    let lie = Record::honest(
        b"OEBPS/c2.xhtml",
        DEFLATE,
        crc32fast::hash(&small),
        body.len() as u64,
        MIB as u64,
    );
    let lie = zip.write_local(lie, &body)?;
    zip.push_central(lie);

    let data = small_chapter("local differs");
    let body = deflate(&data);
    let local = Record::honest(b"OEBPS/c3.xhtml", DEFLATE, 0x0BAD_F00D, 1, 5);
    let mut central = zip.write_local(local, &body)?;
    central.crc32 = crc32fast::hash(&data);
    central.compressed = body.len() as u64;
    central.inflated = data.len() as u64;
    zip.push_central(central);

    let data = small_chapter("wrong crc");
    let body = deflate(&data);
    let lie = Record::honest(
        b"OEBPS/c4.xhtml",
        DEFLATE,
        crc32fast::hash(&data) ^ 1,
        body.len() as u64,
        data.len() as u64,
    );
    let lie = zip.write_local(lie, &body)?;
    zip.push_central(lie);

    Ok(truth(
        &package,
        "declared-sizes",
        "chapter",
        "Spine item 0 reads. Items 1 to 4 lie about inflated size or CRC, which only inflating can detect.",
    ))
}

pub fn lying_extent(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-lying-extent", "Bomb Lying Extent");
    package.items.push(caller_item("c0", "c0.xhtml"));
    package.items.push(caller_item("c1", "c1.xhtml"));
    package.items.push(caller_item("c2", "c2.xhtml"));
    package.write_front(zip)?;
    deflated_entry(zip, b"OEBPS/c0.xhtml", &small_chapter("honest"), false)?;

    let data = small_chapter("compressed past end");
    let body = deflate(&data);
    let record = Record::honest(
        b"OEBPS/c1.xhtml",
        DEFLATE,
        crc32fast::hash(&data),
        body.len() as u64,
        data.len() as u64,
    );
    let mut central = zip.write_local(record, &body)?;
    central.compressed = 1 << 31;
    zip.push_central(central);

    let data = small_chapter("offset past end");
    let mut central = Record::honest(
        b"OEBPS/c2.xhtml",
        STORED,
        crc32fast::hash(&data),
        data.len() as u64,
        data.len() as u64,
    );
    central.offset = 0x7FFF_FFF0;
    zip.push_central(central);

    Ok(truth(
        &package,
        "extent-past-eof",
        "open",
        "Central records claim a compressed length and a local offset beyond the end of the file.",
    ))
}

pub fn huge_entry_count(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-huge-entry-count", "Bomb Huge Entry Count");
    package.items.push(caller_item("c0", "c0.xhtml"));
    package.write_front(zip)?;
    deflated_entry(zip, b"OEBPS/c0.xhtml", &small_chapter("control"), false)?;
    let empty_crc = crc32fast::hash(b"");
    for ix in 0..ENTRIES_OVER_COUNT_LIMIT {
        let name = format!("OEBPS/x/{ix:06}");
        let record = Record::honest(name.as_bytes(), STORED, empty_crc, 0, 0);
        let record = zip.write_local(record, b"")?;
        zip.push_central(record);
    }
    Ok(truth(
        &package,
        "max_entries",
        "open",
        "A true count past max_entries, recorded through the ZIP64 end records.",
    ))
}

fn traversal_book(
    zip: &mut ZipWriter,
    name: &'static str,
    title: &str,
    zip_name: &[u8],
    href: &str,
    detail: &'static str,
) -> io::Result<Truth> {
    let mut package = Package::new(name, title);
    let mut item = Item::page(
        "c0",
        href,
        Kind::Text,
        Some("control"),
        small_chapter("control"),
    );
    item.zip_name = Some(zip_name.to_vec());
    package.items.push(item);
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(truth(&package, "traversal", "open", detail))
}

pub fn traversal_dotdot(zip: &mut ZipWriter) -> io::Result<Truth> {
    traversal_book(
        zip,
        "bomb-traversal-dotdot",
        "Bomb Traversal Dotdot",
        b"OEBPS/../../../evil.xhtml",
        "../../../evil.xhtml",
        "Entry name climbs out of the book with dot-dot segments.",
    )
}

pub fn traversal_absolute(zip: &mut ZipWriter) -> io::Result<Truth> {
    traversal_book(
        zip,
        "bomb-traversal-absolute",
        "Bomb Traversal Absolute",
        b"/tmp/jadeslip-evil.xhtml",
        "/tmp/jadeslip-evil.xhtml",
        "Entry name is an absolute path.",
    )
}

pub fn traversal_backslash(zip: &mut ZipWriter) -> io::Result<Truth> {
    traversal_book(
        zip,
        "bomb-traversal-backslash",
        "Bomb Traversal Backslash",
        b"OEBPS\\..\\..\\evil.xhtml",
        "evil.xhtml",
        "Entry name uses backslash separators.",
    )
}

pub fn traversal_nul(zip: &mut ZipWriter) -> io::Result<Truth> {
    traversal_book(
        zip,
        "bomb-traversal-nul",
        "Bomb Traversal Nul",
        b"OEBPS/c0.xhtml\0.png",
        "c0.xhtml",
        "Entry name embeds a NUL byte.",
    )
}

pub fn traversal_opf_href(zip: &mut ZipWriter) -> io::Result<Truth> {
    traversal_book(
        zip,
        "bomb-traversal-opf-href",
        "Bomb Traversal Opf Href",
        b"OEBPS/c0.xhtml",
        "../../../../etc/hostname",
        "Clean ZIP names, but a manifest href climbs out of the book.",
    )
}

pub fn traversal_container(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-traversal-container", "Bomb Traversal Container");
    package.container_target = Some("../../../etc/content.opf".to_owned());
    package.items.push(Item::page(
        "c0",
        "c0.xhtml",
        Kind::Text,
        Some("control"),
        small_chapter("control"),
    ));
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(truth(
        &package,
        "traversal",
        "open",
        "container.xml points the rootfile outside the book.",
    ))
}

pub fn deep_chapter(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-deep-xml-chapter", "Bomb Deep Xml Chapter");
    package.items.push(Item::page(
        "c0",
        "c0.xhtml",
        Kind::Text,
        Some("control"),
        small_chapter("control"),
    ));
    let mut deep = "<div>".repeat(XML_DEPTH);
    deep.push_str("deep");
    deep.push_str(&"</div>".repeat(XML_DEPTH));
    package.items.push(Item::page(
        "c1",
        "c1.xhtml",
        Kind::Text,
        Some("deep"),
        wrap_chapter(deep.into_bytes()),
    ));
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(truth(
        &package,
        "max_xml_depth",
        "chapter",
        "Spine item 0 reads. Spine item 1 nests 100,000 div elements.",
    ))
}

pub fn deep_opf(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("bomb-deep-xml-opf", "Bomb Deep Xml Opf");
    package.extra_metadata = format!(
        "{}deep{}\n",
        "<x>".repeat(OPF_DEPTH),
        "</x>".repeat(OPF_DEPTH)
    );
    package.items.push(Item::page(
        "c0",
        "c0.xhtml",
        Kind::Text,
        Some("control"),
        small_chapter("control"),
    ));
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(truth(
        &package,
        "max_xml_depth",
        "open",
        "The OPF metadata nests 1,000 elements, so open itself must refuse it.",
    ))
}
