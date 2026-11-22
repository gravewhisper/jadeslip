use crate::epub::{Item, Kind, Package, Spine, TocNode};
use crate::plants::{Counts, Density};
use crate::png;
use crate::prose::{ChapterSpec, Decl, Defect, Filler, Shape, title_words};
use crate::rng::{Rng, derive};
use crate::truth::Truth;
use crate::zip::ZipWriter;
use std::io;

const CSS: &str = "body{margin:1em;line-height:1.5}\nh1{font-size:1.4em}\n.note{color:#555}\n";
const LARGE_CHAPTERS: u32 = 6_000;
const LARGE_WORDS: (usize, usize) = (2_850, 3_150);
const SMALL_WORDS: (usize, usize) = (180, 260);

fn spec(book: &'static str, ordinal: u32, total: u32, title: Option<String>) -> ChapterSpec {
    ChapterSpec {
        book,
        ordinal,
        total,
        title,
        words: SMALL_WORDS,
        shape: Shape::H1P,
        filler: Filler::Latin,
        decl: Decl::Utf8,
        density: Density::Dense,
        defect: None,
        anchors: &[],
    }
}

fn chapter_title(book: &str, ordinal: u32) -> String {
    let mut rng = Rng::new(derive(book, u64::from(ordinal) + 1_000_000));
    format!("Chapter {}: {}", ordinal + 1, title_words(&mut rng))
}

fn image_page(title: &str, body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><head><title>{title}</title></head>\n<body>\n{body}\n</body></html>\n"
    )
    .into_bytes()
}

fn finish(
    package: &Package,
    category: &str,
    meant_valid: bool,
    counts: Counts,
    planted: bool,
) -> Truth {
    Truth {
        book: package.name.to_owned(),
        category: category.to_owned(),
        meant_valid,
        title: package.title.clone(),
        linear_chapters: package.linear_chapters(),
        spine: package.spine_rows(),
        toc: package.toc_rows(),
        terms: counts.finish(planted),
        bomb: None,
        attack: None,
        clean: None,
    }
}

pub fn finish_plain(package: &Package, category: &str, counts: Counts) -> Truth {
    finish(package, category, true, counts, false)
}

fn written(
    package: Package,
    category: &str,
    meant_valid: bool,
    planted: bool,
    zip: &mut ZipWriter,
) -> io::Result<Truth> {
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(finish(&package, category, meant_valid, counts, planted))
}

pub fn large_h1_p(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "large-h1-p";
    let mut package = Package::new(name, "Large H1 P");
    package.items.push(Item::ncx());
    package.items.push(Item::nav(false));
    package.items.push(Item::resource(
        "css",
        "Styles/main.css",
        "text/css",
        CSS.as_bytes().to_vec(),
    ));
    push_large_chapters(&mut package, name, Shape::H1P);
    written(package, "large", true, true, zip)
}

pub fn large_h1_div_br(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "large-h1-div-br";
    let mut package = Package::new(name, "Large H1 Div Br");
    package.items.push(Item::ncx());
    package.items.push(Item::resource(
        "css",
        "Styles/main.css",
        "text/css",
        CSS.as_bytes().to_vec(),
    ));
    let mut cover_image = Item::resource(
        "cover_png",
        "Images/cover.png",
        "image/png",
        png::page(derive(name, 7), 600, 800),
    );
    cover_image.properties = Some("cover-image");
    package.items.push(cover_image);
    package.items.push(Item::page(
        "cover",
        "Text/cover.xhtml",
        Kind::Image,
        None,
        image_page(
            "Cover",
            "<div><img src=\"../Images/cover.png\" alt=\"Cover\"/></div>",
        ),
    ));
    let mut intro = spec(name, LARGE_CHAPTERS + 1, 0, Some("Introduction".to_owned()));
    intro.density = Density::Sparse;
    package
        .items
        .push(Item::prose("intro", "Text/intro.xhtml", intro));
    package.items.push(Item::nav(true));
    push_large_chapters(&mut package, name, Shape::H1DivBr);
    let mut afterword = spec(name, LARGE_CHAPTERS + 2, 0, Some("Afterword".to_owned()));
    afterword.density = Density::Sparse;
    let mut afterword = Item::prose("afterword", "Text/afterword.xhtml", afterword);
    afterword.spine = Spine::NonLinear;
    package.items.push(afterword);
    written(package, "large", true, true, zip)
}

fn push_large_chapters(package: &mut Package, name: &'static str, shape: Shape) {
    for ordinal in 0..LARGE_CHAPTERS {
        let title = chapter_title(name, ordinal);
        let href = format!("Text/c{ordinal:05}.xhtml");
        package.toc.push(TocNode::link(title.clone(), href.clone()));
        let mut chapter = spec(name, ordinal, LARGE_CHAPTERS, Some(title));
        chapter.words = LARGE_WORDS;
        chapter.shape = shape;
        chapter.density = Density::Sparse;
        package
            .items
            .push(Item::prose(&format!("c{ordinal:05}"), &href, chapter));
    }
}

fn gbk_book(
    name: &'static str,
    title: &str,
    extension: &str,
    decl_of: fn(u32) -> Decl,
    zip: &mut ZipWriter,
) -> io::Result<Truth> {
    let mut package = Package::new(name, title);
    package.items.push(Item::nav(false));
    let total = 8;
    for ordinal in 0..total {
        let label = format!("第{}章 山河", ordinal + 1);
        let href = format!("Text/c{ordinal:02}.{extension}");
        package.toc.push(TocNode::link(label.clone(), href.clone()));
        let mut chapter = spec(name, ordinal, total, Some(label));
        chapter.filler = Filler::Han;
        chapter.decl = decl_of(ordinal);
        package
            .items
            .push(Item::prose(&format!("c{ordinal:02}"), &href, chapter));
    }
    written(package, "encoding", true, true, zip)
}

pub fn gbk_xml(zip: &mut ZipWriter) -> io::Result<Truth> {
    gbk_book(
        "gbk-xml-decl",
        "山河 GBK Xml",
        "xhtml",
        |_| Decl::GbkXml,
        zip,
    )
}

pub fn gbk_meta(zip: &mut ZipWriter) -> io::Result<Truth> {
    gbk_book(
        "gbk-meta-charset",
        "山河 GBK Meta",
        "html",
        |ordinal| {
            if ordinal % 4 == 3 {
                Decl::GbkHttpEquiv
            } else {
                Decl::GbkMeta
            }
        },
        zip,
    )
}

fn small_chapters(package: &mut Package, name: &'static str, total: u32) {
    for ordinal in 0..total {
        let title = chapter_title(name, ordinal);
        let href = format!("Text/c{ordinal:02}.xhtml");
        package.toc.push(TocNode::link(title.clone(), href.clone()));
        package.items.push(Item::prose(
            &format!("c{ordinal:02}"),
            &href,
            spec(name, ordinal, total, Some(title)),
        ));
    }
}

pub fn shelf_book(index: u32, cover: bool, zip: &mut ZipWriter) -> io::Result<()> {
    let name = "shelf-book";
    let mut package = Package::new(name, format!("Shelf Book {index:02}"));
    package.items.push(Item::nav(false));
    if cover {
        let mut image = Item::resource(
            "cover_png",
            "Images/cover.png",
            "image/png",
            png::cover(derive(name, u64::from(index)), 600, 900),
        );
        image.properties = Some("cover-image");
        package.items.push(image);
    }
    small_chapters(&mut package, name, 2 + index % 9);
    written(package, "shelf", true, false, zip).map(drop)
}

pub fn cover_epub3(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "cover-epub3";
    let mut package = Package::new(name, "Cover Epub3");
    package.items.push(Item::nav(false));
    let mut art = Item::resource(
        "art",
        "Images/art.png",
        "image/png",
        png::cover(derive(name, 0), 600, 900),
    );
    art.properties = Some("cover-image");
    package.items.push(art);
    small_chapters(&mut package, name, 4);
    written(package, "cover", true, false, zip)
}

pub fn cover_epub2(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "cover-epub2";
    let mut package = Package::new(name, "Cover Epub2");
    package.version = "2.0";
    package.extra_metadata = "<meta name=\"cover\" content=\"frontimage\"/>\n".to_owned();
    package.items.push(Item::ncx());
    package.items.push(Item::resource(
        "frontimage",
        "Images/front.png",
        "image/png",
        png::cover(derive(name, 0), 500, 750),
    ));
    small_chapters(&mut package, name, 4);
    written(package, "cover", true, false, zip)
}

pub fn malformed_markup(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "malformed-markup";
    let mut package = Package::new(name, "Malformed Markup");
    package.items.push(Item::nav(false));
    let defects = [
        None,
        Some(Defect::UnslashedBr),
        Some(Defect::BareAmpersand),
        Some(Defect::NbspNoDtd),
        Some(Defect::StrayClose),
        Some(Defect::ImpliedClose),
        Some(Defect::UnquotedAttr),
    ];
    let total = defects.len() as u32;
    for (ix, defect) in defects.into_iter().enumerate() {
        let ordinal = ix as u32;
        let title = chapter_title(name, ordinal);
        let href = format!("Text/c{ordinal:02}.xhtml");
        package.toc.push(TocNode::link(title.clone(), href.clone()));
        let mut chapter = spec(name, ordinal, total, Some(title));
        chapter.defect = defect;
        if defect == Some(Defect::UnslashedBr) {
            chapter.shape = Shape::H1DivBr;
        }
        package
            .items
            .push(Item::prose(&format!("c{ordinal:02}"), &href, chapter));
    }
    written(package, "malformed", false, true, zip)
}

pub fn malformed_missing_chapter(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "malformed-missing-chapter";
    let mut package = Package::new(name, "Malformed Missing Chapter");
    package.items.push(Item::nav(false));
    small_chapters(&mut package, name, 2);
    package.toc.push(TocNode::link("Gone", "Text/gone.xhtml"));
    package.items.push(Item::missing("gone", "Text/gone.xhtml"));
    let title = chapter_title(name, 2);
    package
        .toc
        .push(TocNode::link(title.clone(), "Text/c02.xhtml"));
    package.items.push(Item::prose(
        "c02",
        "Text/c02.xhtml",
        spec(name, 2, 3, Some(title)),
    ));
    written(package, "malformed", false, true, zip)
}

pub fn malformed_spine_ref(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "malformed-spine-ref";
    let mut package = Package::new(name, "Malformed Spine Ref");
    package.items.push(Item::nav(false));
    small_chapters(&mut package, name, 2);
    package.items.push(Item::unresolved("ghost"));
    let title = chapter_title(name, 2);
    package
        .toc
        .push(TocNode::link(title.clone(), "Text/c02.xhtml"));
    package.items.push(Item::prose(
        "c02",
        "Text/c02.xhtml",
        spec(name, 2, 3, Some(title)),
    ));
    written(package, "malformed", false, true, zip)
}

pub fn malformed_container_ref(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "malformed-container-ref";
    let mut package = Package::new(name, "Malformed Container Ref");
    package.container_target = Some("OEBPS/nowhere.opf".to_owned());
    package.items.push(Item::nav(false));
    small_chapters(&mut package, name, 2);
    written(package, "malformed", false, true, zip)
}

pub fn images_only(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "images-only";
    let mut package = Package::new(name, "Images Only");
    package.items.push(Item::nav(false));
    let bodies = [
        "<img src=\"../Images/p00.png\" alt=\"\"/>",
        "<div><img src=\"../Images/p01.png\" alt=\"\"/></div>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 300 400\"><image xlink:href=\"../Images/p02.png\" width=\"300\" height=\"400\"/></svg>",
        "<p><img src=\"../Images/p03.png\" alt=\"\"/></p>",
    ];
    for (ix, body) in bodies.iter().enumerate() {
        package.items.push(Item::resource(
            &format!("p{ix:02}_png"),
            &format!("Images/p{ix:02}.png"),
            "image/png",
            png::page(derive(name, ix as u64), 300, 400),
        ));
        let href = format!("Text/p{ix:02}.xhtml");
        package
            .toc
            .push(TocNode::link(format!("Plate {}", ix + 1), href.clone()));
        package.items.push(Item::page(
            &format!("p{ix:02}"),
            &href,
            Kind::Image,
            None,
            image_page(&format!("Plate {}", ix + 1), body),
        ));
    }
    let mut raw = Item::resource(
        "raw",
        "Images/raw.png",
        "image/png",
        png::page(derive(name, 99), 300, 400),
    );
    raw.spine = Spine::Linear;
    raw.kind = Kind::Image;
    package
        .toc
        .push(TocNode::link("Raw plate", "Images/raw.png"));
    package.items.push(raw);
    written(package, "images", true, false, zip)
}

pub fn images_mixed(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "images-mixed";
    let mut package = Package::new(name, "Images Mixed");
    let total = 5;
    let mut cover_image = Item::resource(
        "cover_png",
        "Images/cover.png",
        "image/png",
        png::page(derive(name, 0), 300, 400),
    );
    cover_image.properties = Some("cover-image");
    package.items.push(cover_image);
    package.items.push(Item::resource(
        "plate_png",
        "Images/plate.png",
        "image/png",
        png::page(derive(name, 1), 300, 400),
    ));
    package.items.push(Item::page(
        "cover",
        "Text/cover.xhtml",
        Kind::Image,
        None,
        image_page(
            "Cover",
            "<div><img src=\"../Images/cover.png\" alt=\"Cover\"/></div>",
        ),
    ));
    package.items.push(Item::nav(true));
    package.toc.push(TocNode::link("Cover", "Text/cover.xhtml"));
    for ordinal in 0..total {
        let title = chapter_title(name, ordinal);
        let href = format!("Text/c{ordinal:02}.xhtml");
        package.toc.push(TocNode::link(title.clone(), href.clone()));
        package.items.push(Item::prose(
            &format!("c{ordinal:02}"),
            &href,
            spec(name, ordinal, total, Some(title)),
        ));
        if ordinal == 1 {
            package.toc.push(TocNode::link("Plate", "Text/plate.xhtml"));
            package.items.push(Item::page(
                "plate",
                "Text/plate.xhtml",
                Kind::Image,
                None,
                image_page("Plate", "<img src=\"../Images/plate.png\" alt=\"\"/>"),
            ));
        }
        if ordinal == 2 {
            package.items.push(Item::page("captioned", "Text/captioned.xhtml", Kind::Text, Some("Captioned plate"), image_page("Captioned plate", "<h1>Captioned plate</h1>\n<p><img src=\"../Images/plate.png\" alt=\"\"/></p>\n<p>A caption the reader can select.</p>")));
            package.items.push(Item::page(
                "blank",
                "Text/blank.xhtml",
                Kind::Empty,
                None,
                image_page("Blank", ""),
            ));
        }
    }
    let mut end = Item::page(
        "colophon",
        "Text/colophon.xhtml",
        Kind::Image,
        None,
        image_page("Colophon", "<img src=\"../Images/plate.png\" alt=\"\"/>"),
    );
    end.spine = Spine::NonLinear;
    package.items.push(end);
    written(package, "images", true, true, zip)
}

pub fn toc_none(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "toc-none";
    let mut package = Package::new(name, "Toc None");
    let total = 6;
    for ordinal in 0..total {
        let title = (ordinal != 3).then(|| chapter_title(name, ordinal));
        package.items.push(Item::prose(
            &format!("c{ordinal:02}"),
            &format!("Text/c{ordinal:02}.xhtml"),
            spec(name, ordinal, total, title),
        ));
    }
    written(package, "toc", true, true, zip)
}

pub fn toc_nav_only(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "toc-nav-only";
    let mut package = Package::new(name, "Toc Nav Only");
    package.items.push(Item::nav(false));
    small_chapters(&mut package, name, 6);
    written(package, "toc", true, true, zip)
}

pub fn toc_ncx_only(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "toc-ncx-only";
    let mut package = Package::new(name, "Toc Ncx Only");
    package.version = "2.0";
    package.items.push(Item::ncx());
    small_chapters(&mut package, name, 6);
    written(package, "toc", true, true, zip)
}

fn nested_toc() -> Vec<TocNode> {
    vec![
        TocNode::group(
            "Part One",
            vec![
                TocNode::link("Chapter One", "Text/c00.xhtml"),
                TocNode {
                    label: "Chapter Two".to_owned(),
                    target: Some("Text/c01.xhtml".to_owned()),
                    children: vec![TocNode::link("Scene Two", "Text/c01.xhtml#s2")],
                },
            ],
        ),
        TocNode::group(
            "Part Two",
            vec![
                TocNode::group(
                    "Book Three",
                    vec![
                        TocNode::link("Chapter Three", "Text/c02.xhtml"),
                        TocNode::link("Chapter Four", "Text/c03.xhtml"),
                    ],
                ),
                TocNode::link("Chapter Five", "Text/c04.xhtml"),
            ],
        ),
        TocNode::link("Chapter Six", "Text/c05.xhtml"),
    ]
}

fn nested_chapters(package: &mut Package, name: &'static str) {
    let titles = [
        "Chapter One",
        "Chapter Two",
        "Chapter Three",
        "Chapter Four",
        "Chapter Five",
        "Chapter Six",
    ];
    let total = titles.len() as u32;
    for (ix, title) in titles.iter().enumerate() {
        let ordinal = ix as u32;
        let mut chapter = spec(name, ordinal, total, Some((*title).to_owned()));
        if ordinal == 1 {
            chapter.anchors = &["s2"];
        }
        package.items.push(Item::prose(
            &format!("c{ordinal:02}"),
            &format!("Text/c{ordinal:02}.xhtml"),
            chapter,
        ));
    }
    package.toc = nested_toc();
}

pub fn toc_nested_nav(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "toc-nested-nav";
    let mut package = Package::new(name, "Toc Nested Nav");
    package.items.push(Item::nav(false));
    nested_chapters(&mut package, name);
    written(package, "toc", true, true, zip)
}

pub fn toc_nested_ncx(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "toc-nested-ncx";
    let mut package = Package::new(name, "Toc Nested Ncx");
    package.version = "2.0";
    package.items.push(Item::ncx());
    nested_chapters(&mut package, name);
    written(package, "toc", true, true, zip)
}

pub fn mimetype_not_first(zip: &mut ZipWriter) -> io::Result<Truth> {
    let name = "mimetype-not-first";
    let mut package = Package::new(name, "Mimetype Not First");
    package.mimetype_not_first = true;
    package.items.push(Item::nav(false));
    small_chapters(&mut package, name, 4);
    written(package, "structure", true, true, zip)
}
