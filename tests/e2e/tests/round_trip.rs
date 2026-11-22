use serde_json::Value;
use jadeslip_core::proof::{BREAK_UNIT, CanonToken, block_tokens_of, block_units_of, tokens_of};
use jadeslip_core::{ChapterIx, DEFAULT_LIMITS, Reader};
use jadeslip_e2e::{App, book_label, generated_book, largest_real_book, real_books};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const textmap = await import('/textmap.js');
  const host = document.getElementById('book-host');
  const notice = document.getElementById('notice');
  const serializer = new XMLSerializer();
  window.__roundTrip = async (ix) => {
    await main.goTo(ix);
    if (notice.classList.contains('error') && !notice.hidden) return { failed: true };
    const blocks = [...textmap.blocksOf(host.shadowRoot)];
    return {
      units: blocks.map((block) => {
        let text = '';
        textmap.walkBlock(block, (run) => { text += run; }, () => { text += '￿'; });
        return text;
      }),
      lengths: blocks.map(textmap.blockLength),
      xml: blocks.map((block) => serializer.serializeToString(block)),
    };
  };
  return true;
})()";

const SAMPLE_PER_BOOK: usize = 150;

#[derive(Default)]
struct Tally {
    chapters: u64,
    blocks: u64,
    units: u64,
    tokens: u64,
    failures: BTreeMap<&'static str, u64>,
    first_failures: Vec<String>,
}

impl Tally {
    fn fail(&mut self, class: &'static str, detail: String) {
        *self.failures.entry(class).or_default() += 1;
        if self.first_failures.len() < 12 {
            self.first_failures.push(format!("{class}: {detail}"));
        }
    }

    fn failed(&self) -> u64 {
        self.failures.values().sum()
    }
}

fn js_units(text: &str) -> Vec<u32> {
    text.encode_utf16()
        .map(|unit| {
            if unit == 0xFFFF {
                BREAK_UNIT
            } else {
                u32::from(unit)
            }
        })
        .collect()
}

fn first_difference<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    left.iter()
        .zip(right)
        .position(|(a, b)| a != b)
        .unwrap_or(left.len().min(right.len()))
}

fn shape(token: Option<&CanonToken>) -> String {
    match token {
        None => "end".to_owned(),
        Some(CanonToken::Open { name, attrs }) => {
            let names: Vec<&str> = attrs.iter().map(|(name, _)| name.as_str()).collect();
            format!("open {name} [{}]", names.join(" "))
        }
        Some(CanonToken::Close { name }) => format!("close {name}"),
        Some(CanonToken::Text(text)) => format!("text of {} bytes", text.len()),
    }
}

fn check_chapter(reader: &Reader, ix: u32, mounted: &Value, tally: &mut Tally) {
    let chapter = ChapterIx(ix);
    let rendered = match reader.render(chapter) {
        Ok(rendered) => rendered,
        Err(_) => {
            tally.fail("rust_render_failed", format!("chapter {ix}"));
            return;
        }
    };
    if mounted["failed"].as_bool() == Some(true) {
        tally.fail("webkit_mount_failed", format!("chapter {ix}"));
        return;
    }
    let Some(texts) = mounted["units"].as_array() else {
        tally.fail("no_reply", format!("chapter {ix}"));
        return;
    };
    tally.chapters += 1;
    let walker: Vec<Vec<u32>> = texts
        .iter()
        .map(|text| js_units(text.as_str().unwrap_or_default()))
        .collect();
    let lengths: Vec<u64> = mounted["lengths"].as_array().map_or_else(Vec::new, |list| {
        list.iter().filter_map(Value::as_u64).collect()
    });
    if lengths
        != walker
            .iter()
            .map(|units| units.len() as u64)
            .collect::<Vec<_>>()
    {
        tally.fail("blocklength_disagrees_with_walk", format!("chapter {ix}"));
    }
    let reference = block_units_of(&rendered.xhtml).unwrap_or_default();
    if walker.len() != reference.len() {
        tally.fail(
            "block_count",
            format!(
                "chapter {ix}: {} in WebKit, {} in Rust",
                walker.len(),
                reference.len()
            ),
        );
        return;
    }
    tally.blocks += walker.len() as u64;
    for (block, (from_js, from_rust)) in walker.iter().zip(&reference).enumerate() {
        tally.units += from_js.len() as u64;
        if from_js != from_rust {
            let at = first_difference(from_js, from_rust);
            tally.fail(
                "dom_text_vs_rust_walker",
                format!(
                    "chapter {ix} block {block} unit {at}, lengths {} and {}",
                    from_js.len(),
                    from_rust.len()
                ),
            );
        }
    }
    match reader.visible_text(chapter) {
        Ok(text) => {
            if let Err(error) = text.check_dom_blocks(&walker) {
                tally.fail("dom_text_vs_visible_text", format!("chapter {ix}: {error}"));
            }
        }
        Err(_) => tally.fail("visible_text_failed", format!("chapter {ix}")),
    }
    let source_blocks = block_tokens_of(&rendered.xhtml).unwrap_or_default();
    let serialized = mounted["xml"].as_array().cloned().unwrap_or_default();
    if serialized.len() != source_blocks.len() {
        tally.fail("serialized_block_count", format!("chapter {ix}"));
        return;
    }
    for (block, (xml, source)) in serialized.iter().zip(&source_blocks).enumerate() {
        let Ok(returned) = tokens_of(xml.as_str().unwrap_or_default()) else {
            tally.fail(
                "serialized_block_did_not_scan",
                format!("chapter {ix} block {block}"),
            );
            continue;
        };
        tally.tokens += source.len() as u64;
        if &returned != source {
            let at = first_difference(&returned, source);
            tally.fail(
                "xmlserializer_tokens",
                format!(
                    "chapter {ix} block {block} token {at}: WebKit {} / source {}",
                    shape(returned.get(at)),
                    shape(source.get(at))
                ),
            );
        }
    }
}

fn sweep(book: &Path, indexes: impl Fn(u32) -> Vec<u32>) -> Tally {
    let reader =
        Reader::open(File::open(book).expect("book opens"), DEFAULT_LIMITS).expect("book parses");
    let mut app = App::launch(Some(book), &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    let mut tally = Tally::default();
    for ix in indexes(reader.rows().len() as u32) {
        match app.try_js(&format!("window.__roundTrip({ix})"), 60_000) {
            Ok(mounted) => check_chapter(&reader, ix, &mounted, &mut tally),
            Err(error) => tally.fail("page_script", format!("chapter {ix}: {error}")),
        }
    }
    app.close();
    tally
}

fn report(label: &str, tally: &Tally) {
    println!(
        "{label:<22} chapters {:>5}  blocks {:>7}  units {:>9}  tokens {:>8}  failed {:>4} {:?}",
        tally.chapters,
        tally.blocks,
        tally.units,
        tally.tokens,
        tally.failed(),
        tally.failures
    );
    for line in &tally.first_failures {
        println!("  {line}");
    }
}

fn every(count: u32) -> Vec<u32> {
    (0..count).collect()
}

fn sample(count: u32) -> Vec<u32> {
    let step = (count as usize / SAMPLE_PER_BOOK).max(1);
    (0..count).step_by(step).collect()
}

#[test]
#[ignore = "GUI and real books: tests/e2e/run.sh round_trip"]
fn every_chapter_of_the_largest_real_book_round_trips() {
    let tally = sweep(&largest_real_book(), every);
    report(&book_label(&largest_real_book()), &tally);
    assert_eq!(tally.failed(), 0);
    assert!(tally.chapters > 6000, "the sweep reached every chapter");
}

#[test]
#[ignore = "GUI and real books: tests/e2e/run.sh round_trip"]
fn a_sample_of_every_other_real_book_round_trips() {
    let largest = largest_real_book();
    let mut failed = 0;
    for book in real_books().iter().filter(|book| **book != largest) {
        let tally = sweep(book, sample);
        report(&book_label(book), &tally);
        failed += tally.failed();
        assert!(tally.chapters > 0, "the sweep reached chapters");
    }
    assert_eq!(failed, 0);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh round_trip"]
fn generated_edge_books_round_trip() {
    let mut failed = 0;
    for name in [
        "gbk-xml-decl",
        "gbk-meta-charset",
        "malformed-markup",
        "images-mixed",
        "attack",
    ] {
        let tally = sweep(&generated_book(name), every);
        report(name, &tally);
        failed += tally.failed();
    }
    assert_eq!(failed, 0);
}
