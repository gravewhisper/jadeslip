use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
use std::fs::File;

const CONTAINER: &str = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
const OPF: &str = "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">Worst</dc:title></metadata><manifest><item id=\"c\" href=\"c.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"c\"/></spine></package>";

fn main() {
    let path = std::env::args().nth(1).expect("an output path");
    let unit = std::env::args().nth(2).expect("a repeated markup unit");
    let size = 63usize << 20;
    let mut chapter = String::with_capacity(size + 128);
    chapter.push_str("<html><body><p>");
    while chapter.len() < size {
        chapter.push_str(&unit);
    }
    chapter.push_str("</p></body></html>");
    let mut zip = ZipWriter::new(File::create(path).expect("output opens"));
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", CONTAINER.as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"content.opf", OPF.as_bytes(), DEFLATE).unwrap();
    zip.add(b"c.xhtml", chapter.as_bytes(), DEFLATE).unwrap();
    zip.finish().unwrap();
}
