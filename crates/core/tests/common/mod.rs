#![allow(dead_code)]

use serde_json::Value;
use jadeslip_fixtures::{CATALOG, Options, generate, generated_dir, lock_path, sha256_file};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn matches_lockfile(dir: &Path) -> bool {
    let Ok(lock) = std::fs::read_to_string(lock_path()) else {
        return false;
    };
    lock.lines().all(|line| {
        line.split_once("  ").is_some_and(|(hash, file)| {
            sha256_file(&dir.join(file)).is_ok_and(|actual| actual == hash)
        })
    })
}

pub fn fixtures() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = generated_dir();
        if !matches_lockfile(&dir) {
            generate(CATALOG, &dir, Options::default()).expect("fixtures generate");
        }
        dir
    })
}

pub fn fixture(book: &str) -> PathBuf {
    fixtures().join(format!("{book}.epub"))
}

pub fn truth(book: &str) -> Value {
    let path = fixtures().join(format!("{book}.truth.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("truth exists"))
        .expect("truth parses")
}

pub fn books() -> Vec<&'static str> {
    CATALOG.iter().map(|def| def.name).collect()
}

pub fn tiny_book(path: &Path, chapters: &[&str]) {
    use jadeslip_fixtures::zip::{DEFLATE, STORED, ZipWriter};
    let mut manifest = String::new();
    let mut spine = String::new();
    for ix in 0..chapters.len() {
        manifest.push_str(&format!(
            "<item id=\"c{ix}\" href=\"c{ix}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine.push_str(&format!("<itemref idref=\"c{ix}\"/>"));
    }
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\"><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\">Tiny</dc:title></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"
    );
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let mut zip = ZipWriter::new(std::fs::File::create(path).expect("book file"));
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", container.as_bytes(), DEFLATE)
        .unwrap();
    zip.add(b"content.opf", opf.as_bytes(), DEFLATE).unwrap();
    for (ix, body) in chapters.iter().enumerate() {
        let xhtml = format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{body}</body></html>"
        );
        zip.add(format!("c{ix}.xhtml").as_bytes(), xhtml.as_bytes(), DEFLATE)
            .unwrap();
    }
    zip.finish().unwrap();
}

pub fn scratch(name: &str) -> PathBuf {
    jadeslip_fixtures::scratch::scratch(&format!("search-{name}"))
}
