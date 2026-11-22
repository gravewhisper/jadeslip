use crate::chapter::{ChapterModel, SourceEncoding, decode_and_model, model_of_source};
use crate::common::{books, fixture, truth};
use crate::markup::{Event, decode_attr, decode_text, scan, splice};
use crate::{
    BlockIx, ChapterIx, DEFAULT_LIMITS, DomPoint, DomRange, Limits, ReadAt, Reader, VisRange,
};
use proptest::prelude::*;
use jadeslip_fixtures::rng::Rng;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn check_scan_invariants(source: &str, limits: &Limits) {
    let Ok(scanned) = scan(source, limits) else {
        return;
    };
    let mut stack: Vec<&str> = Vec::new();
    let mut last_start = 0;
    let check_span = |start: u32, end: u32| {
        assert!(
            start <= end && end as usize <= source.len(),
            "span in bounds"
        );
        assert!(
            source.is_char_boundary(start as usize) && source.is_char_boundary(end as usize),
            "span on chars"
        );
    };
    for event in &scanned.events {
        match event {
            Event::Open {
                name,
                span,
                self_closed,
                attrs,
            } => {
                check_span(span.start, span.end);
                assert!(span.start >= last_start, "spans ascend");
                last_start = span.start;
                for attr in attrs {
                    check_span(attr.value_span.start, attr.value_span.end);
                }
                if !self_closed {
                    stack.push(name);
                    assert!(
                        stack.len() <= limits.max_xml_depth as usize,
                        "depth is capped"
                    );
                }
            }
            Event::Close { name, span } => {
                check_span(span.start, span.end);
                assert!(span.start >= last_start, "spans ascend");
                last_start = span.start;
                assert_eq!(stack.pop(), Some(*name), "closes balance");
            }
            Event::Text { span, .. } | Event::CData { span, .. } | Event::Other { span } => {
                check_span(span.start, span.end);
                assert!(span.start >= last_start, "spans ascend");
                last_start = span.start;
            }
        }
    }
    assert!(stack.is_empty(), "every open closes");
    for edit in &scanned.fixes.point {
        check_span(edit.span.start, edit.span.end);
    }
}

fn check_model_invariants(model: &ChapterModel) {
    let text = &model.text;
    let (visible, segs, firsts) = text.parts();
    assert_eq!(
        firsts.len(),
        model.layout.blocks.len(),
        "one first-seg per block"
    );
    assert!(
        firsts.windows(2).all(|pair| pair[0] <= pair[1]),
        "blocks ascend"
    );
    assert!(
        firsts.iter().all(|&first| first as usize <= segs.len()),
        "first segs in bounds"
    );
    assert!(
        visible.len() <= model.source.len() + 1,
        "visible text is not longer than source"
    );
    let mut previous_end = 0;
    for block in &model.layout.blocks {
        assert!(block.span.start <= block.span.end, "block span ordered");
        assert!(block.span.start >= previous_end, "blocks do not overlap");
        previous_end = block.span.end;
    }
    let mut vis = 0;
    let mut src = 0;
    for (ix, seg) in segs.iter().enumerate() {
        assert!(seg.vis >= vis, "seg vis ascends");
        assert!(seg.src >= src, "seg src ascends");
        vis = seg.vis;
        src = seg.src;
        let end = segs
            .get(ix + 1)
            .map_or(visible.len() as u32, |next| next.vis);
        assert!(visible.is_char_boundary(seg.vis as usize), "seg on a char");
        let source_slice = &model.source[seg.src as usize..(seg.src + seg.src_len) as usize];
        if seg.kind == crate::chapter::SegKind::Literal {
            assert_eq!(
                &visible[seg.vis as usize..end as usize],
                source_slice,
                "literal equals source"
            );
        }
    }
}

fn check_repairs(source: &str, limits: &Limits) {
    let Ok(scanned) = scan(source, limits) else {
        return;
    };
    if scanned.fixes.structural || scanned.fixes.point.is_empty() {
        return;
    }
    let edits = scanned.fixes.point.clone();
    let repaired = splice(source, edits);
    let Ok(again) = scan(&repaired, limits) else {
        return;
    };
    assert!(
        again.fixes.point.is_empty() && !again.fixes.structural,
        "point fixes finish the repair"
    );
    let utf8 = || SourceEncoding::Utf8 { bom: false };
    let before = model_of_source(source.to_owned(), utf8(), limits).expect("scanned once");
    let after = model_of_source(repaired, utf8(), limits).expect("scanned once");
    let removal_joins_a_crlf = source.as_bytes().windows(2).any(|pair| {
        pair[0] == b'\r'
            && (pair[1] < 0x09
                || pair[1] == 0x0B
                || (0x0E..0x20).contains(&pair[1])
                || pair[1] == 0xEF)
    });
    if !removal_joins_a_crlf {
        assert_eq!(
            before.text.as_str(),
            after.text.as_str(),
            "repair keeps what the reader sees"
        );
    }
}

fn check_mapping(model: &ChapterModel, rng: &mut Rng, samples: usize) {
    let text = &model.text;
    let len = text.as_str().len();
    for _ in 0..samples {
        let a = rng.below(len + 2) as u32;
        let b = rng.below(len + 2) as u32;
        let once = text.from_dom(text.to_dom(VisRange { start: a, end: b }));
        assert!(
            once.start <= once.end && once.end as usize <= len,
            "mapped range in bounds"
        );
        let twice = text.from_dom(text.to_dom(once));
        assert_eq!(once, twice, "mapping settles after one pass");
        let wild = DomRange {
            start: DomPoint {
                block: BlockIx(rng.below(12) as u32),
                offset_u16: rng.below(len + 9) as u32,
            },
            end: DomPoint {
                block: BlockIx(rng.below(12) as u32),
                offset_u16: rng.below(len + 9) as u32,
            },
        };
        let from_wild = text.from_dom(wild);
        assert!(from_wild.end as usize <= len, "wild points clamp");
        let edits = text.source_edits(VisRange { start: a, end: b }, "X&<");
        let edited = splice(&model.source, edits);
        let _ = decode_and_model(edited.into_bytes(), &DEFAULT_LIMITS);
    }
}

fn check_document(bytes: &[u8], rng: &mut Rng) {
    let small = Limits {
        max_xml_depth: 6,
        max_attrs_per_tag: 3,
        max_tag_bytes: 64,
        ..DEFAULT_LIMITS
    };
    let lossy = String::from_utf8_lossy(bytes);
    check_scan_invariants(&lossy, &DEFAULT_LIMITS);
    check_scan_invariants(&lossy, &small);
    check_repairs(&lossy, &DEFAULT_LIMITS);
    if let Ok(model) = decode_and_model(bytes.to_vec(), &DEFAULT_LIMITS) {
        check_model_invariants(&model);
        check_mapping(&model, rng, 8);
    }
}

const DICTIONARY: &[&str] = &[
    "<p>",
    "</p>",
    "<br>",
    "<br/>",
    "</br>",
    "<div>",
    "</div>",
    "<b>",
    "</b>",
    "<i>",
    "</i>",
    "<span a=b c>",
    "</span>",
    "<li>",
    "<ul>",
    "</ul>",
    "<table>",
    "<tr>",
    "<td>",
    "<pre>",
    "</pre>",
    "<script>",
    "</script>",
    "<style>",
    "</style>",
    "<body>",
    "</body>",
    "<html>",
    "</html>",
    "<head>",
    "</head>",
    "<img src=x>",
    "<svg>",
    "</svg>",
    "<x:y/>",
    "<foo>",
    "</foo>",
    "<!--",
    "-->",
    "--",
    "<![CDATA[",
    "]]>",
    "<?xml version='1.0' encoding='GBK'?>",
    "<?pi",
    "?>",
    "<!DOCTYPE html>",
    "<!DOCTYPE a [",
    "<!ENTITY x 'y'>",
    "]>",
    "<meta charset=gbk>",
    "<meta charset=\"utf-16\">",
    "&amp;",
    "&lt;",
    "&nbsp;",
    "&eacute;",
    "&#160;",
    "&#32;",
    "&#x1F732;",
    "&#xD800;",
    "&#0;",
    "&#99999999999;",
    "&",
    "&bogus;",
    "<",
    ">",
    "</",
    "<!",
    "\"",
    "'",
    "=",
    "/",
    " ",
    "  ",
    "\n",
    "\r\n",
    "\r",
    "\t",
    "\u{c}",
    "\u{1}",
    "\u{a0}",
    "\u{e9}",
    "\u{1F732}",
    "\u{FFFE}",
    "\u{FEFF}",
    "春",
    "e\u{301}",
    "\u{0}",
];

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = seed.to_vec();
    for _ in 0..rng.range(1, 8) {
        match rng.below(6) {
            0 if !out.is_empty() => {
                let at = rng.below(out.len());
                out[at] = rng.below(256) as u8;
            }
            1 if !out.is_empty() => {
                let start = rng.below(out.len());
                let end = (start + rng.below(64)).min(out.len());
                out.drain(start..end);
            }
            2 if !out.is_empty() => {
                let start = rng.below(out.len());
                let end = (start + rng.below(256)).min(out.len());
                let copy = out[start..end].to_vec();
                let at = rng.below(out.len());
                out.splice(at..at, copy);
            }
            3 if !out.is_empty() && out.len() > 64 => {
                out.truncate(rng.below(out.len()));
            }
            _ => {
                let token = DICTIONARY[rng.below(DICTIONARY.len())];
                let at = rng.below(out.len() + 1);
                out.splice(at..at, token.bytes());
            }
        }
    }
    out
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn corpus() -> Vec<Vec<u8>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("regression corpus is committed")
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect()
}

fn chapter_bytes(book: &str, limit: usize) -> Vec<Vec<u8>> {
    let reader = Reader::open(File::open(fixture(book)).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut out = Vec::new();
    for row in reader.rows().iter().take(limit) {
        if let Ok(crate::Extracted::Document(bytes)) = reader.extract(row.ix) {
            out.push(bytes);
        }
    }
    out
}

fn seeds() -> Vec<Vec<u8>> {
    let mut seeds = corpus();
    for book in [
        "malformed-markup",
        "gbk-xml-decl",
        "gbk-meta-charset",
        "images-mixed",
        "toc-nested-nav",
    ] {
        seeds.extend(chapter_bytes(book, 4));
    }
    seeds.extend(chapter_bytes("attack", 160));
    seeds.retain(|seed| seed.len() < 64 << 10);
    seeds
}

struct Memory(Vec<u8>);

impl ReadAt for Memory {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| std::io::ErrorKind::UnexpectedEof)?;
        let slice = self
            .0
            .get(start..start.saturating_add(buf.len()))
            .ok_or(std::io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(slice);
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.0.len() as u64)
    }
}

fn check_book_bytes(bytes: Vec<u8>) {
    if let Ok(reader) = Reader::open(Memory(bytes), DEFAULT_LIMITS) {
        for row in reader.rows().iter().take(16) {
            let _ = reader.visible_text(row.ix);
            let _ = reader.locate(row.ix, VisRange { start: 1, end: 3 });
        }
    }
}

fn save_crash(input: &[u8], base: u64, iteration: u64) {
    if let Ok(dir) = std::env::var("SPINE_FUZZ_CRASH_DIR") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            Path::new(&dir).join(format!("crash-{base}-{iteration}.bin")),
            input,
        );
    }
}

fn fuzz_documents(seeds: &[Vec<u8>], base: u64, budget: Budget) -> (u64, Duration) {
    let mut rng = Rng::new(base);
    let mut iterations = 0;
    let mut slowest = Duration::ZERO;
    while budget.more(iterations) {
        let seed = &seeds[rng.below(seeds.len())];
        let input = mutate(seed, &mut rng);
        let started = Instant::now();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check_document(&input, &mut rng);
        }));
        if let Err(panic) = outcome {
            save_crash(&input, base, iterations);
            std::panic::resume_unwind(panic);
        }
        slowest = slowest.max(started.elapsed());
        iterations += 1;
    }
    (iterations, slowest)
}

fn fuzz_books(seeds: &[Vec<u8>], base: u64, budget: Budget) -> u64 {
    let mut rng = Rng::new(base);
    let mut iterations = 0;
    while budget.more(iterations) {
        let seed = &seeds[rng.below(seeds.len())];
        let mut input = seed.clone();
        for _ in 0..rng.range(1, 6) {
            let at = rng.below(input.len());
            let tail_bias = input.len().saturating_sub(rng.below(2048).max(1));
            let at = if rng.below(2) == 0 { at } else { tail_bias };
            input[at] = match rng.below(4) {
                0 => 0xFF,
                1 => 0,
                _ => rng.below(256) as u8,
            };
        }
        check_book_bytes(input);
        iterations += 1;
    }
    iterations
}

#[derive(Clone, Copy)]
enum Budget {
    Iterations(u64),
    Until(Instant),
}

impl Budget {
    fn more(self, done: u64) -> bool {
        match self {
            Budget::Iterations(limit) => done < limit,
            Budget::Until(deadline) => !done.is_multiple_of(64) || Instant::now() < deadline,
        }
    }
}

fn book_seeds() -> Vec<Vec<u8>> {
    [
        "toc-nav-only",
        "malformed-markup",
        "gbk-xml-decl",
        "bomb-lying-sizes",
        "bomb-deep-xml-chapter",
        "images-only",
    ]
    .iter()
    .map(|book| std::fs::read(fixture(book)).unwrap())
    .collect()
}

#[test]
fn the_regression_corpus_keeps_every_invariant() {
    let mut rng = Rng::new(0x5EED);
    let inputs = corpus();
    assert!(inputs.len() >= 10, "corpus is present");
    for input in &inputs {
        check_document(input, &mut rng);
    }
}

#[test]
fn a_short_seeded_fuzz_finds_no_violation() {
    let seeds = seeds();
    let (iterations, _) = fuzz_documents(&seeds, 7, Budget::Iterations(1_500));
    assert_eq!(iterations, 1_500);
    assert_eq!(fuzz_books(&book_seeds(), 11, Budget::Iterations(200)), 200);
}

#[test]
#[ignore = "long fuzz: SPINE_FUZZ_SECS=300 cargo test --release -p jadeslip-core fuzz_for_a_real_duration -- --ignored --nocapture"]
fn fuzz_for_a_real_duration() {
    let seconds: u64 = std::env::var("SPINE_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let base_seed: u64 = std::env::var("SPINE_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let deadline = Budget::Until(Instant::now() + Duration::from_secs(seconds));
    let document_seeds = seeds();
    let book_inputs = book_seeds();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let results: Vec<(u64, u64, Duration)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread| {
                let document_seeds = &document_seeds;
                let book_inputs = &book_inputs;
                scope.spawn(move || {
                    let mut book_runs = 0;
                    let mut document_runs = 0;
                    let mut slowest = Duration::ZERO;
                    if thread % 4 == 3 {
                        book_runs = fuzz_books(book_inputs, base_seed * 1_000 + thread, deadline);
                    } else {
                        let (runs, worst) =
                            fuzz_documents(document_seeds, base_seed * 1_000 + thread, deadline);
                        document_runs = runs;
                        slowest = worst;
                    }
                    (document_runs, book_runs, slowest)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("no fuzz panic"))
            .collect()
    });
    let documents: u64 = results.iter().map(|result| result.0).sum();
    let opened: u64 = results.iter().map(|result| result.1).sum();
    let slowest = results
        .iter()
        .map(|result| result.2)
        .max()
        .unwrap_or_default();
    println!(
        "fuzz ran {seconds} s on {threads} threads: {documents} documents, {opened} mutated books, slowest document {slowest:?}, no violation"
    );
}

fn body_token() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("word".to_owned()),
        Just("é".to_owned()),
        Just("\u{1F732}".to_owned()),
        Just("春江".to_owned()),
        Just(" ".to_owned()),
        Just("   ".to_owned()),
        Just("\n  ".to_owned()),
        Just("\r\n".to_owned()),
        Just("&amp;".to_owned()),
        Just("&#233;".to_owned()),
        Just("&#x1F732;".to_owned()),
        Just("&#160;".to_owned()),
        Just("\u{a0}".to_owned()),
        Just("<br/>".to_owned()),
        Just("<b>".to_owned()),
        Just("</b>".to_owned()),
        Just("<i></i>".to_owned()),
        Just("<!-- c -->".to_owned()),
        Just("<script>s</script>".to_owned()),
        Just("<img src=\"x\"/>".to_owned()),
    ]
}

fn balanced_inline(tokens: Vec<String>) -> String {
    let mut out = String::new();
    let mut open = 0;
    for token in tokens {
        match token.as_str() {
            "<b>" => open += 1,
            "</b>" if open == 0 => continue,
            "</b>" => open -= 1,
            _ => {}
        }
        out.push_str(&token);
    }
    out.push_str(&"</b>".repeat(open));
    out
}

fn body_of_blocks() -> impl Strategy<Value = String> {
    let paragraph = prop::collection::vec(body_token(), 0..12).prop_map(balanced_inline);
    prop::collection::vec((paragraph, 0..3u8), 1..5).prop_map(|parts| {
        parts
            .into_iter()
            .map(|(inner, shape)| match shape {
                0 => format!("<p>{inner}</p>\n"),
                1 => format!("<div>{inner}</div> "),
                _ => format!("<section><h2>{inner}</h2></section>"),
            })
            .collect()
    })
}

fn document(body: &str) -> String {
    format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{body}</body></html>"
    )
}

fn char_bounds(text: &str) -> Vec<usize> {
    (0..=text.len())
        .filter(|&at| text.is_char_boundary(at))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(512),
        max_global_rejects: 1_000_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn to_dom_then_from_dom_returns_the_same_range(body in body_of_blocks(), a in any::<prop::sample::Index>(), b in any::<prop::sample::Index>()) {
        let source = document(&body);
        let model = decode_and_model(source.into_bytes(), &DEFAULT_LIMITS).unwrap();
        let text = &model.text;
        let bounds = char_bounds(text.as_str());
        let (x, y) = (*a.get(&bounds), *b.get(&bounds));
        let want = VisRange { start: x.min(y) as u32, end: x.max(y) as u32 };
        prop_assert_eq!(text.from_dom(text.to_dom(want)), want);
    }

    #[test]
    fn nested_blocks_round_trip_after_at_most_one_snap(inner in prop::collection::vec(body_token(), 0..10), a in any::<prop::sample::Index>(), b in any::<prop::sample::Index>()) {
        let inner = balanced_inline(inner);
        let source = document(&format!("<ul><li>{inner}</li> <li>x{inner}</li></ul><div>y<p>{inner}</p>z</div>"));
        let model = decode_and_model(source.into_bytes(), &DEFAULT_LIMITS).unwrap();
        let text = &model.text;
        let bounds = char_bounds(text.as_str());
        let (x, y) = (*a.get(&bounds), *b.get(&bounds));
        let want = VisRange { start: x.min(y) as u32, end: x.max(y) as u32 };
        let once = text.from_dom(text.to_dom(want));
        prop_assert_eq!(text.from_dom(text.to_dom(once)), once);
        let visible = text.as_str();
        let trimmed = |range: VisRange| visible[range.start as usize..range.end as usize].trim_matches('\n').to_owned();
        prop_assert_eq!(trimmed(once), trimmed(want));
    }

    #[test]
    fn source_edits_change_exactly_the_selected_visible_text(body in body_of_blocks(), a in any::<prop::sample::Index>(), b in any::<prop::sample::Index>()) {
        let source = document(&body);
        let model = decode_and_model(source.clone().into_bytes(), &DEFAULT_LIMITS).unwrap();
        let visible = model.text.as_str();
        let bounds = char_bounds(visible);
        let (x, y) = (*a.get(&bounds), *b.get(&bounds));
        let (start, end) = (x.min(y), x.max(y));
        prop_assume!(!visible.is_empty() && !visible[start..end].contains('\n'));
        let edits = model.text.source_edits(VisRange { start: start as u32, end: end as u32 }, "X");
        let edited = splice(&source, edits);
        let after = decode_and_model(edited.into_bytes(), &DEFAULT_LIMITS).unwrap();
        let want = format!("{}X{}", &visible[..start], &visible[end..]);
        prop_assert_eq!(after.text.as_str(), want);
    }

    #[test]
    fn any_string_scans_and_maps_without_panic(source in any::<String>(), a in any::<u32>(), b in any::<u32>()) {
        let mut rng = Rng::new(u64::from(a));
        check_document(source.as_bytes(), &mut rng);
        if let Ok(model) = decode_and_model(source.into_bytes(), &DEFAULT_LIMITS) {
            let once = model.text.from_dom(model.text.to_dom(VisRange { start: a % 4096, end: b % 4096 }));
            prop_assert!(once.end as usize <= model.text.as_str().len());
        }
    }

    #[test]
    fn dictionary_soup_scans_and_maps_without_panic(tokens in prop::collection::vec(prop::sample::select(DICTIONARY), 0..64), a in any::<u32>()) {
        let source: String = tokens.concat();
        let mut rng = Rng::new(u64::from(a));
        check_document(source.as_bytes(), &mut rng);
    }
}

fn expat_events(source: &str) -> Option<Vec<String>> {
    let scanned = scan(source, &DEFAULT_LIMITS).ok()?;
    let mut events = Vec::new();
    let mut text = String::new();
    let mut depth = 0;
    let flush = |text: &mut String, events: &mut Vec<String>| {
        if !text.is_empty() {
            events.push(format!("T{text}"));
            text.clear();
        }
    };
    for event in &scanned.events {
        match event {
            Event::Open {
                name,
                attrs,
                self_closed,
                ..
            } => {
                flush(&mut text, &mut events);
                let pairs: Vec<String> = attrs
                    .iter()
                    .map(|attr| format!("{}={}", attr.name, decode_attr(attr.raw_value)))
                    .collect();
                events.push(format!("S{name}|{}", pairs.join("|")));
                if *self_closed {
                    events.push(format!("E{name}"));
                } else {
                    depth += 1;
                }
            }
            Event::Close { name, .. } => {
                flush(&mut text, &mut events);
                events.push(format!("E{name}"));
                depth -= 1;
            }
            Event::Text { raw, .. } if depth > 0 => text.push_str(&decode_text(raw)),
            Event::CData { raw, .. } if depth > 0 => {
                text.push_str(&raw.replace("\r\n", "\n").replace('\r', "\n"))
            }
            _ => {}
        }
    }
    flush(&mut text, &mut events);
    Some(events)
}

fn run_expat(documents: &[String]) -> Option<Vec<serde_json::Value>> {
    let dir = jadeslip_fixtures::scratch::scratch(&format!(
        "expat-{:?}",
        std::thread::current().id()
    ));
    let input = dir.join("in.json");
    let output = dir.join("out.json");
    std::fs::write(&input, serde_json::to_vec(documents).ok()?).ok()?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/expat_dump.py");
    let status = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(&input)
        .arg(&output)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let results = serde_json::from_slice(&std::fs::read(&output).ok()?).ok();
    let _ = std::fs::remove_dir_all(&dir);
    results
}

struct FixtureDocument {
    source: String,
    sidecar_well_formed: bool,
    clean: bool,
    label: String,
    repaired: Option<String>,
}

fn fixture_documents() -> Vec<FixtureDocument> {
    let mut out = Vec::new();
    for book in books() {
        if book.starts_with("bomb-") {
            continue;
        }
        let truth = truth(book);
        let reader = Reader::open(File::open(fixture(book)).unwrap(), DEFAULT_LIMITS).unwrap();
        let step = if book.starts_with("large-") { 40 } else { 1 };
        for row in reader.rows().iter().step_by(step) {
            let row_truth = &truth["spine"][row.ix.0 as usize];
            let Ok(Some(model)) = reader.model(row.ix) else {
                continue;
            };
            let well_formed = row_truth["well_formed_after_decoding"].as_bool().unwrap();
            if model.source.contains("<!ENTITY") {
                continue;
            }
            let clean = model.fixes.point.is_empty() && !model.fixes.structural;
            let repaired =
                (!model.fixes.structural).then(|| splice(&model.source, model.fixes.point));
            out.push(FixtureDocument {
                source: model.source,
                sidecar_well_formed: well_formed,
                clean,
                label: format!("{book} chapter {}", row.ix.0),
                repaired,
            });
        }
    }
    out
}

#[test]
fn fixes_are_empty_exactly_when_the_sidecar_says_well_formed() {
    let mut checked = 0;
    for document in fixture_documents() {
        assert_eq!(
            document.clean, document.sidecar_well_formed,
            "{}",
            document.label
        );
        checked += 1;
    }
    assert!(checked > 300, "{checked}");
}

#[test]
fn expat_agrees_with_the_scanner_on_strict_fixture_chapters() {
    if Command::new("python3").arg("--version").output().is_err() {
        println!("python3 is not installed, so the expat cross-check did not run");
        return;
    }
    let documents = fixture_documents();
    let sources: Vec<String> = documents
        .iter()
        .map(|document| document.source.clone())
        .collect();
    let results = run_expat(&sources).expect("expat script ran");
    let mut strict = 0;
    for (document, result) in documents.iter().zip(&results) {
        let label = &document.label;
        if result["well_formed"].as_bool() != Some(true) {
            assert!(
                !document.clean,
                "{label}: scanner called it clean, expat said {}",
                result["why"]
            );
            continue;
        }
        strict += 1;
        let ours = expat_events(&document.source).unwrap();
        let theirs: Vec<String> = result["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ours.len(), theirs.len(), "{label}: event count");
        let same = ours.iter().zip(&theirs).position(|(a, b)| a != b);
        assert_eq!(same, None, "{label}: first differing event");
    }
    println!("expat agreed on every event of {strict} strict chapters");
    assert!(strict > 300, "{strict}");
}

#[test]
fn expat_agrees_with_the_repairs_on_chapters_that_only_need_point_fixes() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let mut repaired = Vec::new();
    for document in fixture_documents() {
        if !document.sidecar_well_formed
            && let Some(fixed) = document.repaired
        {
            repaired.push(fixed);
        }
    }
    assert!(repaired.len() >= 3, "{}", repaired.len());
    let results = run_expat(&repaired).expect("expat script ran");
    for result in &results {
        assert_eq!(
            result["well_formed"].as_bool(),
            Some(true),
            "{}",
            result["why"]
        );
    }
}

#[test]
#[ignore = "differential: SPINE_DIFF_RUNS=200000 cargo test --release -p jadeslip-core expat_differential -- --ignored --nocapture"]
fn expat_differential_over_mutated_documents() {
    let runs: u64 = std::env::var("SPINE_DIFF_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);
    let seeds = seeds();
    let mut rng = Rng::new(99);
    let mut documents = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        let seed = &seeds[rng.below(seeds.len())];
        let bytes = mutate(seed, &mut rng);
        documents.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    let results = run_expat(&documents).expect("expat script ran");
    let mut strict = 0u64;
    let mut clean_rejected = 0u64;
    let mut dirty_accepted = 0u64;
    let mut events_differ = 0u64;
    let mut unexplained = Vec::new();
    for (document, result) in documents.iter().zip(&results) {
        let Ok(scanned) = scan(document, &DEFAULT_LIMITS) else {
            continue;
        };
        let has_dtd = document.contains("<!DOCTYPE")
            && (document.contains('[')
                || document.contains("SYSTEM")
                || document.contains("PUBLIC"));
        let clean = scanned.fixes.point.is_empty() && !scanned.fixes.structural;
        let why = result["why"].as_str().unwrap_or("");
        if result["well_formed"].as_bool() == Some(true) {
            strict += 1;
            let theirs: Vec<String> = result["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            if expat_events(document).unwrap() != theirs {
                events_differ += 1;
                if !has_dtd {
                    unexplained.push("events differ on a document with no DTD".to_owned());
                }
            }
            if !clean {
                dirty_accepted += 1;
                if !has_dtd {
                    unexplained.push(format!(
                        "fix requested, expat accepts: {:?}",
                        scanned
                            .fixes
                            .point
                            .first()
                            .map(|edit| &document[edit.span.range()])
                    ));
                }
            }
        } else if clean {
            clean_rejected += 1;
            if !has_non_bmp_or_compat_name(document) {
                let line = why
                    .rsplit("line ")
                    .next()
                    .and_then(|tail| tail.split(',').next())
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(1);
                let excerpt: String = document
                    .split('\n')
                    .nth(line - 1)
                    .unwrap_or("")
                    .chars()
                    .take(90)
                    .collect();
                unexplained.push(format!("scanner clean, expat says {why} on {excerpt:?}"));
            }
        }
    }
    println!(
        "{runs} mutated documents: expat accepts {strict}, events differ {events_differ}, scanner clean but expat rejects {clean_rejected}, scanner asks for a fix but expat accepts {dirty_accepted}, unexplained {}",
        unexplained.len()
    );
    unexplained.truncate(20);
    assert!(unexplained.is_empty(), "{unexplained:#?}");
}

fn has_non_bmp_or_compat_name(document: &str) -> bool {
    document.chars().any(|ch| ch as u32 >= 0x10000 || matches!(ch, '\u{2C00}'..='\u{2FEF}' | '\u{F900}'..='\u{FEFE}' | '\u{300}'..='\u{36F}'))
}

#[test]
fn an_internal_dtd_is_never_expanded_and_asks_for_a_rewrite() {
    let reader = Reader::open(File::open(fixture("attack")).unwrap(), DEFAULT_LIMITS).unwrap();
    let model = reader.model(ChapterIx(79)).unwrap().unwrap();
    assert!(model.source.contains("<!ENTITY lol4"));
    assert!(model.text.as_str().contains("&lol4;"));
    assert!(!model.text.as_str().contains("lollol"));
    assert!(model.fixes.structural);
}

#[test]
fn a_failed_chapter_leaves_its_neighbours_readable() {
    let reader = Reader::open(
        File::open(fixture("bomb-lying-sizes")).unwrap(),
        DEFAULT_LIMITS,
    )
    .unwrap();
    assert!(reader.visible_text(ChapterIx(1)).is_err());
    assert!(reader.visible_text(ChapterIx(0)).is_ok());
    assert!(reader.visible_text(ChapterIx(99)).is_err());
}

fn real_paths() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "epub"));
    paths.sort();
    paths
}

#[test]
#[ignore = "real books against expat, counts only: cargo test --release -p jadeslip-core --lib real_books_agree -- --ignored --nocapture"]
fn real_books_agree_with_expat_on_chapters_the_scanner_calls_clean() {
    for path in real_paths() {
        let id = path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('-')
            .next()
            .unwrap()
            .to_owned();
        let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
        let mut sources = Vec::new();
        let (mut point_only, mut structural) = (0, 0);
        for row in reader.rows() {
            let Ok(Some(model)) = reader.model(row.ix) else {
                continue;
            };
            structural += usize::from(model.fixes.structural);
            point_only += usize::from(!model.fixes.structural && !model.fixes.point.is_empty());
            sources.push(model.source);
        }
        let results = run_expat(&sources).expect("expat script ran");
        let (mut agree, mut disagree_events, mut clean_rejected, mut dirty_accepted) = (0, 0, 0, 0);
        for (source, result) in sources.iter().zip(&results) {
            let scanned = scan(source, &DEFAULT_LIMITS).unwrap();
            let clean = scanned.fixes.point.is_empty() && !scanned.fixes.structural;
            if result["well_formed"].as_bool() == Some(true) {
                let theirs: Vec<String> = result["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect();
                if expat_events(source).unwrap() == theirs {
                    agree += 1;
                } else {
                    disagree_events += 1;
                }
                dirty_accepted += usize::from(!clean);
            } else {
                clean_rejected += usize::from(clean);
            }
        }
        println!(
            "{id}: {} documents, point fixes {point_only}, structural {structural}, expat events equal {agree}, differ {disagree_events}, scanner clean but expat rejects {clean_rejected}, scanner asks for a fix but expat accepts {dirty_accepted}",
            sources.len()
        );
    }
}

#[test]
#[ignore = "open phases: cargo test --release -p jadeslip-core --lib open_phases -- --ignored --nocapture"]
fn open_phases_on_the_largest_books() {
    use crate::package::{OpfDoc, find_opf};
    use crate::zip::ZipIndex;
    let mut paths = vec![fixture("large-h1-div-br")];
    paths.extend(real_paths().into_iter().max_by_key(|path| {
        std::fs::metadata(path)
            .map_or(0, |meta| meta.len())
            .min(u64::from(!path.to_string_lossy().contains("Refactoring")) * u64::MAX)
    }));
    for path in paths {
        let mut phases = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut opf_bytes = 0;
        for round in 0..41 {
            let file = File::open(&path).unwrap();
            let t0 = Instant::now();
            let zip = ZipIndex::parse(&file, &DEFAULT_LIMITS).unwrap();
            let t1 = Instant::now();
            let name = find_opf(&zip, &file, &DEFAULT_LIMITS).unwrap();
            let t2 = Instant::now();
            let bytes = zip
                .read_entry(&file, zip.entry(&name).unwrap(), &DEFAULT_LIMITS)
                .unwrap();
            opf_bytes = bytes.len();
            let t3 = Instant::now();
            let opf = OpfDoc::parse(bytes, &DEFAULT_LIMITS).unwrap();
            let t4 = Instant::now();
            std::hint::black_box(&opf.spine);
            let started = Instant::now();
            let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
            let t5 = started.elapsed();
            std::hint::black_box(reader.rows().len());
            if round > 0 {
                for (slot, value) in phases
                    .iter_mut()
                    .zip([t1 - t0, t2 - t1, t3 - t2, t4 - t3, t5])
                {
                    slot.push(value.as_secs_f64() * 1e3);
                }
            }
        }
        for slot in &mut phases {
            slot.sort_by(f64::total_cmp);
        }
        let median = |slot: &Vec<f64>| slot[slot.len() / 2];
        println!(
            "OPF {} KB: zip directory {:.2} ms, container {:.2} ms, read OPF {:.2} ms, parse OPF {:.2} ms, whole Reader::open {:.2} ms (medians of 40)",
            opf_bytes / 1000,
            median(&phases[0]),
            median(&phases[1]),
            median(&phases[2]),
            median(&phases[3]),
            median(&phases[4])
        );
    }
}

#[test]
#[ignore = "chapter phases: cargo test --release -p jadeslip-core --lib chapter_phases -- --ignored --nocapture"]
fn chapter_phases_on_the_largest_books() {
    use crate::chapter::{BlockLayout, VisibleText, decode_source};
    let mut paths = vec![fixture("large-h1-div-br")];
    paths.extend(
        real_paths()
            .into_iter()
            .filter(|path| path.to_string_lossy().contains("serial_a")),
    );
    for path in paths {
        let reader = Reader::open(File::open(&path).unwrap(), DEFAULT_LIMITS).unwrap();
        let mut runs = Vec::new();
        for _ in 0..5 {
            let mut times = [Duration::ZERO; 5];
            let (mut bytes_total, mut events_total, mut segs_total, mut events_most) =
                (0usize, 0usize, 0usize, 0usize);
            for row in reader.rows() {
                let t0 = Instant::now();
                let Ok(crate::Extracted::Document(raw)) = reader.extract(row.ix) else {
                    continue;
                };
                let t1 = Instant::now();
                bytes_total += raw.len();
                let (source, _) = decode_source(raw);
                let t2 = Instant::now();
                let scanned = scan(&source, &DEFAULT_LIMITS).unwrap();
                let t3 = Instant::now();
                let (layout, ranges) = BlockLayout::build(&scanned.events, &source);
                let t4 = Instant::now();
                let text = VisibleText::fold(&scanned.events, &source, &layout.blocks, &ranges);
                let t5 = Instant::now();
                events_total += scanned.events.len();
                events_most = events_most.max(scanned.events.len());
                segs_total += text.parts().1.len();
                for (slot, spent) in
                    times
                        .iter_mut()
                        .zip([t1 - t0, t2 - t1, t3 - t2, t4 - t3, t5 - t4])
                {
                    *slot += spent;
                }
            }
            runs.push((times, bytes_total, events_total, segs_total, events_most));
        }
        let ms = |phase: usize| {
            let mut values: Vec<f64> = runs
                .iter()
                .map(|run| run.0[phase].as_secs_f64() * 1e3)
                .collect();
            values.sort_by(f64::total_cmp);
            format!("{:.1} ({:.1}..{:.1})", values[2], values[0], values[4])
        };
        println!(
            "{} rows, {:.1} MB, {} events, most in one document {}, {} segs. medians of 5 ms (range): inflate {}, decode {}, scan {}, layout {}, fold {}",
            reader.rows().len(),
            runs[0].1 as f64 / 1e6,
            runs[0].2,
            runs[0].4,
            runs[0].3,
            ms(0),
            ms(1),
            ms(2),
            ms(3),
            ms(4)
        );
    }
}
