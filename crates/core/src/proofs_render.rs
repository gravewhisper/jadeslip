use crate::common::{books, fixture, tiny_book, truth};
use crate::markup::{ElementClass, class_of};
use crate::{
    CancelToken, ChapterIx, ChapterKind, DEFAULT_LIMITS, LinkTarget, Reader, RenderedChapter,
    TocSource, VisibleText,
};
use roxmltree::{Document, Node, ParsingOptions};
use serde_json::Value;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const XHTML: &str = "http://www.w3.org/1999/xhtml";
const SVG: &str = "http://www.w3.org/2000/svg";
const MATH: &str = "http://www.w3.org/1998/Math/MathML";
use crate::chapter::BREAK_UNIT;

fn open(book: &str) -> Reader {
    Reader::open(File::open(fixture(book)).unwrap(), DEFAULT_LIMITS).unwrap()
}

fn strict(xhtml: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(
        xhtml,
        ParsingOptions {
            allow_dtd: false,
            ..ParsingOptions::default()
        },
    )
    .map_err(|error| format!("not well-formed at {:?}", error.pos()))
}

fn walk_block(node: Node<'_, '_>, units: &mut Vec<u32>) -> Result<(), String> {
    for child in node.children() {
        if child.is_text() {
            units.extend(child.text().unwrap_or("").encode_utf16().map(u32::from));
        } else if child.is_element() {
            if child.attribute("data-ss-b").is_some() {
                return Err("a block is nested in a block".to_owned());
            }
            if child.attribute("data-ss-w").is_some() {
                return Err("a wrapper is inside a block".to_owned());
            }
            let local = child.tag_name().name();
            if class_of(local) == ElementClass::Replaced {
                continue;
            }
            if local == "br" {
                units.push(BREAK_UNIT);
            }
            walk_block(child, units)?;
        }
    }
    Ok(())
}

fn collect_blocks(node: Node<'_, '_>, blocks: &mut Vec<Vec<u32>>) -> Result<(), String> {
    if let Some(number) = node.attribute("data-ss-b") {
        if number != blocks.len().to_string() {
            return Err(format!("block {number} out of order at {}", blocks.len()));
        }
        let mut units = Vec::new();
        if class_of(node.tag_name().name()) != ElementClass::Replaced {
            walk_block(node, &mut units)?;
        }
        blocks.push(units);
        return Ok(());
    }
    for child in node.children() {
        if child.is_text() {
            let gap = child.text().unwrap_or("");
            if !gap
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
            {
                return Err("visible text outside every block".to_owned());
            }
        } else if child.is_element() {
            collect_blocks(child, blocks)?;
        }
    }
    Ok(())
}

fn dom_blocks(xhtml: &str) -> Result<Vec<Vec<u32>>, String> {
    let document = strict(xhtml)?;
    let root = document.root_element();
    if root.tag_name().name() != "div" || root.tag_name().namespace() != Some(XHTML) {
        return Err("the root is not one XHTML div".to_owned());
    }
    let mut blocks = Vec::new();
    collect_blocks(root, &mut blocks)?;
    Ok(blocks)
}

fn check_dom_text(xhtml: &str, text: &VisibleText) -> Result<(u64, u64), String> {
    text.check_dom_blocks(&dom_blocks(xhtml)?)
}

fn check_kind(rendered: &RenderedChapter, text: &VisibleText) -> Result<(), String> {
    let has_text = !text
        .as_str()
        .trim_matches(|ch: char| ch.is_ascii_whitespace() || ch == '\u{C}')
        .is_empty();
    if has_text != (rendered.kind == ChapterKind::Text) {
        return Err(format!(
            "kind is {:?} and visible text present is {has_text}",
            rendered.kind
        ));
    }
    Ok(())
}

fn check_chapter<R: crate::ReadAt>(
    reader: &Reader<R>,
    ix: ChapterIx,
) -> Result<(u64, u64), String> {
    let rendered = reader.render(ix).map_err(|error| error.user_message())?;
    let text = reader
        .visible_text(ix)
        .map_err(|error| error.user_message())?;
    check_kind(&rendered, &text)?;
    check_dom_text(&rendered.xhtml, &text)
}

struct Totals {
    chapters: u64,
    blocks: u64,
    units: u64,
    failed_to_load: u64,
    failures: Vec<String>,
}

fn sweep<R: crate::ReadAt>(reader: &Reader<R>, label: &str) -> Totals {
    let rows = reader.rows();
    let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
    let chunk = rows.len().div_ceil(threads).max(1);
    let results: Vec<Totals> = std::thread::scope(|scope| {
        let workers: Vec<_> = rows
            .chunks(chunk)
            .map(|rows| {
                scope.spawn(move || {
                    let mut totals = Totals {
                        chapters: 0,
                        blocks: 0,
                        units: 0,
                        failed_to_load: 0,
                        failures: Vec::new(),
                    };
                    for row in rows {
                        if reader.extract(row.ix).is_err() || reader.visible_text(row.ix).is_err() {
                            totals.failed_to_load += 1;
                            continue;
                        }
                        match check_chapter(reader, row.ix) {
                            Ok((blocks, units)) => {
                                totals.chapters += 1;
                                totals.blocks += blocks;
                                totals.units += units;
                            }
                            Err(why) => {
                                totals.failures.push(format!("{label} {}: {why}", row.ix.0))
                            }
                        }
                    }
                    totals
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });
    let mut merged = Totals {
        chapters: 0,
        blocks: 0,
        units: 0,
        failed_to_load: 0,
        failures: Vec::new(),
    };
    for totals in results {
        merged.chapters += totals.chapters;
        merged.blocks += totals.blocks;
        merged.units += totals.units;
        merged.failed_to_load += totals.failed_to_load;
        merged.failures.extend(totals.failures);
    }
    merged
}

fn opens(book: &str) -> Option<Reader> {
    Reader::open(File::open(fixture(book)).unwrap(), DEFAULT_LIMITS).ok()
}

#[test]
fn every_fixture_chapter_is_strict_xml_and_its_dom_text_matches_the_model() {
    let mut totals = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for book in books() {
        let Some(reader) = opens(book) else { continue };
        let swept = sweep(&reader, book);
        totals.0 += swept.chapters;
        totals.1 += swept.blocks;
        totals.2 += swept.units;
        totals.3 += swept.failed_to_load;
        failures.extend(swept.failures);
    }
    println!(
        "fixtures: chapters {}, blocks {}, utf16 units {}, refused to load {}, failures {}",
        totals.0,
        totals.1,
        totals.2,
        totals.3,
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{}",
        failures[..failures.len().min(20)].join("\n")
    );
    assert!(totals.0 > 12_000, "the sweep reached the large books");
}

fn corpus_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn sanitized_source(source: &str) -> Result<String, String> {
    use crate::chapter::BlockLayout;
    use crate::markup::scan;
    use crate::sanitize::{Context, Layout, ResourceUrls, UrlMap, sanitize};
    use crate::zip::EntryName;
    let scanned = scan(source, &DEFAULT_LIMITS).map_err(|error| error.user_message())?;
    let (layout, ranges) = BlockLayout::build(&scanned.events, source);
    let serveable = |_: &EntryName| true;
    let no_sheets = |_: &EntryName| false;
    let context = Context {
        urls: ResourceUrls {
            map: UrlMap::new(1, EntryName::parse("c.xhtml").unwrap()),
            serveable: &serveable,
        },
        is_stylesheet: &no_sheets,
    };
    Ok(sanitize(
        &scanned.events,
        source.len(),
        &Layout {
            blocks: &layout.blocks,
            ranges: &ranges,
            body_inner: layout.body_inner,
        },
        &context,
    )
    .xhtml)
}

fn check_source(source: &str) -> Result<(), String> {
    let Ok(xhtml) = sanitized_source(source) else {
        return Ok(());
    };
    let model = crate::chapter::model_of_source(
        source.to_owned(),
        crate::chapter::SourceEncoding::Utf8 { bom: false },
        &DEFAULT_LIMITS,
    )
    .map_err(|error| error.user_message())?;
    check_dom_text(&xhtml, &model.text).map(|_| ())
}

#[test]
fn every_regression_corpus_file_renders_strict_and_matches_the_model() {
    let paths = corpus_paths();
    assert!(paths.len() > 20);
    for path in paths {
        let raw = std::fs::read(&path).unwrap();
        let (source, _) = crate::chapter::decode_source(raw);
        check_source(&source).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
    }
}

const SOUP: &[&str] = &[
    "<p>",
    "</p>",
    "<div>",
    "</div>",
    "<body>",
    "</body>",
    "<html>",
    "</html>",
    "<head>",
    "</head>",
    "<br>",
    "<br/>",
    "<img src=a.png>",
    "<svg>",
    "</svg>",
    "<math>",
    "</math>",
    "<text>x</text>",
    "<mi>y</mi>",
    "<picture>",
    "</picture>",
    "<video>v</video>",
    "<canvas>",
    "</canvas>",
    "<pre>",
    "</pre>",
    "<style>",
    "</style>",
    "<script>",
    "</script>",
    "<title>",
    "</title>",
    "<b>",
    "</b>",
    "<a href=x>",
    "</a>",
    "<h1>",
    "</h1>",
    "<li>",
    "<ul>",
    "</ul>",
    "<table>",
    "<tr>",
    "<td>",
    "<form>",
    "</form>",
    "<foo>",
    "</foo>",
    "<x:p>",
    "</x:p>",
    "text",
    " ",
    "\n",
    "\r\n",
    "\r",
    "\t",
    "\u{C}",
    "\u{1}",
    "\u{FFFE}",
    "é",
    "𝄞",
    "&amp;",
    "&lt;",
    "&#10;",
    "&#13;",
    "&#32;",
    "&#160;",
    "&nbsp;",
    "&bogus;",
    "&",
    "<",
    ">",
    "]]>",
    "<![CDATA[",
    "<![CDATA[c\rd]]>",
    "<!--",
    "-->",
    "<?pi?>",
    "<!DOCTYPE x>",
    "<p id=a id=b>",
    "<p style='color:red;background:url(x)'>",
    "<p ONCLICK=x>",
    "<q title='\u{1}\"&'>",
    "<",
];

fn soup_document(rng: &mut jadeslip_fixtures::rng::Rng, len: usize) -> String {
    let mut out = String::new();
    for _ in 0..len {
        out.push_str(SOUP[rng.below(SOUP.len())]);
    }
    out
}

fn soup_runs(runs: u64, base: u64) -> Vec<String> {
    let mut failures = Vec::new();
    for run in 0..runs {
        let mut rng = jadeslip_fixtures::rng::Rng::for_item("render-soup", base + run);
        let len = rng.below(60);
        let source = soup_document(&mut rng, len);
        if let Err(why) = check_source(&source) {
            failures.push(format!("{why}: {source:?}"));
            if failures.len() > 5 {
                break;
            }
        }
    }
    failures
}

#[test]
fn seeded_tag_soup_always_renders_strict_and_matches_the_model() {
    let failures = soup_runs(20_000, 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "long soup: SPINE_SOUP_RUNS=2000000 cargo test --release -p jadeslip-core --lib long_tag_soup -- --ignored --nocapture"]
fn long_tag_soup_always_renders_strict_and_matches_the_model() {
    let runs = std::env::var("SPINE_SOUP_RUNS").map_or(1_000_000, |value| value.parse().unwrap());
    let threads = std::thread::available_parallelism().unwrap().get() as u64;
    let started = Instant::now();
    let failures: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|worker| scope.spawn(move || soup_runs(runs / threads, 1 << 32 | worker << 40)))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    });
    println!(
        "soup: {runs} documents in {:.1}s, failures {}",
        started.elapsed().as_secs_f64(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{}",
        failures[..failures.len().min(5)].join("\n")
    );
}

fn xmllint_accepts(xhtml: &str) -> bool {
    use std::io::Write;
    let mut child = Command::new("xmllint")
        .args(["--noout", "--nonet", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("xmllint is installed");
    let mut stdin = child.stdin.take().unwrap();
    let wrote = stdin.write_all(xhtml.as_bytes()).is_ok();
    drop(stdin);
    child.wait().unwrap().success() && wrote
}

fn xmllint_sweep<R: crate::ReadAt>(reader: &Reader<R>) -> (u64, u64) {
    let rows = reader.rows();
    let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
    let chunk = rows.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = rows
            .chunks(chunk)
            .map(|rows| {
                scope.spawn(move || {
                    let (mut passed, mut failed) = (0, 0);
                    for row in rows {
                        if let Ok(rendered) = reader.render(row.ix) {
                            if xmllint_accepts(&rendered.xhtml) {
                                passed += 1;
                            } else {
                                failed += 1;
                            }
                        }
                    }
                    (passed, failed)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .fold((0, 0), |sum, one| (sum.0 + one.0, sum.1 + one.1))
    })
}

#[test]
fn xmllint_rejects_a_broken_document_so_its_verdict_means_something() {
    assert!(xmllint_accepts("<a/>"));
    assert!(!xmllint_accepts("<a>"));
}

#[test]
fn xmllint_accepts_every_small_fixture_render() {
    let mut passed = 0;
    for book in books() {
        let over_libxml2_text_node_limit = book == "bomb-ratio";
        if book.starts_with("large-") || over_libxml2_text_node_limit {
            continue;
        }
        let Some(reader) = opens(book) else { continue };
        let (ok, failed) = xmllint_sweep(&reader);
        assert_eq!(failed, 0, "{book}");
        passed += ok;
    }
    println!("xmllint small fixtures: {passed} passed");
    assert!(passed > 150);
}

#[test]
#[ignore = "all large fixtures through xmllint: cargo test --release -p jadeslip-core --lib xmllint_accepts_the_large -- --ignored --nocapture"]
fn xmllint_accepts_the_large_fixtures() {
    for book in ["large-h1-p", "large-h1-div-br"] {
        let (passed, failed) = xmllint_sweep(&open(book));
        println!("xmllint {book}: {passed} passed, {failed} failed");
        assert_eq!(failed, 0);
    }
}

fn real_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "epub")
    });
    paths.sort();
    paths
}

fn book_id(path: &Path) -> String {
    let stem = path.file_stem().unwrap().to_string_lossy();
    stem.rsplit('-').next().unwrap_or("").to_owned()
}

#[test]
#[ignore = "real books, counts only: cargo test --release -p jadeslip-core --lib real_books_render -- --ignored --nocapture"]
fn real_books_render_strict_and_match_the_model() {
    let paths = real_paths();
    assert!(!paths.is_empty());
    println!(
        "book     rows  checked blocks  utf16_units xmllint_ok xmllint_bad refused text image empty toc toc_source"
    );
    let mut failures = 0;
    for path in &paths {
        let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
        let swept = sweep(&reader, "");
        failures += swept.failures.len();
        let shown: Vec<String> = swept
            .failures
            .iter()
            .take(5)
            .map(|why| why.split(':').nth(1).unwrap_or("").to_owned())
            .collect();
        let (lint_ok, lint_bad) = xmllint_sweep(&reader);
        failures += lint_bad as usize;
        let mut kinds = [0u32; 3];
        for row in reader.rows() {
            if let Ok(rendered) = reader.render(row.ix) {
                kinds[match rendered.kind {
                    ChapterKind::Text => 0,
                    ChapterKind::ImagePage => 1,
                    ChapterKind::Empty => 2,
                }] += 1;
            }
        }
        let toc = reader.toc().unwrap();
        println!(
            "{:8} {:5} {:7} {:7} {:11} {:10} {:11} {:7} {:4} {:5} {:5} {:5} {:?}",
            book_id(path),
            reader.rows().len(),
            swept.chapters,
            swept.blocks,
            swept.units,
            lint_ok,
            lint_bad,
            swept.failed_to_load,
            kinds[0],
            kinds[1],
            kinds[2],
            toc.entries.len(),
            toc.source,
        );
        for why in shown {
            println!("  failure:{why}");
        }
    }
    assert_eq!(failures, 0);
}

fn target_label(reader: &Reader, entry: &crate::TocEntry) -> Option<String> {
    let ix = entry.target?;
    let href = reader.rows()[ix.0 as usize].href.as_ref()?.as_str();
    let suffix = href.strip_prefix("OEBPS/").unwrap_or(href);
    Some(match &entry.fragment {
        Some(fragment) => format!("{suffix}#{fragment}"),
        None => suffix.to_owned(),
    })
}

fn check_toc_against_truth(book: &str, source: TocSource) {
    let reader = open(book);
    let toc = reader.toc().unwrap();
    assert_eq!(toc.source, source, "{book}");
    let want = truth(book);
    let rows = want["toc"].as_array().unwrap();
    assert_eq!(toc.entries.len(), rows.len(), "{book}: entry count");
    for (entry, row) in toc.entries.iter().zip(rows) {
        assert_eq!(entry.label, row["label"].as_str().unwrap(), "{book}");
        assert_eq!(
            u64::from(entry.depth),
            row["depth"].as_u64().unwrap(),
            "{book}: {}",
            entry.label
        );
        assert_eq!(
            target_label(&reader, entry).as_deref(),
            row["target"].as_str(),
            "{book}: {}",
            entry.label
        );
    }
}

#[test]
fn toc_entries_match_the_sidecars_for_nav_and_ncx() {
    check_toc_against_truth("toc-nested-nav", TocSource::Nav);
    check_toc_against_truth("toc-nested-ncx", TocSource::Ncx);
    check_toc_against_truth("toc-nav-only", TocSource::Nav);
    check_toc_against_truth("toc-ncx-only", TocSource::Ncx);
    check_toc_against_truth("images-only", TocSource::Nav);
    check_toc_against_truth("attack", TocSource::Nav);
}

#[test]
fn nested_toc_keeps_parents_without_links() {
    for book in ["toc-nested-nav", "toc-nested-ncx"] {
        let reader = open(book);
        let toc = reader.toc().unwrap();
        let shape: Vec<(u16, &str, bool)> = toc
            .entries
            .iter()
            .map(|entry| (entry.depth, entry.label.as_str(), entry.target.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                (0, "Part One", false),
                (1, "Chapter One", true),
                (1, "Chapter Two", true),
                (2, "Scene Two", true),
                (0, "Part Two", false),
                (1, "Book Three", false),
                (2, "Chapter Three", true),
                (2, "Chapter Four", true),
                (1, "Chapter Five", true),
                (0, "Chapter Six", true),
            ],
            "{book}"
        );
        assert_eq!(toc.entries[3].fragment.as_deref(), Some("s2"), "{book}");
        assert_eq!(toc.entries[3].target, toc.entries[2].target, "{book}");
    }
}

#[test]
fn a_book_with_no_toc_gets_placeholders_and_heading_titles() {
    let reader = open("toc-none");
    let toc = reader.toc().unwrap();
    assert_eq!(toc.source, TocSource::HeadingsPending);
    let want = truth("toc-none");
    let spine = want["spine"].as_array().unwrap();
    assert_eq!(toc.entries.len(), spine.len());
    assert_eq!(toc.entries[0].label, "Chapter 1");
    assert_eq!(toc.entries[5].target, Some(ChapterIx(5)));

    let mut titles = Vec::new();
    reader.heading_titles(&CancelToken::default(), &mut |ix, title| {
        titles.push((ix.0, title))
    });
    let expected: Vec<(u32, String)> = spine
        .iter()
        .filter_map(|row| {
            Some((
                row["ix"].as_u64()? as u32,
                row["title"].as_str()?.to_owned(),
            ))
        })
        .collect();
    assert_eq!(titles, expected);
    assert_eq!(expected.len(), 5, "chapter 3 has no heading");

    let cancel = CancelToken::default();
    let mut seen = 0;
    reader.heading_titles(&cancel, &mut |_, _| {
        seen += 1;
        cancel.cancel();
    });
    assert_eq!(seen, 1, "cancelling stops the stream");
}

#[test]
fn start_chapter_follows_the_toc_and_skips_the_nav_document() {
    let reader = open("images-mixed");
    assert_eq!(
        reader.start_chapter().unwrap(),
        reader.toc().unwrap().entries[0].target.unwrap()
    );
    let reader = open("toc-none");
    assert_eq!(reader.start_chapter().unwrap(), ChapterIx(0));
    let reader = open("toc-nested-nav");
    assert_eq!(reader.start_chapter().unwrap(), ChapterIx(0));
    let reader = open("large-h1-p");
    let start = reader.start_chapter().unwrap();
    assert!(!reader.rows()[start.0 as usize].is_nav);
}

#[test]
fn kinds_and_titles_match_the_sidecars() {
    for book in books() {
        if book.starts_with("bomb-") || book.starts_with("large-") {
            continue;
        }
        let Some(reader) = opens(book) else { continue };
        let want = truth(book);
        for row in want["spine"].as_array().unwrap() {
            let ix = ChapterIx(row["ix"].as_u64().unwrap() as u32);
            let Ok(rendered) = reader.render(ix) else {
                continue;
            };
            let kind = match row["kind"].as_str().unwrap() {
                "text" | "nav" => ChapterKind::Text,
                "image" => ChapterKind::ImagePage,
                "empty" => ChapterKind::Empty,
                other => panic!("{book}: unknown kind {other}"),
            };
            assert_eq!(rendered.kind, kind, "{book} chapter {}", ix.0);
            if let Some(title) = row["title"].as_str()
                && kind == ChapterKind::Text
            {
                assert_eq!(
                    rendered.title.as_deref(),
                    Some(title),
                    "{book} chapter {}",
                    ix.0
                );
            }
        }
    }
}

#[test]
fn links_are_classified_in_rust() {
    let reader = open("toc-nested-nav");
    let from = ChapterIx(0);
    assert_eq!(
        reader.link(from, "c01.xhtml#s2"),
        LinkTarget::Chapter {
            ix: ChapterIx(1),
            fragment: Some("s2".to_owned())
        }
    );
    assert_eq!(
        reader.link(from, "#top"),
        LinkTarget::Chapter {
            ix: from,
            fragment: Some("top".to_owned())
        }
    );
    assert_eq!(
        reader.link(from, "./c02.xhtml"),
        LinkTarget::Chapter {
            ix: ChapterIx(2),
            fragment: None
        }
    );
    assert_eq!(
        reader.link(from, " https://x.test/a "),
        LinkTarget::External {
            url: "https://x.test/a".to_owned()
        }
    );
    for blocked in [
        "javascript:x",
        "../../../etc/passwd",
        "nothere.xhtml",
        "file:///etc/hostname",
        "ipc://localhost/x",
    ] {
        assert_eq!(reader.link(from, blocked), LinkTarget::Blocked, "{blocked}");
    }
    assert_eq!(
        reader.link(ChapterIx(999), "c01.xhtml"),
        LinkTarget::Blocked
    );
}

#[test]
fn resources_serve_only_listed_non_documents_under_limits() {
    let reader = open("attack");
    let png = reader.resource("OEBPS/Res/safe.png").unwrap();
    assert_eq!(png.media_type, "image/png");
    assert!(png.bytes.starts_with(b"\x89PNG"));
    for refused in [
        "OEBPS/Text/control_plain.xhtml",
        "OEBPS/Res/evil.js",
        "OEBPS/Res/evil.css",
        "OEBPS/Res/unlisted.png",
        "OEBPS/content.opf",
        "META-INF/container.xml",
        "mimetype",
        "../../etc/hostname",
        "/etc/hostname",
        "",
    ] {
        assert!(reader.resource(refused).is_err(), "{refused} was served");
    }
}

#[test]
fn a_resource_over_the_entry_limit_is_refused() {
    use jadeslip_fixtures::zip::{STORED, ZipWriter};
    let container = "<container><rootfiles><rootfile full-path=\"a.opf\"/></rootfiles></container>";
    let opf = "<package><manifest><item id=\"p\" href=\"p.png\" media-type=\"image/png\"/><item id=\"c\" href=\"c.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"c\"/></spine></package>";
    let path = jadeslip_fixtures::scratch::scratch("limit").join("limit.epub");
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    zip.add(b"mimetype", b"application/epub+zip", STORED)
        .unwrap();
    zip.add(b"META-INF/container.xml", container.as_bytes(), STORED)
        .unwrap();
    zip.add(b"a.opf", opf.as_bytes(), STORED).unwrap();
    zip.add(b"c.xhtml", b"<p>c</p>", STORED).unwrap();
    zip.add(b"p.png", &vec![0u8; 8192], STORED).unwrap();
    zip.finish().unwrap();
    let limits = crate::Limits {
        max_entry_inflated: 4096,
        ..DEFAULT_LIMITS
    };
    let reader = Reader::open(File::open(&path).unwrap(), limits).unwrap();
    let refused = reader.resource("p.png");
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(refused, Err(crate::Error::Limit(_))));
}

#[test]
fn svg_resources_are_served_with_their_active_parts_removed() {
    let reader = open("attack");
    for file in [
        "evil_img",
        "evil_iframe",
        "evil_object",
        "evil_embed",
        "evil_nav",
        "evil_cssbg",
    ] {
        let resource = reader.resource(&format!("OEBPS/Res/{file}.svg")).unwrap();
        assert_eq!(resource.media_type, "image/svg+xml");
        let text = String::from_utf8(resource.bytes).unwrap();
        strict(&text).unwrap_or_else(|why| panic!("{file}: {why}"));
        assert!(text.contains("<rect"), "{file}: the drawing survives");
        for gone in [
            "<script",
            "onload",
            "pwned",
            "127.0.0.1",
            "<image href=\"http",
        ] {
            assert!(!text.contains(gone), "{file} still has {gone}");
        }
    }
}

#[test]
fn book_scheme_urls_round_trip_through_the_reader() {
    let reader = open("attack");
    let rendered = reader.render(ChapterIx(0)).unwrap();
    let document = strict(&rendered.xhtml).unwrap();
    let src = document
        .descendants()
        .find_map(|node| node.attribute("src"))
        .expect("the control chapter keeps its image")
        .to_owned();
    assert_eq!(
        reader.entry_of_url(&src).as_deref(),
        Some("OEBPS/Res/safe.png")
    );
    assert!(reader.resource(&reader.entry_of_url(&src).unwrap()).is_ok());
    assert_eq!(
        reader.entry_of_url("jadeslip-book://forged-token/OEBPS/Res/safe.png"),
        None
    );
    let other = open("attack");
    assert_ne!(
        reader.session(),
        other.session(),
        "sessions differ per open"
    );
    assert_eq!(other.entry_of_url(&src), None);
}

#[test]
fn a_shelf_cover_url_in_a_chapter_never_reaches_the_page() {
    let path = jadeslip_fixtures::scratch::scratch("cover-url").join("book.epub");
    let cover = "jadeslip-book://cover/0123456789abcdef0123456789abcdef?v=1";
    tiny_book(
        &path,
        &[&format!(
            "<p>kept one</p><img src=\"{cover}\" alt=\"\"/><p style=\"background-image:url('{cover}')\">kept two</p><a href=\"{cover}\">kept three</a><svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"{cover}\"/></svg>"
        )],
    );
    let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
    let rendered = reader.render(ChapterIx(0)).unwrap();
    strict(&rendered.xhtml).unwrap();
    for kept in ["kept one", "kept two", "kept three"] {
        assert!(rendered.xhtml.contains(kept), "{kept}");
    }
    assert!(
        !rendered.xhtml.contains("cover/0123"),
        "a cover URL passed through"
    );
    assert_eq!(reader.entry_of_url(cover), None);
}

#[derive(Clone, Copy)]
struct Proof {
    gone: &'static [&'static str],
    text_only: &'static [&'static str],
    kept: &'static [&'static str],
    link: Option<LinkExpect>,
    inert_because: &'static str,
}

#[derive(Clone, Copy)]
enum LinkExpect {
    External,
    Blocked,
}

const fn gone(gone: &'static [&'static str]) -> Proof {
    Proof {
        gone,
        text_only: &[],
        kept: &[],
        link: None,
        inert_because: "",
    }
}

const fn gone_kept(gone: &'static [&'static str], kept: &'static [&'static str]) -> Proof {
    Proof {
        gone,
        text_only: &[],
        kept,
        link: None,
        inert_because: "",
    }
}

const fn as_text(text_only: &'static [&'static str]) -> Proof {
    Proof {
        gone: &[],
        text_only,
        kept: &[],
        link: None,
        inert_because: "",
    }
}

const fn anchor(
    gone: &'static [&'static str],
    link: LinkExpect,
    inert_because: &'static str,
) -> Proof {
    Proof {
        gone,
        text_only: &[],
        kept: &["<a "],
        link: Some(link),
        inert_because,
    }
}

const ATTACK_TABLE: &[(&str, Proof)] = &[
    (
        "control_plain",
        Proof {
            gone: &[],
            text_only: &[],
            kept: &["<img src=\"jadeslip-book://", "href=\"#top\""],
            link: None,
            inert_because: "control row",
        },
    ),
    ("script_inline_head", gone(&["<script", "pwned"])),
    ("script_inline_body", gone(&["<script", "pwned"])),
    ("script_cdata", gone(&["<script", "pwned", "cdata"])),
    ("script_src_remote", gone(&["<script", "/r/"])),
    ("script_src_local", gone(&["<script", "evil.js"])),
    (
        "script_svg_inline",
        gone_kept(&["<script", "pwned"], &["<svg"]),
    ),
    (
        "script_svg_xlink",
        gone_kept(&["<script", "xlink", "/r/"], &["<svg"]),
    ),
    ("script_namespaced", gone(&["script", "pwned"])),
    (
        "event_body_onload",
        gone_kept(&["onload", "onerror", "pwned", "x-invalid"], &["<img"]),
    ),
    ("event_body_attr", gone(&["onload", "pwned"])),
    (
        "event_img_onerror",
        gone_kept(&["onerror", "pwned", "x-invalid"], &["<img"]),
    ),
    (
        "event_img_onerror_upper",
        gone_kept(&["onerror", "pwned"], &["<img"]),
    ),
    (
        "event_svg_onload",
        gone_kept(&["onload", "pwned"], &["<svg"]),
    ),
    (
        "event_svg_animate_onbegin",
        gone_kept(&["<animate", "onbegin", "pwned"], &["<svg"]),
    ),
    (
        "event_details_ontoggle",
        gone_kept(&["ontoggle", "pwned"], &["<details"]),
    ),
    (
        "event_video_onerror",
        gone_kept(&["onerror", "pwned", "x-invalid"], &["<video"]),
    ),
    (
        "event_audio_onerror",
        gone_kept(&["onerror", "pwned", "x-invalid"], &["<audio"]),
    ),
    (
        "event_input_onfocus",
        gone(&["<input", "onfocus", "autofocus", "pwned"]),
    ),
    (
        "event_marquee_onstart",
        gone(&["<marquee", "onstart", "pwned"]),
    ),
    ("event_style_onload", gone(&["onload", "pwned"])),
    (
        "frame_iframe_javascript",
        gone(&["<iframe", "javascript", "/s/"]),
    ),
    ("frame_iframe_srcdoc", gone(&["<iframe", "srcdoc", "pwned"])),
    ("frame_iframe_data", gone(&["<iframe", "data:", "pwned"])),
    ("frame_iframe_data_base64", gone(&["<iframe", "base64"])),
    ("frame_iframe_remote", gone(&["<iframe", "/r/"])),
    ("frame_object_javascript", gone(&["<object", "javascript"])),
    ("frame_object_remote", gone(&["<object", "/r/"])),
    ("frame_embed_javascript", gone(&["<embed", "javascript"])),
    ("frame_embed_remote", gone(&["<embed", "/r/"])),
    (
        "svg_local_img",
        Proof {
            gone: &["onload"],
            text_only: &[],
            kept: &["<img src=\"jadeslip-book://"],
            link: None,
            inert_because: "the served SVG has no script, handler or remote image, proved in svg_resources_are_served_with_their_active_parts_removed",
        },
    ),
    ("svg_local_iframe", gone(&["<iframe", "evil_iframe"])),
    ("svg_local_object", gone(&["<object", "evil_object"])),
    ("svg_local_embed", gone(&["<embed", "evil_embed"])),
    ("resource_img", gone_kept(&["/r/"], &["<img"])),
    (
        "resource_img_srcset",
        gone_kept(&["/r/", "srcset"], &["<img"]),
    ),
    (
        "resource_picture_source",
        gone_kept(
            &["<source", "/r/", "srcset", "data:"],
            &["<picture", "<img"],
        ),
    ),
    (
        "resource_video_poster",
        gone_kept(&["poster", "/r/"], &["<video"]),
    ),
    (
        "resource_video_src",
        gone_kept(&["src=", "preload", "/r/"], &["<video"]),
    ),
    (
        "resource_video_source",
        gone_kept(&["<source", "/r/"], &["<video"]),
    ),
    (
        "resource_audio_src",
        gone_kept(&["src=", "preload", "/r/"], &["<audio"]),
    ),
    ("resource_track", gone_kept(&["<track", "/r/"], &["<video"])),
    ("resource_input_image", gone(&["<input", "/r/"])),
    (
        "resource_svg_image",
        gone_kept(&["href", "/r/"], &["<image"]),
    ),
    (
        "resource_svg_image_xlink",
        gone_kept(&["href", "xlink", "/r/"], &["<image"]),
    ),
    ("resource_svg_use", gone_kept(&["<use", "/r/"], &["<svg"])),
    (
        "resource_svg_foreign_object",
        gone_kept(&["foreignobject", "<img", "/r/"], &["<svg"]),
    ),
    ("resource_link_stylesheet", gone(&["<link", "/r/"])),
    ("resource_link_prefetch", gone(&["<link", "/r/"])),
    ("resource_link_preload", gone(&["<link", "/r/"])),
    ("resource_link_icon", gone(&["<link", "/r/"])),
    ("resource_link_body", gone(&["<link", "/r/"])),
    (
        "css_file_import",
        gone_kept(&["<link", "@import", "/r/"], &["id=\"cssfile\""]),
    ),
    ("css_import", gone(&["@import", "/r/"])),
    ("css_import_bare", gone(&["@import", "/r/"])),
    (
        "css_font_face",
        gone_kept(&["@font-face", "/r/"], &[".ss-book p{font-family:evil}"]),
    ),
    ("css_background", gone_kept(&["url(", "/r/"], &["styled"])),
    ("css_cursor", gone_kept(&["cursor", "/r/"], &["styled"])),
    ("css_list_style", gone(&["list-style-image", "/r/"])),
    (
        "css_border_image",
        gone_kept(&["border-image", "/r/"], &["border:1px solid"]),
    ),
    ("css_mask", gone_kept(&["mask", "/r/"], &["styled"])),
    ("css_filter", gone_kept(&["filter", "/r/"], &["styled"])),
    (
        "css_image_set",
        gone_kept(&["image-set", "/r/"], &["styled"]),
    ),
    (
        "css_escaped_url",
        gone_kept(&["url(", "\\75", "/r/"], &["styled"]),
    ),
    (
        "css_comment_split",
        gone_kept(&["@im", "rl(", "/r/"], &["styled"]),
    ),
    (
        "css_custom_property",
        gone_kept(&["--x", "var(", "/r/"], &["styled"]),
    ),
    (
        "css_attribute_exfil",
        gone_kept(&["url(", "/r/"], &["title=\"secret\""]),
    ),
    ("css_inline_url", gone_kept(&["style=", "/r/"], &["styled"])),
    (
        "css_inline_entity_url",
        gone_kept(&["style=", "&#117;", "/r/"], &["styled"]),
    ),
    (
        "css_javascript_url",
        gone_kept(&["style=", "javascript"], &["styled"]),
    ),
    (
        "css_svg_background",
        Proof {
            gone: &["127.0.0.1"],
            text_only: &[],
            kept: &["url(\"jadeslip-book://"],
            link: None,
            inert_because: "the sheet can load only the sanitised SVG from the book scheme",
        },
    ),
    (
        "css_overlay_fixed",
        gone_kept(&["position"], &["id=\"fixedov\"", "over"]),
    ),
    (
        "css_host_escape",
        Proof {
            gone: &[":host", ":root"],
            text_only: &[],
            kept: &[
                ".ss-book .ss-body{display:none}",
                ".ss-book *{visibility:hidden}",
            ],
            link: None,
            inert_because: "every surviving selector is scoped under .ss-book, so it can hide only this chapter",
        },
    ),
    (
        "doc_base",
        Proof {
            gone: &["<base", "/r/", "src="],
            text_only: &[],
            kept: &["<img", "<a href=\"relative\""],
            link: Some(LinkExpect::Blocked),
            inert_because: "the relative link resolves to nothing in the book",
        },
    ),
    (
        "doc_meta_refresh_remote",
        gone(&["<meta", "refresh", "/r/"]),
    ),
    (
        "doc_meta_refresh_javascript",
        gone(&["<meta", "javascript"]),
    ),
    (
        "doc_meta_csp_relax",
        gone(&["<meta", "<script", "unsafe-inline", "pwned"]),
    ),
    ("doc_xml_stylesheet", gone(&["<?xml", "/r/"])),
    ("doc_doctype_external", gone(&["doctype", "/r/"])),
    (
        "doc_entity_expansion",
        Proof {
            gone: &["<!doctype", "lollollol"],
            text_only: &["&amp;lol4;"],
            kept: &[],
            link: None,
            inert_because: "the entity is never declared in the output, so it stays literal text",
        },
    ),
    (
        "doc_entity_external_file",
        Proof {
            gone: &["<!doctype", "file:"],
            text_only: &["&amp;ext;"],
            kept: &[],
            link: None,
            inert_because: "the entity is never declared in the output, so it stays literal text",
        },
    ),
    (
        "doc_entity_external_remote",
        Proof {
            gone: &["<!doctype", "/r/"],
            text_only: &["&amp;ext;"],
            kept: &[],
            link: None,
            inert_because: "the entity is never declared in the output, so it stays literal text",
        },
    ),
    ("doc_xinclude", gone(&["include", "file:", "hostname"])),
    ("scheme_img_ipc", gone_kept(&["ipc:", "pwned"], &["<img"])),
    ("scheme_iframe_ipc", gone(&["<iframe", "ipc:"])),
    ("scheme_iframe_tauri", gone(&["<iframe", "tauri:"])),
    ("scheme_iframe_app", gone(&["<iframe", "jadeslip:"])),
    (
        "scheme_img_file",
        gone_kept(&["file:", "hostname"], &["<img"]),
    ),
    (
        "scheme_iframe_file",
        gone(&["<iframe", "file:", "hostname"]),
    ),
    (
        "scheme_img_book_traversal",
        gone_kept(&["forged-token", "hostname", "src="], &["<img"]),
    ),
    (
        "scheme_img_relative_traversal",
        gone_kept(&["../", "hostname", "src="], &["<img"]),
    ),
    (
        "scheme_img_chapter_as_image",
        gone_kept(&["control_plain", "src="], &["<img"]),
    ),
    (
        "scheme_img_unlisted",
        gone_kept(&["unlisted", "src="], &["<img"]),
    ),
    (
        "click_a_remote",
        anchor(
            &["/r/x", "target="],
            LinkExpect::External,
            "the anchor keeps only an http href, which opens in the system browser after a trusted click",
        ),
    ),
    (
        "click_a_target_top",
        anchor(
            &["target="],
            LinkExpect::External,
            "target is stripped, and the href opens only after a trusted click",
        ),
    ),
    (
        "click_a_target_parent",
        anchor(
            &["target="],
            LinkExpect::External,
            "target is stripped, and the href opens only after a trusted click",
        ),
    ),
    (
        "click_a_target_blank",
        anchor(
            &["target="],
            LinkExpect::External,
            "target is stripped, and the href opens only after a trusted click",
        ),
    ),
    (
        "click_a_ping",
        Proof {
            gone: &["ping", "/r/"],
            text_only: &[],
            kept: &["href=\"#x\""],
            link: None,
            inert_because: "",
        },
    ),
    (
        "click_a_download",
        anchor(
            &["download="],
            LinkExpect::External,
            "download is stripped, and the href opens only after a trusted click",
        ),
    ),
    (
        "click_a_svg_document",
        anchor(
            &["target="],
            LinkExpect::Blocked,
            "the href stays relative and link() refuses it because it is not a chapter",
        ),
    ),
    ("click_a_ipc", gone_kept(&["ipc:", "href="], &["<a "])),
    (
        "click_a_app",
        gone_kept(&["jadeslip:", "target", "href="], &["<a "]),
    ),
    ("click_a_file", gone_kept(&["file:", "href="], &["<a "])),
    (
        "click_a_data",
        gone_kept(&["data:", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_plain",
        gone_kept(&["javascript", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_mixed_case",
        gone_kept(&["javascript", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_leading_space",
        gone_kept(&["javascript", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_entity_decimal",
        gone_kept(&["java", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_entity_hex",
        gone_kept(&["javascript", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_entity_tab",
        gone_kept(&["java", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_entity_newline",
        gone_kept(&["java", "pwned", "href="], &["<a "]),
    ),
    (
        "click_js_percent",
        gone_kept(&["java", "pwned", "href="], &["<a "]),
    ),
    (
        "click_vbscript",
        gone_kept(&["vbscript", "href="], &["<a "]),
    ),
    (
        "click_svg_a_href",
        gone_kept(&["javascript", "<a", "href=", "pwned"], &["<rect"]),
    ),
    (
        "click_svg_a_xlink",
        gone_kept(&["javascript", "<a", "href=", "xlink", "pwned"], &["<rect"]),
    ),
    (
        "click_svg_set_href",
        gone_kept(&["<set", "javascript", "<a", "pwned"], &["<rect"]),
    ),
    (
        "click_math_href",
        gone_kept(&["href", "javascript", "pwned"], &["<math", "<mtext"]),
    ),
    (
        "click_area",
        gone_kept(
            &["<map", "<area", "usemap", "javascript", "pwned"],
            &["<img src=\"jadeslip-book://"],
        ),
    ),
    (
        "click_form_get",
        gone(&["<form", "<input", "<button", "action", "/r/"]),
    ),
    (
        "click_form_post",
        gone(&["<form", "<input", "<button", "action", "/r/"]),
    ),
    (
        "click_form_top",
        gone(&["<form", "<button", "action", "target", "/r/"]),
    ),
    (
        "click_form_javascript",
        gone(&["<form", "<button", "action", "javascript", "pwned"]),
    ),
    (
        "click_form_ipc",
        gone(&["<form", "<input", "<button", "action", "ipc:"]),
    ),
    (
        "click_button_formaction",
        gone(&["<form", "<button", "formaction", "/r/"]),
    ),
    ("click_onclick", gone_kept(&["onclick", "pwned"], &["go"])),
    (
        "forge_data_ss_block",
        gone_kept(
            &["data-ss-b=\"9999\"", "data-ss-w=\"0\""],
            &["forged block number"],
        ),
    ),
    (
        "forge_data_ss_orig",
        gone_kept(
            &["data-ss-orig", "/r/", "javascript"],
            &["src=\"jadeslip-book://", "href=\"#x\""],
        ),
    ),
    (
        "forge_scheme_url",
        gone_kept(&["forged-token"], &["<a", "<img"]),
    ),
    (
        "forge_id_clobber",
        Proof {
            gone: &["<form", "<input", "getelementbyid", "parentnode"],
            text_only: &[],
            kept: &["id=\"__tauri_internals__\""],
            link: None,
            inert_because: "ids stay for fragments, and a shadow root has no named access on window, which the end-to-end attack test in unit 4 must confirm",
        },
    ),
    (
        "text_double_decode",
        as_text(&["onerror", "pwned", "&lt;img", "&amp;lt;script"]),
    ),
    (
        "text_cdata",
        as_text(&["onerror", "pwned", "&lt;img", "&lt;script"]),
    ),
    ("text_comment", gone(&["<script", "pwned", "<!--"])),
    (
        "mxss_template",
        gone(&["<template", "<img", "onerror", "pwned"]),
    ),
    (
        "mxss_noscript_title",
        gone(&["noscript", "onerror", "title=", "pwned"]),
    ),
    (
        "mxss_svg_title",
        gone_kept(&["<title", "onerror", "pwned"], &["<svg"]),
    ),
    (
        "mxss_math_annotation",
        gone_kept(&["annotation", "<img", "onerror", "pwned"], &["<math"]),
    ),
    (
        "mxss_svg_foreign_script",
        gone_kept(&["foreignobject", "<script", "pwned"], &["<svg"]),
    ),
    (
        "soup_svg_style_img",
        gone_kept(&["<style", "<img", "onerror", "pwned"], &["<svg"]),
    ),
    (
        "soup_math_mglyph",
        gone_kept(
            &["mglyph", "<table", "<style", "<img", "onerror", "pwned"],
            &["<math", "<mtext"],
        ),
    ),
    (
        "soup_noscript_close",
        gone(&["noscript", "<img", "onerror", "pwned"]),
    ),
    (
        "soup_comment_break",
        gone(&["<!--", "<img", "onerror", "pwned"]),
    ),
    (
        "soup_unclosed_img",
        gone_kept(&["onerror", "pwned", "src="], &["<img"]),
    ),
    (
        "soup_unquoted_onerror",
        gone_kept(&["onerror", "pwned", "src="], &["<img"]),
    ),
    (
        "soup_slash_separators",
        gone_kept(&["onerror", "pwned", "src="], &["<img"]),
    ),
    ("soup_script_upper", gone(&["script", "pwned"])),
    ("soup_script_mixed", gone(&["script", "pwned"])),
    ("soup_script_slash", gone(&["<script", "pwned"])),
    ("soup_script_unclosed", gone(&["<script", "pwned"])),
    ("soup_nul_in_tag", as_text(&["pwned"])),
    (
        "soup_nested_script",
        Proof {
            gone: &["<script", "<scr<"],
            text_only: &["pwned"],
            kept: &[],
            link: None,
            inert_because: "the broken tag is unknown, so it vanishes and its content is shown as text",
        },
    ),
    (
        "soup_attr_breakout",
        gone_kept(&["onmouseover", "pwned"], &["title=\"a\"", "go"]),
    ),
    (
        "soup_javascript_unquoted",
        gone_kept(&["javascript", "pwned", "href="], &["<a "]),
    ),
    (
        "search_text_img",
        as_text(&["searchprobe", "onerror", "pwned", "&lt;img"]),
    ),
    (
        "search_text_script",
        as_text(&["searchprobe", "pwned", "&lt;script", "&lt;svg"]),
    ),
    (
        "search_text_breakout",
        as_text(&["searchprobe", "&lt;/span", "&lt;img", "&lt;iframe"]),
    ),
    (
        "search_text_in_match",
        as_text(&["searchprobe", "onerror", "&lt;img"]),
    ),
    ("search_title_markup", as_text(&["searchprobe"])),
    ("encoding_utf7", as_text(&["pwned", "+adw-script"])),
];

fn lowered_markup_outside_text(node: Node<'_, '_>, out: &mut String) {
    if node.is_element() {
        out.push_str(&node.tag_name().name().to_ascii_lowercase());
        out.push(' ');
        for attr in node.attributes() {
            out.push_str(&attr.name().to_ascii_lowercase());
            out.push('=');
            out.push_str(&attr.value().to_ascii_lowercase());
            out.push(' ');
        }
    }
    for child in node.children() {
        lowered_markup_outside_text(child, out);
    }
}

fn is_known_element(node: Node<'_, '_>) -> bool {
    let local = node.tag_name().name();
    match node.tag_name().namespace() {
        Some(XHTML) => {
            matches!(
                class_of(local),
                ElementClass::Container
                    | ElementClass::Block
                    | ElementClass::Inline
                    | ElementClass::Replaced
            ) && !matches!(local, "html" | "body")
        }
        Some(SVG) => !matches!(
            local,
            "script"
                | "use"
                | "foreignObject"
                | "animate"
                | "animateMotion"
                | "animateTransform"
                | "set"
                | "a"
                | "style"
        ),
        Some(MATH) => !matches!(
            local,
            "annotation" | "annotation-xml" | "maction" | "mglyph"
        ),
        _ => false,
    }
}

fn check_active_markup(reader: &Reader, rendered: &RenderedChapter) -> Result<(), String> {
    let document = strict(&rendered.xhtml)?;
    for node in document.descendants().filter(|node| node.is_element()) {
        if !is_known_element(node) {
            return Err(format!("element {} survived", node.tag_name().name()));
        }
        for attr in node.attributes() {
            let name = attr.name().to_ascii_lowercase();
            let value = attr.value();
            if name.starts_with("on") {
                return Err(format!("handler {name} survived"));
            }
            if name == "data-ss-w" && !value.is_empty() {
                return Err("a book-supplied wrapper mark survived".to_owned());
            }
            if name == "src" || name == "poster" {
                let entry = reader
                    .entry_of_url(value)
                    .ok_or_else(|| format!("{name} is outside the session"))?;
                reader
                    .resource(&entry)
                    .map_err(|_| format!("{name} points at a resource that is not served"))?;
            }
            if name == "href" {
                match node.tag_name().namespace() {
                    Some(XHTML) => {
                        if node.tag_name().name() != "a" {
                            return Err("href on a non-anchor".to_owned());
                        }
                        let lowered = value.trim().to_ascii_lowercase();
                        let scheme_end = lowered.find([':', '/', '?', '#']);
                        let has_scheme =
                            scheme_end.is_some_and(|at| lowered.as_bytes()[at] == b':');
                        let allowed = !has_scheme
                            || lowered.starts_with("http://")
                            || lowered.starts_with("https://")
                            || lowered.starts_with("mailto:");
                        if !allowed {
                            return Err("an anchor kept a forbidden scheme".to_owned());
                        }
                    }
                    _ => {
                        if !value.starts_with('#') && reader.entry_of_url(value).is_none() {
                            return Err("an svg href left the book".to_owned());
                        }
                    }
                }
            }
            if name == "style" {
                check_css_text(value)?;
            }
        }
    }
    for sheet in &rendered.css {
        check_css_text(sheet)?;
        check_sheet_scoped(sheet)?;
    }
    Ok(())
}

fn check_css_text(css: &str) -> Result<(), String> {
    let lowered = css.to_ascii_lowercase();
    for banned in [
        "@import",
        "expression(",
        "javascript:",
        "var(",
        "--",
        "position:fixed",
        "position:sticky",
        "behavior",
        "binding",
        "image-set",
        "\\",
    ] {
        if lowered.contains(banned) && !(banned == "\\" && lowered.contains("\\\"")) {
            return Err(format!("css kept {banned}"));
        }
    }
    let mut rest = lowered.as_str();
    while let Some(at) = rest.find("url(") {
        let tail = &rest[at + 4..];
        if !tail.starts_with("\"jadeslip-book://") && !tail.starts_with("\"#") {
            return Err("css kept a url outside the book".to_owned());
        }
        rest = tail;
    }
    Ok(())
}

fn check_sheet_scoped(sheet: &str) -> Result<(), String> {
    let mut parser = cssparser::Parser::new(sheet);
    check_rules_scoped(&mut parser)
}

fn check_rules_scoped(parser: &mut cssparser::Parser<'_>) -> Result<(), String> {
    use cssparser::{ParseError, Token};
    loop {
        let start = parser.position();
        let mut saw_block = false;
        while let Ok(token) = parser.next() {
            if matches!(token, Token::CurlyBracketBlock) {
                saw_block = true;
                break;
            }
        }
        if !saw_block {
            return Ok(());
        }
        let prelude = parser.slice_from(start);
        let prelude = prelude.strip_suffix('{').unwrap_or(prelude);
        let is_media = prelude.starts_with("@media");
        parser
            .parse_nested_block(|inner| {
                if is_media {
                    check_rules_scoped(inner).map_err(|_| ParseError::<()>::custom(()))?;
                }
                while inner.next().is_ok() {}
                Ok::<_, ParseError<()>>(())
            })
            .map_err(|_| "an @media body was not scoped".to_owned())?;
        if !is_media && !prelude.starts_with("@font-face") {
            for selector in prelude.split(',') {
                if !selector.starts_with(".ss-book ") {
                    return Err(format!("selector {selector} is not scoped"));
                }
            }
        }
    }
}

fn check_attack_row(
    reader: &Reader,
    id: &str,
    ix: ChapterIx,
    proof: &Proof,
    probes: &[Value],
) -> Result<(), String> {
    let rendered = reader.render(ix).map_err(|error| error.user_message())?;
    check_active_markup(reader, &rendered)?;
    let text = reader
        .visible_text(ix)
        .map_err(|error| error.user_message())?;
    check_dom_text(&rendered.xhtml, &text)?;

    let own_text = format!("attack row {id}").to_ascii_lowercase();
    let everything =
        format!("{}\n{}", rendered.xhtml, rendered.css.join("\n")).to_ascii_lowercase();
    if id != "control_plain" && !everything.contains(&own_text) {
        return Err(
            "the row's own control text is missing, so the render did not reach it".to_owned(),
        );
    }
    let everything = everything.replace(&own_text, "");
    for needle in proof.gone {
        if everything.contains(&needle.to_ascii_lowercase()) {
            return Err(format!("{needle} is still in the output"));
        }
    }
    for needle in proof.kept {
        if !everything.contains(&needle.to_ascii_lowercase()) {
            return Err(format!("{needle} should have survived"));
        }
    }

    let document = strict(&rendered.xhtml)?;
    let mut markup = String::new();
    lowered_markup_outside_text(document.root_element(), &mut markup);
    let css = rendered.css.join("\n").to_ascii_lowercase();
    for needle in proof.text_only {
        if !everything.contains(&needle.to_ascii_lowercase()) {
            return Err(format!("{needle} should remain as visible text"));
        }
        let plain = needle
            .trim_start_matches("&lt;")
            .trim_start_matches("&amp;")
            .to_ascii_lowercase();
        if markup.contains(&plain) || css.contains(&plain) {
            return Err(format!("{needle} reached markup outside a text node"));
        }
    }

    let external_ok = matches!(proof.link, Some(LinkExpect::External));
    for probe in probes {
        let probe = probe.as_str().unwrap_or("").to_ascii_lowercase();
        if probe.is_empty() {
            continue;
        }
        let in_css = css.contains(&probe);
        let in_markup = markup.contains(&probe);
        if in_css || (in_markup && !external_ok && id != "control_plain") {
            return Err(format!("probe {probe} is reachable from the output"));
        }
    }

    if let Some(expect) = proof.link {
        let hrefs: Vec<&str> = document
            .descendants()
            .filter(|node| node.tag_name().name() == "a")
            .filter_map(|node| node.attribute("href"))
            .collect();
        if hrefs.is_empty() {
            return Err("the row expected a link to classify".to_owned());
        }
        for href in hrefs {
            let verdict = reader.link(ix, href);
            let good = match expect {
                LinkExpect::External => matches!(verdict, LinkTarget::External { .. }),
                LinkExpect::Blocked => verdict == LinkTarget::Blocked,
            };
            if !good {
                return Err(format!("link verdict {verdict:?}"));
            }
        }
    }
    Ok(())
}

#[test]
fn every_attack_row_is_gone_or_inert_by_construction() {
    let reader = open("attack");
    let sidecar = truth("attack");
    let rows = sidecar["attack"]["rows"].as_array().unwrap();
    let table_ids: Vec<&str> = ATTACK_TABLE.iter().map(|(id, _)| *id).collect();
    let sidecar_ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    assert_eq!(
        table_ids, sidecar_ids,
        "the table covers the sidecar exactly"
    );
    let mut failures = Vec::new();
    for (row, (id, proof)) in rows.iter().zip(ATTACK_TABLE) {
        let ix = ChapterIx(reader_row_of(&reader, id));
        if let Err(why) =
            check_attack_row(&reader, id, ix, proof, row["probes"].as_array().unwrap())
        {
            failures.push(format!("{id}: {why}"));
        }
    }
    let mut inert = 0;
    for (id, proof) in ATTACK_TABLE {
        if !proof.inert_because.is_empty() {
            inert += 1;
            println!("inert by construction, {id}: {}", proof.inert_because);
        }
    }
    println!(
        "attack rows: {} checked, {} with a payload shown only as text, {inert} inert by a stated reason, {} failed",
        rows.len(),
        ATTACK_TABLE
            .iter()
            .filter(|(_, proof)| !proof.text_only.is_empty())
            .count(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn reader_row_of(reader: &Reader, id: &str) -> u32 {
    let suffix = format!("{id}.xhtml");
    reader
        .rows()
        .iter()
        .find(|row| {
            row.href
                .as_ref()
                .is_some_and(|href| href.as_str().ends_with(&suffix))
        })
        .unwrap_or_else(|| panic!("no row for {id}"))
        .ix
        .0
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn largest_real_book() -> PathBuf {
    real_paths()
        .into_iter()
        .max_by_key(|path| {
            Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS)
                .map_or(0, |reader| reader.rows().len())
        })
        .unwrap()
}

#[test]
#[ignore = "toc count on the largest real book: cargo test --release -p jadeslip-core --lib real_toc -- --ignored --nocapture"]
fn real_toc_count_on_the_largest_book() {
    let reader = Reader::open(File::open(largest_real_book()).unwrap(), DEFAULT_LIMITS).unwrap();
    let toc = reader.toc().unwrap();
    let resolved = toc
        .entries
        .iter()
        .filter(|entry| entry.target.is_some())
        .count();
    let max_depth = toc
        .entries
        .iter()
        .map(|entry| entry.depth)
        .max()
        .unwrap_or(0);
    println!(
        "largest real book toc: {} entries, {} resolved, max depth {max_depth}, source {:?}, start chapter {}",
        toc.entries.len(),
        resolved,
        toc.source,
        reader.start_chapter().unwrap().0
    );
    assert_eq!(toc.entries.len(), 6_189);
}

fn time_render<R: crate::ReadAt>(reader: &Reader<R>, ix: ChapterIx) -> (f64, u64) {
    let started = Instant::now();
    let rendered = reader.render(ix).unwrap();
    let elapsed = started.elapsed().as_secs_f64() * 1e3;
    (elapsed, rendered.xhtml.len() as u64)
}

#[test]
#[ignore = "render timing: cargo test --release -p jadeslip-core --lib render_timing -- --ignored --nocapture"]
fn render_timing_on_the_largest_real_book() {
    let reader = Reader::open(File::open(largest_real_book()).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut sizes: Vec<(u32, ChapterIx)> = reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter_map(|row| Some((row.rev?.inflated_len, row.ix)))
        .collect();
    sizes.sort();
    let (median_size, median_ix) = sizes[sizes.len() / 2];
    let nav = reader
        .rows()
        .iter()
        .find(|row| row.is_nav)
        .expect("a nav in the spine");
    let nav_size = nav.rev.unwrap().inflated_len;
    println!(
        "uptime: {}",
        std::fs::read_to_string("/proc/loadavg")
            .unwrap_or_default()
            .trim()
    );
    let mut parts: [Vec<f64>; 6] = Default::default();
    let mut out_bytes = [0u64; 2];
    let mut parts_sum = 0usize;
    let rounds = 201;
    for round in 0..rounds {
        let (median_ms, median_out) = time_render(&reader, median_ix);
        let (nav_ms, nav_out) = time_render(&reader, nav.ix);
        let started = Instant::now();
        let raw = match reader.extract(nav.ix).unwrap() {
            crate::Extracted::Document(raw) => raw,
            crate::Extracted::Image { .. } => panic!("the nav is a document"),
        };
        let inflate = started.elapsed().as_secs_f64() * 1e3;
        let started = Instant::now();
        let (source, _) = crate::chapter::decode_source(raw);
        let scanned = crate::markup::scan(&source, &DEFAULT_LIMITS).unwrap();
        let scan_ms = started.elapsed().as_secs_f64() * 1e3;
        parts_sum += scanned.events.len();
        let started = Instant::now();
        let (layout, ranges) = crate::chapter::BlockLayout::build(&scanned.events, &source);
        let layout_ms = started.elapsed().as_secs_f64() * 1e3;
        let started = Instant::now();
        let serveable = |_: &crate::zip::EntryName| false;
        let no_sheets = |_: &crate::zip::EntryName| false;
        let context = crate::sanitize::Context {
            urls: crate::sanitize::ResourceUrls {
                map: crate::sanitize::UrlMap::new(1, nav.href.clone().unwrap()),
                serveable: &serveable,
            },
            is_stylesheet: &no_sheets,
        };
        let walked = crate::sanitize::sanitize(
            &scanned.events,
            source.len(),
            &crate::sanitize::Layout {
                blocks: &layout.blocks,
                ranges: &ranges,
                body_inner: layout.body_inner,
            },
            &context,
        );
        let walk_ms = started.elapsed().as_secs_f64() * 1e3;
        assert_eq!(walked.xhtml.len() as u64, nav_out);
        out_bytes = [median_out, nav_out];
        if round > 0 {
            parts[0].push(median_ms);
            parts[1].push(nav_ms);
            parts[2].push(inflate);
            parts[3].push(scan_ms);
            parts[4].push(layout_ms);
            parts[5].push(walk_ms);
        }
    }
    let describe = |samples: &mut Vec<f64>| {
        let middle = median(samples);
        format!(
            "median {middle:.3} ms, min {:.3}, p95 {:.3}, max {:.3}",
            samples[0],
            samples[samples.len() * 95 / 100],
            samples[samples.len() - 1]
        )
    };
    println!(
        "median chapter ({median_size} source bytes, {} output bytes): {}",
        out_bytes[0],
        describe(&mut parts[0])
    );
    println!(
        "nav ({nav_size} source bytes, {} output bytes): {}",
        out_bytes[1],
        describe(&mut parts[1])
    );
    println!("nav inflate alone: {}", describe(&mut parts[2]));
    println!(
        "nav decode and scan alone: {}, {} events total",
        describe(&mut parts[3]),
        parts_sum
    );

    println!("nav block layout alone: {}", describe(&mut parts[4]));
    println!("nav sanitising walk alone: {}", describe(&mut parts[5]));

    let generated = open("large-h1-div-br");
    let shaped = open("large-h1-p");
    let mut samples: [Vec<f64>; 2] = Default::default();
    let mut bytes = [0u64; 2];
    let middle = ChapterIx(generated.rows().len() as u32 / 2);
    for round in 0..rounds {
        for (which, book) in [&generated, &shaped].into_iter().enumerate() {
            let (elapsed, out) = time_render(book, middle);
            bytes[which] = out;
            if round > 0 {
                samples[which].push(elapsed);
            }
        }
    }
    println!(
        "large-h1-div-br chapter ({} output bytes): {}",
        bytes[0],
        describe(&mut samples[0])
    );
    println!(
        "large-h1-p chapter ({} output bytes): {}",
        bytes[1],
        describe(&mut samples[1])
    );

    let mut toc_ms = Vec::new();
    let mut heading_ms = Vec::new();
    let mut toc_entries = 0;
    let mut headings = 0;
    for _ in 0..7 {
        let fresh = Reader::open(File::open(largest_real_book()).unwrap(), DEFAULT_LIMITS).unwrap();
        let started = Instant::now();
        toc_entries = fresh.toc().unwrap().entries.len();
        toc_ms.push(started.elapsed().as_secs_f64() * 1e3);
        let started = Instant::now();
        let mut found = 0;
        fresh.heading_titles(&CancelToken::default(), &mut |_, _| found += 1);
        headings = found;
        heading_ms.push(started.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "toc parse, {toc_entries} entries: {}",
        describe(&mut toc_ms)
    );
    println!(
        "heading_titles over the whole book, {headings} headings: {}",
        describe(&mut heading_ms)
    );
}
