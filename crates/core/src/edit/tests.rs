use super::*;
use crate::canon::proof::{block_units_of, block_xml_of};
use crate::chapter::{decode_and_model, decode_source};
use crate::common::fixture;
use crate::sanitize::{ResourceUrls, Sheet, sanitize};
use crate::zip::EntryName;
use crate::{Book, BookKey, ChapterIx, DEFAULT_LIMITS, Extracted, Reader};
use proptest::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const SESSION: u64 = 0x0123_4567_89AB_CDEF;
const HOST: &str = "jadeslip-book://0123456789abcdef";
const BASE: ChapterRev = ChapterRev {
    crc32: 0,
    inflated_len: 0,
};

fn head(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><title>t</title></head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

fn with_context<T>(work: impl FnOnce(&Context<'_>) -> T) -> T {
    let serveable = |entry: &EntryName| entry.as_str().ends_with(".png");
    let is_stylesheet = |_: &EntryName| false;
    work(&Context {
        urls: ResourceUrls {
            map: UrlMap::new(SESSION, EntryName::parse("OEBPS/text/c.xhtml").unwrap()),
            serveable: &serveable,
        },
        is_stylesheet: &is_stylesheet,
    })
}

fn display_xml(raw: &[u8]) -> Vec<String> {
    let model = decode_and_model(raw.to_vec(), &DEFAULT_LIMITS).unwrap();
    let scanned = scan(&model.source, &DEFAULT_LIMITS).unwrap();
    let (layout, ranges) = BlockLayout::build(&scanned.events, &model.source);
    let sanitized = with_context(|context| {
        sanitize(
            &scanned.events,
            model.source.len(),
            &Layout {
                blocks: &layout.blocks,
                ranges: &ranges,
                body_inner: layout.body_inner,
            },
            context,
        )
    });
    assert!(
        sanitized
            .sheets
            .iter()
            .all(|sheet| !matches!(sheet, Sheet::Linked(_)))
    );
    block_xml_of(&sanitized.xhtml).unwrap()
}

fn plan(raw: &[u8], changes: Vec<BlockChange>) -> Result<ChapterPlan, Error> {
    let model = decode_and_model(raw.to_vec(), &DEFAULT_LIMITS)?;
    let edits = BlockEdits {
        base: BASE,
        changes,
    };
    with_context(|context| plan_chapter(&model, &edits, context, &DEFAULT_LIMITS))
}

fn written(raw: &[u8], changes: Vec<BlockChange>) -> Vec<u8> {
    match plan(raw, changes).unwrap() {
        ChapterPlan::Write(planned) => planned.bytes,
        ChapterPlan::Unchanged { .. } => panic!("the plan wrote nothing"),
    }
}

fn replace(of: u32, xml: impl Into<String>) -> BlockChange {
    BlockChange::Replace {
        of: BlockIx(of),
        xml: xml.into(),
    }
}

fn is_unchanged(raw: &[u8], changes: Vec<BlockChange>) -> bool {
    matches!(plan(raw, changes).unwrap(), ChapterPlan::Unchanged { .. })
}

fn tokens_block(raw: &[u8], block: usize) -> String {
    display_xml(raw)[block].clone()
}

#[test]
fn every_block_serialised_back_is_unchanged_with_serializer_noise() {
    let raw = head(
        "<h1 id=\"a\" class=\"t\">One &amp; two</h1>\n<p class=\"x\" id=\"b\">a&#160;b<br/>c &#8212; d</p>\n<p>tail<img src=\"../i/a.png\" alt=\"x\"/></p>",
    );
    let serialised = vec![
        replace(
            0,
            "<h1 xmlns=\"http://www.w3.org/1999/xhtml\" class=\"t\" id=\"a\" data-ss-b=\"0\">One &amp; two</h1>",
        ),
        replace(
            1,
            "<p xmlns=\"http://www.w3.org/1999/xhtml\" id=\"b\" class=\"x\" data-ss-b=\"1\">a\u{A0}b<br></br>c \u{2014} d</p>",
        ),
        replace(
            2,
            format!(
                "<p xmlns=\"http://www.w3.org/1999/xhtml\" data-ss-b=\"2\">tail<img src=\"{HOST}/OEBPS/i/a.png\" alt=\"x\"></img></p>"
            ),
        ),
    ];
    assert!(is_unchanged(raw.as_bytes(), serialised));
}

#[test]
fn a_typo_changes_one_byte_and_every_entity_and_line_ending_elsewhere_keeps_its_bytes() {
    let raw = head("<p>first&#160;word &#8212; caf&#233;\r\nsecond &amp; third</p>");
    let edited = tokens_block(raw.as_bytes(), 0).replace("third", "thirt");
    let new = written(raw.as_bytes(), vec![replace(0, edited)]);
    assert_eq!(new, raw.replace("third<", "thirt<").into_bytes());
}

#[test]
fn two_far_apart_typos_keep_every_break_and_entity_between_them() {
    let raw = head("<div>alpha<br />one&#160;two<br />three &#233;<br />omega</div>");
    let edited = tokens_block(raw.as_bytes(), 0)
        .replace("alpha", "alpho")
        .replace("omega", "omego");
    let new = written(raw.as_bytes(), vec![replace(0, edited)]);
    let expected = raw.replace("alpha", "alpho").replace("omega", "omego");
    assert_eq!(new, expected.into_bytes());
}

#[test]
fn a_new_line_break_inserts_only_the_break() {
    let raw = head("<div>one two<br />three</div>");
    let edited = tokens_block(raw.as_bytes(), 0).replace("one two", "one<br></br>two");
    let new = written(raw.as_bytes(), vec![replace(0, edited)]);
    assert_eq!(
        new,
        head("<div>one<br/>two<br />three</div>").into_bytes(),
        "the space became a break and the old break kept its bytes"
    );
}

#[test]
fn markup_the_display_dropped_keeps_its_bytes_around_an_edit() {
    let raw = head(
        "<p onclick=\"x()\" data-keep=\"1\">ab<!--note-->cd<script>s()</script>ef<foo>gh</foo>ij</p>",
    );
    let shown = tokens_block(raw.as_bytes(), 0);
    let new = written(raw.as_bytes(), vec![replace(0, shown.replace("ij", "iJ"))]);
    assert_eq!(new, raw.replace("ij</p>", "iJ</p>").into_bytes());
    let new = written(
        raw.as_bytes(),
        vec![replace(0, shown.replace("abcdef", "xyzabc"))],
    );
    let text = String::from_utf8(new).unwrap();
    assert!(text.contains("<!--note-->") && text.contains("<script>s()</script>"));
    assert!(text.contains("onclick=\"x()\" data-keep=\"1\""));
    assert_eq!(
        decode_and_model(text.into_bytes(), &DEFAULT_LIMITS)
            .unwrap()
            .text
            .as_str(),
        "xyzabcghij"
    );
}

#[test]
fn deleting_across_a_dropped_tag_keeps_the_markup_balanced() {
    let raw = head("<p>a<foo>b</foo>c</p>");
    let shown = tokens_block(raw.as_bytes(), 0);
    let new = written(raw.as_bytes(), vec![replace(0, shown.replace("ab", ""))]);
    assert_eq!(new, head("<p><foo></foo>c</p>").into_bytes());
}

#[test]
fn a_webkit_nbsp_where_the_source_has_a_space_is_no_change() {
    let raw = head("<p>a b</p>");
    let shown = tokens_block(raw.as_bytes(), 0);
    assert!(is_unchanged(
        raw.as_bytes(),
        vec![replace(0, shown.replace("a b", "a\u{A0}b"))]
    ));
}

#[test]
fn a_typed_nbsp_is_written_as_a_space_and_a_source_nbsp_outside_the_change_stays() {
    let raw = head("<p>a&#160;b\u{A0}c word</p>");
    let shown = tokens_block(raw.as_bytes(), 0);
    let new = written(
        raw.as_bytes(),
        vec![replace(0, shown.replace("word", "wo\u{A0}rd"))],
    );
    assert_eq!(new, raw.replace(" word", " wo rd").into_bytes());
}

#[test]
fn urls_in_new_markup_go_back_to_relative_hrefs() {
    let raw = head("<p>text</p>");
    let edited = format!(
        "<p data-ss-b=\"0\">te<img src=\"{HOST}/OEBPS/i/a%20b.png#f\" alt=\"x\"></img><span style=\"background-image: url(&quot;{HOST}/OEBPS/i/c.png&quot;)\">xt</span></p>"
    );
    let new = String::from_utf8(written(raw.as_bytes(), vec![replace(0, edited)])).unwrap();
    assert!(!new.contains("jadeslip-book:"));
    assert!(new.contains("src=\"../i/a%20b.png#f\""));
    assert!(new.contains("../i/c.png"));
}

#[test]
fn a_scheme_url_from_another_session_is_dropped_not_saved() {
    let raw = head("<p>text</p>");
    let edited = "<p data-ss-b=\"0\">text<img src=\"jadeslip-book://ffffffffffffffff/OEBPS/i/a.png\" alt=\"x\"></img></p>";
    let new = String::from_utf8(written(raw.as_bytes(), vec![replace(0, edited)])).unwrap();
    assert!(!new.contains("jadeslip-book:"));
    assert!(new.contains("<img alt=\"x\"/>"));
}

#[test]
fn a_cdata_edit_with_a_special_character_widens_and_keeps_the_text_right() {
    let raw = head("<p><![CDATA[a b]]></p>");
    let shown = tokens_block(raw.as_bytes(), 0);
    let new = written(
        raw.as_bytes(),
        vec![replace(0, shown.replace("a b", "a &amp; b"))],
    );
    assert_eq!(
        decode_and_model(new, &DEFAULT_LIMITS)
            .unwrap()
            .text
            .as_str(),
        "a & b"
    );
}

#[test]
fn a_trailing_break_in_an_emptied_block_is_dropped_but_a_source_lone_break_is_kept() {
    let raw = head("<p class=\"a\">x</p>");
    let emptied =
        "<p xmlns=\"http://www.w3.org/1999/xhtml\" class=\"a\" data-ss-b=\"0\"><br></br></p>";
    assert_eq!(
        written(raw.as_bytes(), vec![replace(0, emptied)]),
        head("<p class=\"a\"></p>").into_bytes()
    );
    let empty = head("<p class=\"a\"></p>");
    assert!(is_unchanged(empty.as_bytes(), vec![replace(0, emptied)]));
    let spacer = head("<p class=\"a\"><br/></p>");
    assert!(is_unchanged(spacer.as_bytes(), vec![replace(0, emptied)]));
}

#[test]
fn a_duplicate_id_from_a_split_stays_on_the_first_element() {
    let raw = head("<p id=\"k\">one two</p>");
    let new = written(
        raw.as_bytes(),
        vec![
            replace(0, "<p id=\"k\" data-ss-b=\"0\">one</p>"),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: "<p id=\"k\">two</p>".to_owned(),
            },
        ],
    );
    assert_eq!(new, head("<p id=\"k\">one</p>\n<p>two</p>").into_bytes());
}

#[test]
fn a_split_writes_only_a_close_and_an_open_and_keeps_the_tail_bytes() {
    let tail =
        "two <!-- kept --><font color=\"red\">three</font> &amp; four&#160;five\r\n<big>six</big>";
    let raw = head(&format!("  <div class=\"c\">one {tail}</div>"));
    let source = tokens_block(raw.as_bytes(), 0);
    let cut = source.find("two").unwrap();
    let first = format!("{}</div>", &source[..cut]);
    let second = format!("<div class=\"c\">{}", &source[cut..]);
    let new = written(
        raw.as_bytes(),
        vec![
            replace(0, first),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: second,
            },
        ],
    );
    assert_eq!(
        String::from_utf8(new).unwrap(),
        head(&format!(
            "  <div class=\"c\">one </div>\n  <div class=\"c\">{tail}</div>"
        ))
    );
}

#[test]
fn a_split_across_a_dropped_tag_widens_and_stays_well_formed() {
    let raw = head("<div>one <foo>two three</foo> four</div>");
    let source = tokens_block(raw.as_bytes(), 0);
    let cut = source.find("three").unwrap();
    let new = written(
        raw.as_bytes(),
        vec![
            replace(0, format!("{}</div>", &source[..cut])),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: format!("<div>{}", &source[cut..]),
            },
        ],
    );
    let text = String::from_utf8(new).unwrap();
    assert!(text.contains("<div>one two </div>"));
    assert!(text.contains("<div>three four</div>"));
    assert!(!text.contains("foo"));
}

#[test]
fn a_split_of_body_text_keeps_the_tail_bytes() {
    let raw = head("one <!-- kept -->two three");
    let new = written(
        raw.as_bytes(),
        vec![
            replace(0, "<div>one </div>"),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: "<div>two three</div>".to_owned(),
            },
        ],
    );
    assert!(
        String::from_utf8(new)
            .unwrap()
            .contains("one <div><!-- kept -->two three</div>")
    );
}

#[test]
fn a_split_with_a_typo_in_the_tail_changes_only_those_places() {
    let raw = head("<p>one <!-- a -->two <!-- b -->three</p>");
    let new = written(
        raw.as_bytes(),
        vec![
            replace(0, "<p>one </p>"),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: "<p>two thrxe</p>".to_owned(),
            },
        ],
    );
    assert_eq!(
        String::from_utf8(new).unwrap(),
        head("<p>one </p>\n<p><!-- a -->two <!-- b -->thrxe</p>")
    );
}

#[test]
fn a_duplicate_id_already_in_the_source_survives_an_edit() {
    let raw = head("<p id=\"k\">one</p>\n<p id=\"k\">two</p>");
    let shown = tokens_block(raw.as_bytes(), 1).replace("two", "tww");
    assert_eq!(
        written(raw.as_bytes(), vec![replace(1, shown)]),
        raw.replace("two", "tww").into_bytes()
    );
}

#[test]
fn delete_removes_the_block_and_its_indent_and_insert_then_delete_is_identity() {
    let raw = head("  <h1>t</h1>\n  <p>one</p>\n  <p>two</p>");
    let deleted = written(raw.as_bytes(), vec![BlockChange::Delete { of: BlockIx(1) }]);
    assert_eq!(deleted, head("  <h1>t</h1>\n  <p>two</p>").into_bytes());
    for after in [None, Some(BlockIx(0)), Some(BlockIx(1)), Some(BlockIx(2))] {
        let inserted = written(
            raw.as_bytes(),
            vec![BlockChange::InsertAfter {
                after,
                xml: "<p>new</p>".to_owned(),
            }],
        );
        let at = after.map_or(0, |of| of.0 + 1);
        let removed = written(&inserted, vec![BlockChange::Delete { of: BlockIx(at) }]);
        assert_eq!(removed, raw.clone().into_bytes(), "after {after:?}");
    }
}

#[test]
fn inserts_keep_their_order_and_combine_with_delete_and_replace_of_the_anchor() {
    let raw = head("<p>one</p>\n<p>two</p>");
    let shown = tokens_block(raw.as_bytes(), 0).replace("one", "uno");
    let new = written(
        raw.as_bytes(),
        vec![
            BlockChange::InsertAfter {
                after: None,
                xml: "<p>zero</p>".to_owned(),
            },
            replace(0, shown),
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: "<p>a</p>".to_owned(),
            },
            BlockChange::InsertAfter {
                after: Some(BlockIx(0)),
                xml: "<p>b</p>".to_owned(),
            },
            BlockChange::Delete { of: BlockIx(1) },
            BlockChange::InsertAfter {
                after: Some(BlockIx(1)),
                xml: "<p>c</p>".to_owned(),
            },
        ],
    );
    assert_eq!(
        new,
        head("<p>zero</p>\n<p>uno</p>\n<p>a</p>\n<p>b</p>\n<p>c</p>").into_bytes()
    );
    let deleted_first = written(
        raw.as_bytes(),
        vec![
            BlockChange::Delete { of: BlockIx(0) },
            BlockChange::InsertAfter {
                after: None,
                xml: "<p>zero</p>".to_owned(),
            },
        ],
    );
    assert_eq!(
        decode_and_model(deleted_first, &DEFAULT_LIMITS)
            .unwrap()
            .text
            .as_str(),
        "zero\ntwo"
    );
}

#[test]
fn an_insert_into_an_empty_body_lands_in_the_body() {
    let raw = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body></body></html>";
    let new = written(
        raw.as_bytes(),
        vec![BlockChange::InsertAfter {
            after: None,
            xml: "<p>new</p>".to_owned(),
        }],
    );
    assert_eq!(new, raw.replace("<body>", "<body><p>new</p>").into_bytes());
}

#[test]
fn a_body_with_direct_text_edits_through_its_wrapper() {
    let raw = head("loose <b>text</b> here");
    let shown = tokens_block_root(raw.as_bytes());
    assert!(is_unchanged(
        raw.as_bytes(),
        vec![replace(0, shown.clone())]
    ));
    let new = written(
        raw.as_bytes(),
        vec![replace(0, shown.replace("here", "hare"))],
    );
    assert_eq!(new, raw.replace("here", "hare").into_bytes());
}

fn tokens_block_root(raw: &[u8]) -> String {
    let shown = display_xml(raw);
    assert_eq!(shown.len(), 1);
    shown[0].clone()
}

#[test]
fn utf8_bom_and_declaration_are_kept() {
    let mut raw = b"\xEF\xBB\xBF".to_vec();
    raw.extend_from_slice(head("<p>word</p>").as_bytes());
    let shown = tokens_block(&raw, 0).replace("word", "ward");
    let new = written(&raw, vec![replace(0, shown)]);
    let mut expected = b"\xEF\xBB\xBF".to_vec();
    expected.extend_from_slice(head("<p>ward</p>").as_bytes());
    assert_eq!(new, expected);
}

fn gbk_chapter() -> Vec<u8> {
    let text = "<?xml version=\"1.0\" encoding=\"GBK\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>\u{4E00}</title></head><body><p>\u{4E00}\u{4E8C}\u{4E09} word</p><p>\u{56DB}</p></body></html>";
    encoding_rs::GBK.encode(text).0.into_owned()
}

#[test]
fn gbk_stays_gbk_and_bytes_outside_the_edit_are_equal() {
    let raw = gbk_chapter();
    let shown = tokens_block(&raw, 0).replace("\u{4E8C}", "\u{4E94}");
    let new = written(&raw, vec![replace(0, shown)]);
    let (old_len, new_len) = (raw.len(), new.len());
    assert_eq!(old_len, new_len);
    let differing = raw.iter().zip(&new).filter(|(a, b)| a != b).count();
    assert!((1..=2).contains(&differing), "{differing} bytes differ");
    let (decoded, had_errors) = encoding_rs::GBK.decode_without_bom_handling(&new);
    assert!(!had_errors);
    assert!(decoded.contains("\u{4E00}\u{4E94}\u{4E09} word"));
    assert!(decoded.contains("encoding=\"GBK\""));
}

#[test]
fn a_character_gbk_cannot_hold_becomes_a_numeric_reference_and_the_chapter_stays_gbk() {
    let raw = gbk_chapter();
    let shown = tokens_block(&raw, 0).replace("word", "w\u{1F600}rd");
    let new = written(&raw, vec![replace(0, shown)]);
    let (decoded, had_errors) = encoding_rs::GBK.decode_without_bom_handling(&new);
    assert!(!had_errors);
    assert!(decoded.contains("w&#128512;rd"));
    assert!(decoded.contains("encoding=\"GBK\""));
    assert!(
        decode_and_model(new, &DEFAULT_LIMITS)
            .unwrap()
            .text
            .as_str()
            .contains("w\u{1F600}rd")
    );
}

#[test]
fn a_chapter_that_does_not_round_trip_converts_to_utf8_with_a_warning() {
    let mut raw = gbk_chapter();
    raw.extend_from_slice(b"<!--\xFF-->");
    let shown = tokens_block(&raw, 0).replace("word", "ward");
    let ChapterPlan::Write(planned) = plan(&raw, vec![replace(0, shown)]).unwrap() else {
        panic!("the plan wrote nothing");
    };
    let text = String::from_utf8(planned.bytes).unwrap();
    assert!(text.contains("encoding=\"UTF-8\""));
    assert!(text.contains("\u{4E00}\u{4E8C}\u{4E09} ward"));
    assert_eq!(planned.warnings.len(), 1);
}

#[test]
fn utf16_converts_to_utf8_with_a_warning() {
    let text = head("<p>word</p>").replace("utf-8", "UTF-16");
    let mut raw = vec![0xFF, 0xFE];
    raw.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let shown = tokens_block(&raw, 0).replace("word", "ward");
    let ChapterPlan::Write(planned) = plan(&raw, vec![replace(0, shown)]).unwrap() else {
        panic!("the plan wrote nothing");
    };
    let new = String::from_utf8(planned.bytes).unwrap();
    assert!(new.contains("encoding=\"UTF-8\"") && new.contains("<p>ward</p>"));
    assert_eq!(planned.warnings.len(), 1);
}

#[test]
fn a_malformed_chapter_is_left_alone_by_a_no_op_and_repaired_by_a_real_edit() {
    let raw = head("<p>one &nbsp; & two<p>three<br>four</p></div>");
    let shown = display_xml(raw.as_bytes());
    let all: Vec<BlockChange> = shown
        .iter()
        .enumerate()
        .map(|(ix, xml)| replace(ix as u32, xml.clone()))
        .collect();
    assert!(is_unchanged(raw.as_bytes(), all));
    let edited = shown[1].replace("four", "fore");
    let ChapterPlan::Write(planned) = plan(raw.as_bytes(), vec![replace(1, edited)]).unwrap()
    else {
        panic!("the plan wrote nothing");
    };
    assert!(!planned.warnings.is_empty());
    let text = String::from_utf8(planned.bytes).unwrap();
    roxmltree::Document::parse(&text).expect("the repaired chapter is XML");
    assert!(text.contains("&#160; &amp; two"));
    assert!(text.contains("three<br/>fore"));
}

#[test]
fn unknown_doubled_and_empty_changes_are_handled() {
    let raw = head("<p>one</p>");
    assert!(matches!(
        plan(raw.as_bytes(), vec![BlockChange::Delete { of: BlockIx(9) }]),
        Err(Error::Book(_))
    ));
    let doubled = vec![
        BlockChange::Delete { of: BlockIx(0) },
        BlockChange::Delete { of: BlockIx(0) },
    ];
    assert!(matches!(plan(raw.as_bytes(), doubled), Err(Error::Book(_))));
    assert!(is_unchanged(raw.as_bytes(), Vec::new()));
}

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/edit-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("data")).unwrap();
        Scratch(dir)
    }

    fn book(&self, source: &str) -> Book {
        let path = self.0.join("book.epub");
        fs::copy(fixture(source), &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Book::open(&path, &self.0.join("data"), BookKey(5), DEFAULT_LIMITS).unwrap()
    }

    fn file(&self) -> Vec<u8> {
        fs::read(self.0.join("book.epub")).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn first_text_chapter(book: &Book) -> (ChapterIx, ChapterRev, Vec<String>) {
    let reader = book.read();
    for row in reader.rows() {
        if let Ok(rendered) = reader.render(row.ix)
            && let Ok(xml) = block_xml_of(&rendered.xhtml)
            && !xml.is_empty()
        {
            return (row.ix, rendered.rev, xml);
        }
    }
    panic!("the fixture has text");
}

fn raw_of(reader: &Reader, ix: ChapterIx) -> Vec<u8> {
    match reader.extract(ix).unwrap() {
        Extracted::Document(bytes) => bytes,
        Extracted::Image { .. } => panic!("not a document"),
    }
}

fn typo_in(xml: &str) -> String {
    let at = xml
        .char_indices()
        .find(|(at, ch)| {
            ch.is_ascii_lowercase()
                && xml[..*at].rfind('>').is_some_and(|close| {
                    xml[close..*at]
                        .bytes()
                        .all(|byte| byte != b'<' && byte != b'&')
                })
        })
        .map(|(at, _)| at)
        .expect("a letter in text");
    let swapped = if xml.as_bytes()[at] == b'q' { "z" } else { "q" };
    format!("{}{swapped}{}", &xml[..at], &xml[at + 1..])
}

#[test]
fn a_save_through_commit_changes_only_the_typo_and_a_no_op_touches_nothing() {
    let scratch = Scratch::new();
    let book = scratch.book("toc-none");
    let (ix, rev, xml) = first_text_chapter(&book);
    let before = scratch.file();
    let all = xml
        .iter()
        .enumerate()
        .map(|(block, xml)| replace(block as u32, xml.clone()))
        .collect();
    let saved = book
        .save_chapter(
            ix,
            &BlockEdits {
                base: rev,
                changes: all,
            },
        )
        .unwrap();
    assert!(matches!(saved.outcome, SaveOutcome::Unchanged));
    assert_eq!(saved.rev, rev);
    assert_eq!(saved.after.unwrap().len(), xml.len());
    assert_eq!(scratch.file(), before);
    assert!(!book.data_has_any_slot(), "a no-op makes no backup");

    let old = raw_of(&book.read(), ix);
    let saved = book
        .save_chapter(
            ix,
            &BlockEdits {
                base: rev,
                changes: vec![replace(0, typo_in(&xml[0]))],
            },
        )
        .unwrap();
    assert!(matches!(saved.outcome, SaveOutcome::Written(_)));
    assert_ne!(saved.rev, rev);
    let new = raw_of(&book.read(), ix);
    assert_eq!(old.len(), new.len());
    assert_eq!(old.iter().zip(&new).filter(|(a, b)| a != b).count(), 1);
    let after = saved.after.expect("the layout kept its blocks");
    assert_eq!(after[0].origin, BlockOrigin::Change(0));
    assert!(
        after[0]
            .display
            .as_ref()
            .is_some_and(|shown| shown.contains("data-ss-b=\"0\""))
    );
    assert!(after[1..].iter().all(|block| block.display.is_none()));
    let stale = book.save_chapter(
        ix,
        &BlockEdits {
            base: rev,
            changes: Vec::new(),
        },
    );
    assert!(matches!(stale, Err(Error::PlanStale)));
}

#[test]
fn a_later_launch_finds_the_backup_from_before_the_first_edit() {
    let scratch = Scratch::new();
    drop(scratch.book("large-h1-p"));
    let path = scratch.0.join("book.epub");
    let data = scratch.0.join("data");
    let original = scratch.file();
    for letter in ["x", "y"] {
        let book = Book::open_by_fingerprint(&path, &data, DEFAULT_LIMITS).unwrap();
        let (ix, rev, shown) = first_text_chapter(&book);
        let edits = BlockEdits {
            base: rev,
            changes: vec![replace(
                0,
                shown[0].replacen("</", &format!("{letter}</"), 1),
            )],
        };
        book.save_chapter(ix, &edits).unwrap();
        assert!(scratch.file() != original);
    }
    let book = Book::open_by_fingerprint(&path, &data, DEFAULT_LIMITS).unwrap();
    book.revert(crate::RevertTarget::WholeBook).unwrap();
    assert!(
        scratch.file() == original,
        "the backup is the pre-edit file"
    );
}

#[test]
fn the_session_survives_a_save_so_a_mounted_chapter_can_save_again() {
    let scratch = Scratch::new();
    let book = scratch.book("images-mixed");
    let reader = book.read();
    let session = reader.session();
    let (ix, rev, xml) = reader
        .rows()
        .iter()
        .find_map(|row| {
            let rendered = reader.render(row.ix).ok()?;
            let xml = block_xml_of(&rendered.xhtml).ok()?;
            xml.iter()
                .any(|block| block.contains("jadeslip-book:"))
                .then_some((row.ix, rendered.rev, xml))
        })
        .expect("a chapter with an image");
    let image_block = xml
        .iter()
        .position(|block| block.contains("jadeslip-book:"))
        .unwrap();
    let last = xml.len() as u32 - 1;
    let first = book
        .save_chapter(
            ix,
            &BlockEdits {
                base: rev,
                changes: vec![BlockChange::InsertAfter {
                    after: Some(BlockIx(last)),
                    xml: "<p>added</p>".to_owned(),
                }],
            },
        )
        .unwrap();
    assert_eq!(book.read().session(), session);
    let second = book
        .save_chapter(
            ix,
            &BlockEdits {
                base: first.rev,
                changes: vec![replace(image_block as u32, xml[image_block].clone())],
            },
        )
        .unwrap();
    assert!(matches!(second.outcome, SaveOutcome::Unchanged));
}

#[test]
fn an_image_chapter_is_not_editable() {
    let scratch = Scratch::new();
    let book = scratch.book("images-only");
    let reader = book.read();
    let row = reader
        .rows()
        .iter()
        .find(|row| matches!(reader.extract(row.ix), Ok(Extracted::Image { .. })))
        .expect("an image in the spine");
    let result = book.save_chapter(
        row.ix,
        &BlockEdits {
            base: row.rev.unwrap(),
            changes: Vec::new(),
        },
    );
    assert!(matches!(result, Err(Error::NotEditable(_))));
}

#[test]
fn one_forced_verify_failure_widens_and_still_saves_the_right_text() {
    let raw = head("<p>first second</p>");
    let shown = tokens_block(raw.as_bytes(), 0).replace("second", "secund");
    seam::CORRUPT_ATTEMPTS.with(|left| left.set(1));
    seam::ATTEMPTS.with(|count| count.set(0));
    let new = written(raw.as_bytes(), vec![replace(0, shown)]);
    assert_eq!(seam::ATTEMPTS.with(|count| count.get()), 2);
    assert_eq!(new, head("<p>first secund</p>").into_bytes());
}

#[test]
fn a_second_forced_verify_failure_writes_nothing_and_leaves_the_file_hash() {
    let scratch = Scratch::new();
    let book = scratch.book("toc-none");
    let (ix, rev, xml) = first_text_chapter(&book);
    let before = scratch.file();
    seam::CORRUPT_ATTEMPTS.with(|left| left.set(2));
    let result = book.save_chapter(
        ix,
        &BlockEdits {
            base: rev,
            changes: vec![replace(0, typo_in(&xml[0]))],
        },
    );
    seam::CORRUPT_ATTEMPTS.with(|left| left.set(0));
    assert!(matches!(result, Err(Error::WouldBreak(_))));
    assert_eq!(scratch.file(), before);
    assert!(!book.data_has_any_slot(), "a refused save makes no backup");
}

#[derive(Clone, Debug)]
enum Op {
    Insert(usize, char),
    Delete(usize),
    Swap(usize, char),
    Break(usize),
}

fn op() -> impl Strategy<Value = Op> {
    let ch = prop_oneof![
        Just('x'),
        Just(' '),
        Just('\u{E9}'),
        Just('\u{4E2D}'),
        Just('\u{1F600}'),
        Just('&'),
        Just('<'),
    ];
    prop_oneof![
        (any::<usize>(), ch.clone()).prop_map(|(at, ch)| Op::Insert(at, ch)),
        any::<usize>().prop_map(Op::Delete),
        (any::<usize>(), ch).prop_map(|(at, ch)| Op::Swap(at, ch)),
        any::<usize>().prop_map(Op::Break),
    ]
}

fn body_shape() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<h1 id=\"h\">Title &#160;here</h1>\n<p>alpha beta &amp; gamma</p>\n<p class=\"c\">delta&#8212;epsilon</p>".to_owned()),
        Just("<h1>Title</h1>\n<div>line one<br />line two<br />line three &#233;<br /></div>".to_owned()),
        Just("<section>\n  <p>nested <em>inline <b>deep</b></em> tail</p>\n  <blockquote>quoted<!--c--> text</blockquote>\n</section>".to_owned()),
        Just("<p>with <foo bar=\"1\">unknown</foo> and <script>x()</script>dropped</p><pre>a\r\n  b</pre>".to_owned()),
        Just("<ul><li>one<li>two</ul><p>unclosed & loose".to_owned()),
    ]
}

fn escaped(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_owned(),
        '<' => "&lt;".to_owned(),
        other => other.to_string(),
    }
}

fn text_positions(xml: &str) -> Vec<(usize, usize)> {
    let scanned = scan(xml, &DEFAULT_LIMITS).unwrap();
    let mut positions = Vec::new();
    for event in &scanned.events {
        if let Event::Text { raw, span } = event {
            let mut at = 0;
            while at < raw.len() {
                let len = if raw[at..].starts_with('&') {
                    raw[at..].find(';').map_or(1, |semi| semi + 1)
                } else {
                    raw[at..].chars().next().unwrap().len_utf8()
                };
                positions.push((span.start as usize + at, len));
                at += len;
            }
        }
    }
    positions
}

fn apply_op(xml: &str, op: &Op) -> Option<String> {
    let positions = text_positions(xml);
    let pick = |at: usize| positions.get(at % positions.len().max(1)).copied();
    let (at, len) = pick(match op {
        Op::Insert(at, _) | Op::Delete(at) | Op::Swap(at, _) | Op::Break(at) => *at,
    })?;
    let (keep_to, keep_from, middle) = match op {
        Op::Insert(_, ch) => (at, at, escaped(*ch)),
        Op::Delete(_) => {
            if positions.len() < 2 {
                return None;
            }
            (at, at + len, String::new())
        }
        Op::Swap(_, ch) => (at, at + len, escaped(*ch)),
        Op::Break(_) => (at, at, "<br></br>".to_owned()),
    };
    Some(format!("{}{middle}{}", &xml[..keep_to], &xml[keep_from..]))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_edits_save_valid_xml_with_the_intended_text(
        body in body_shape(),
        ops in proptest::collection::vec((any::<usize>(), op()), 1..4),
    ) {
        let raw = head(&body).into_bytes();
        let original_parses = roxmltree::Document::parse(std::str::from_utf8(&raw).unwrap()).is_ok();
        let mut shown = display_xml(&raw);
        let mut changed = HashSet::new();
        for (block, op) in &ops {
            let block = block % shown.len();
            if let Some(next) = apply_op(&shown[block], op) {
                shown[block] = next;
                changed.insert(block);
            }
        }
        let changes: Vec<BlockChange> = changed
            .iter()
            .map(|block| replace(*block as u32, shown[*block].clone()))
            .collect();
        let units_of = |blocks: &[String]| -> Vec<u32> {
            blocks
                .iter()
                .flat_map(|xml| block_units_of(xml).unwrap())
                .flatten()
                .map(|unit| if unit == 0xA0 { 0x20 } else { unit })
                .collect()
        };
        let intended = units_of(&shown);
        match plan(&raw, changes).unwrap() {
            ChapterPlan::Unchanged { .. } => {
                prop_assert_eq!(units_of(&display_xml(&raw)), intended);
            }
            ChapterPlan::Write(planned) => {
                let (text, _) = decode_source(planned.bytes.clone());
                if original_parses {
                    prop_assert!(roxmltree::Document::parse(&text).is_ok());
                }
                prop_assert!(!text.contains("jadeslip-book:"));
                let saved = display_xml(&planned.bytes);
                prop_assert_eq!(units_of(&saved), intended);
                let model = decode_and_model(planned.bytes, &DEFAULT_LIMITS).unwrap();
                let saved_units: Vec<Vec<u32>> = saved
                    .iter()
                    .flat_map(|xml| block_units_of(xml).unwrap())
                    .collect();
                prop_assert!(model.text.check_dom_blocks(&saved_units).is_ok());
            }
        }
    }
}
