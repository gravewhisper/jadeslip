pub mod attack;
pub mod bombs;
pub mod books;
pub mod cleanup;
pub mod epub;
pub mod plants;
pub mod png;
pub mod prose;
pub mod reader;
pub mod rng;
pub mod samples;
pub mod scratch;
pub mod spelling;
pub mod truth;
pub mod zip;

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use truth::Truth;
use zip::ZipWriter;

pub const DEFAULT_PROBE_PORT: u16 = 18765;

#[derive(Clone, Copy)]
pub struct Options {
    pub probe_port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            probe_port: DEFAULT_PROBE_PORT,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Builder {
    Plain(fn(&mut ZipWriter) -> io::Result<Truth>),
    Port(fn(u16, &mut ZipWriter) -> io::Result<Truth>),
}

#[derive(Clone, Copy)]
pub struct BookDef {
    pub name: &'static str,
    pub builder: Builder,
}

const fn plain(name: &'static str, build: fn(&mut ZipWriter) -> io::Result<Truth>) -> BookDef {
    BookDef {
        name,
        builder: Builder::Plain(build),
    }
}

pub const CATALOG: &[BookDef] = &[
    plain("large-h1-p", books::large_h1_p),
    plain("large-h1-div-br", books::large_h1_div_br),
    plain("gbk-xml-decl", books::gbk_xml),
    plain("gbk-meta-charset", books::gbk_meta),
    plain("malformed-markup", books::malformed_markup),
    plain(
        "malformed-missing-chapter",
        books::malformed_missing_chapter,
    ),
    plain("malformed-spine-ref", books::malformed_spine_ref),
    plain("malformed-container-ref", books::malformed_container_ref),
    plain("bomb-ratio", bombs::ratio),
    plain("bomb-oversize-entry", bombs::oversize_entry),
    plain("bomb-book-total", bombs::book_total),
    plain("bomb-overlap-shared", bombs::overlap_shared),
    plain("bomb-overlap-nested", bombs::overlap_nested),
    plain("bomb-lying-sizes", bombs::lying_sizes),
    plain("bomb-lying-extent", bombs::lying_extent),
    plain("bomb-huge-entry-count", bombs::huge_entry_count),
    plain("bomb-traversal-dotdot", bombs::traversal_dotdot),
    plain("bomb-traversal-absolute", bombs::traversal_absolute),
    plain("bomb-traversal-backslash", bombs::traversal_backslash),
    plain("bomb-traversal-nul", bombs::traversal_nul),
    plain("bomb-traversal-opf-href", bombs::traversal_opf_href),
    plain("bomb-traversal-container", bombs::traversal_container),
    plain("bomb-deep-xml-chapter", bombs::deep_chapter),
    plain("bomb-deep-xml-opf", bombs::deep_opf),
    BookDef {
        name: "attack",
        builder: Builder::Port(attack::build),
    },
    plain("images-only", books::images_only),
    plain("images-mixed", books::images_mixed),
    plain("toc-none", books::toc_none),
    plain("toc-nav-only", books::toc_nav_only),
    plain("toc-ncx-only", books::toc_ncx_only),
    plain("toc-nested-nav", books::toc_nested_nav),
    plain("toc-nested-ncx", books::toc_nested_ncx),
    plain("mimetype-not-first", books::mimetype_not_first),
    BookDef {
        name: "toc-hostile",
        builder: Builder::Port(reader::toc_hostile),
    },
    BookDef {
        name: "headings-hostile",
        builder: Builder::Port(reader::headings_hostile),
    },
    plain("reader-css", reader::reader_css),
    plain("cover-epub3", books::cover_epub3),
    plain("cover-epub2", books::cover_epub2),
    plain("planted-junk", cleanup::planted_junk),
];

pub fn generated_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated")
}

pub fn lock_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures.sha256")
}

pub fn generate_book(def: &BookDef, dir: &Path, options: Options) -> io::Result<Truth> {
    let epub_path = dir.join(format!("{}.epub", def.name));
    let temp_path = dir.join(format!(".{}.epub.tmp", def.name));
    let mut zip = ZipWriter::new(File::create(&temp_path)?);
    let truth = match def.builder {
        Builder::Plain(build) => build(&mut zip)?,
        Builder::Port(build) => build(options.probe_port, &mut zip)?,
    };
    zip.finish()?;
    fs::rename(&temp_path, &epub_path)?;
    let mut json = serde_json::to_vec_pretty(&truth).map_err(io::Error::other)?;
    json.push(b'\n');
    fs::write(dir.join(format!("{}.truth.json", def.name)), json)?;
    Ok(truth)
}

pub fn generate(defs: &[BookDef], dir: &Path, options: Options) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let next = AtomicUsize::new(0);
    let failure = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some(def) = defs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Err(error) = generate_book(def, dir, options) {
                        let mut slot = failure.lock().unwrap_or_else(|e| e.into_inner());
                        *slot = Some(io::Error::other(format!("{}: {error}", def.name)));
                    }
                }
            });
        }
    });
    match failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub fn output_files() -> Vec<String> {
    let mut files: Vec<String> = CATALOG
        .iter()
        .flat_map(|def| {
            [
                format!("{}.epub", def.name),
                format!("{}.truth.json", def.name),
            ]
        })
        .collect();
    files.sort();
    files
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn lock_text(dir: &Path) -> io::Result<String> {
    let mut text = String::new();
    for file in output_files() {
        text.push_str(&format!("{}  {file}\n", sha256_file(&dir.join(&file))?));
    }
    Ok(text)
}
