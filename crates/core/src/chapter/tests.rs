use super::*;
use crate::DEFAULT_LIMITS;
use crate::markup::splice;

fn model(source: &str) -> ChapterModel {
    decode_and_model(source.as_bytes().to_vec(), &DEFAULT_LIMITS).expect("models")
}

fn doc(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{body}</body></html>"
    )
}

fn visible(body: &str) -> String {
    model(&doc(body)).text.as_str().to_owned()
}

fn range(start: usize, end: usize) -> VisRange {
    VisRange {
        start: start as u32,
        end: end as u32,
    }
}

#[test]
fn visible_text_applies_each_rule() {
    assert_eq!(visible("<p>a</p>\n<p>b</p>"), "a\nb");
    assert_eq!(visible("<p>  a \n  b  </p>"), "a b");
    assert_eq!(visible("<p>a <b> b</b></p>"), "a b");
    assert_eq!(visible("<p>a<b> </b> b</p>"), "a b");
    assert_eq!(visible("<p>a<br/>\n  b</p>"), "a\nb");
    assert_eq!(visible("<p>a  <br/>b</p>"), "a\nb");
    assert_eq!(visible("<p>a<br/><br/>b</p>"), "a\n\nb");
    assert_eq!(
        visible("<p>a&amp;b &lt;c&gt; &#233; &#x1F732; &nbsp;</p>"),
        "a&b <c> é \u{1F732} \u{a0}"
    );
    assert_eq!(visible("<p>&amp;lt;</p>"), "&lt;");
    assert_eq!(visible("<p>a\u{a0} b</p>"), "a\u{a0} b");
    assert_eq!(visible("<p>a<![CDATA[<b>&amp;]]>c</p>"), "a<b>&amp;c");
    assert_eq!(visible("<pre>a\n  b</pre>"), "a\n  b");
    assert_eq!(visible("<pre>a\r\nb</pre>"), "a\nb");
    assert_eq!(visible("<p>a&#32;b</p>"), "a b");
    assert_eq!(visible("<p>a &#32; b</p>"), "a b");
    assert_eq!(visible("<ul><li>a</li> <li>b</li></ul>"), "a\nb");
    assert_eq!(visible("<div>t<p>a</p>u</div>"), "t\na\nu");
}

#[test]
fn non_rendered_content_contributes_nothing() {
    assert_eq!(
        visible("<p>a<script>x</script><!-- y -->b<style>z</style></p>"),
        "ab"
    );
    assert_eq!(visible("<p title=\"x\">a</p><template>y</template>"), "a");
    assert_eq!(visible("<p>a<iframe>y</iframe></p><form>z</form>"), "a");
    assert_eq!(visible("<p>a<img alt=\"x\" src=\"y\"/>b</p>"), "ab");
    assert_eq!(visible("<svg><text>hidden</text></svg><p>a</p>"), "a");
    assert_eq!(visible("<foo>a</foo><foo><p>b</p></foo>"), "a\nb");
    assert_eq!(model(&doc("<img src=\"x\"/>")).text.as_str(), "");
    assert_eq!(
        model("<html><head><title>t</title></head></html>")
            .text
            .as_str(),
        ""
    );
}

#[test]
fn layout_partitions_on_the_real_book_shapes() {
    let kinds = |body: &str| -> Vec<String> {
        let source = doc(body);
        let chapter = model(&source);
        chapter
            .layout
            .blocks
            .iter()
            .map(|block| match block.kind {
                BlockKind::Element => source[block.span.range()].to_owned(),
                BlockKind::BodyInner => format!("body:{}", &source[block.span.range()]),
            })
            .collect()
    };
    assert_eq!(
        kinds("<h1>t</h1>\n<div>a<br/>b</div>\n"),
        ["<h1>t</h1>", "<div>a<br/>b</div>"]
    );
    assert_eq!(
        kinds("<div><div><p>a</p></div><p>b</p></div>"),
        ["<p>a</p>", "<p>b</p>"]
    );
    assert_eq!(
        kinds("<div>text<p>a</p></div>"),
        ["<div>text<p>a</p></div>"]
    );
    assert_eq!(
        kinds("<div><span>x</span></div>"),
        ["<div><span>x</span></div>"]
    );
    assert_eq!(kinds("text<p>a</p>"), ["body:text<p>a</p>"]);
    assert_eq!(
        kinds("<div><img src=\"x\"/></div><hr/>"),
        ["<img src=\"x\"/>", "<hr/>"]
    );
    assert_eq!(kinds("<div></div>"), Vec::<String>::new());
}

#[test]
fn body_inner_span_excludes_the_body_tags() {
    let source = doc(" <p>a</p> ");
    assert_eq!(
        &source[model(&source).layout.body_inner.range()],
        " <p>a</p> "
    );
}

#[test]
fn encoding_declarations_are_found_for_later_conversion() {
    let source = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html><head><meta charset=\"utf-8\"/><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"/></head><body/></html>";
    let spans = model(source).layout.declarations;
    let values: Vec<&str> = spans.iter().map(|span| &source[span.range()]).collect();
    assert_eq!(values, ["UTF-8", "utf-8", "utf-8"]);
}

#[test]
fn decoding_follows_bom_then_declaration_then_meta_then_utf8() {
    let name = |raw: &[u8]| match decode_source(raw.to_vec()).1 {
        SourceEncoding::Utf8 { bom } => format!("utf8 bom={bom}"),
        SourceEncoding::Other { encoding, .. } => encoding.to_owned(),
    };
    assert_eq!(name(b"\xEF\xBB\xBF<a/>"), "utf8 bom=true");
    assert_eq!(name(b"\xFF\xFE<\x00a\x00/\x00>\x00"), "UTF-16LE");
    assert_eq!(name(b"\xFE\xFF\x00<\x00a\x00/\x00>"), "UTF-16BE");
    assert_eq!(
        name(b"<?xml version='1.0' encoding='GBK'?><a>\xB4\xBA</a>"),
        "GBK"
    );
    assert_eq!(name(b"<html><head><meta charset=gbk><body>\xB4\xBA"), "GBK");
    assert_eq!(
        name(b"<!-- <meta charset=gbk> --><a>x</a>"),
        "utf8 bom=false"
    );
    assert_eq!(
        name(b"<html><body><meta charset=gbk></body>"),
        "utf8 bom=false"
    );
    assert_eq!(
        name(b"<?xml version='1.0' encoding='UTF-7'?><a/>"),
        "utf8 bom=false"
    );
    assert_eq!(
        name(b"<?xml version='1.0' encoding='UTF-16'?><a/>"),
        "utf8 bom=false"
    );
    assert_eq!(name(b"<a>\xFF</a>"), "UTF-8");
}

#[test]
fn a_declared_encoding_that_fails_loses_to_valid_utf8() {
    let raw = "<?xml version='1.0' encoding='ISO-2022-JP'?><a>\u{e9}</a>"
        .as_bytes()
        .to_vec();
    let (source, encoding) = decode_source(raw);
    assert!(matches!(encoding, SourceEncoding::Utf8 { bom: false }));
    assert!(source.contains('\u{e9}'));

    let raw = b"<?xml version='1.0' encoding='ISO-2022-JP'?><a>\xFF</a>".to_vec();
    let (source, encoding) = decode_source(raw);
    assert!(matches!(
        encoding,
        SourceEncoding::Other {
            encoding: "ISO-2022-JP",
            ..
        }
    ));
    assert!(source.contains('\u{FFFD}'));
}

#[test]
fn replacement_rule_from_the_design_holds() {
    let source = doc("foo <b>bar</b> baz");
    let chapter = model(&source);
    let start = chapter.text.as_str().find("o ba").unwrap();
    let edits = chapter.text.source_edits(range(start, start + 4), "X");
    assert_eq!(splice(&source, edits), doc("foX<b>r</b> baz"));
}

#[test]
fn replacements_are_escaped_and_entities_are_atomic() {
    let source = doc("<p>a&amp;b</p>");
    let chapter = model(&source);
    assert_eq!(chapter.text.as_str(), "a&b");
    let edits = chapter.text.source_edits(range(1, 2), "<&>");
    assert_eq!(splice(&source, edits), doc("<p>a&lt;&amp;&gt;b</p>"));
}

#[test]
fn replacement_keeps_tags_breaks_and_drops_nothing_else() {
    let source = doc("<p>one<br/>two</p>");
    let chapter = model(&source);
    let edits = chapter.text.source_edits(range(2, 5), "X");
    assert_eq!(splice(&source, edits), doc("<p>onX<br/>wo</p>"));
    let edits = chapter.text.source_edits(range(1, 1), "Y");
    assert_eq!(splice(&source, edits), doc("<p>oYne<br/>two</p>"));
    let edits = chapter.text.source_edits(range(3, 4), "Z");
    assert_eq!(splice(&source, edits), doc("<p>oneZ<br/>two</p>"));
}

#[test]
fn dom_points_count_utf16_breaks_and_raw_whitespace() {
    let chapter = model(&doc("<p>a\u{1F732}b</p><p>  c  d<br/>e</p>"));
    let text = &chapter.text;
    assert_eq!(text.as_str(), "a\u{1F732}b\nc d\ne");
    let at = |pos: usize, end: usize| text.to_dom(range(pos, end));
    assert_eq!(
        at(5, 6).start,
        DomPoint {
            block: BlockIx(0),
            offset_u16: 3
        }
    );
    assert_eq!(
        at(5, 6).end,
        DomPoint {
            block: BlockIx(0),
            offset_u16: 4
        }
    );
    let c = text.as_str().find('c').unwrap();
    assert_eq!(
        at(c, c + 1).start,
        DomPoint {
            block: BlockIx(1),
            offset_u16: 2
        }
    );
    let d = text.as_str().find('d').unwrap();
    assert_eq!(
        at(d, d + 1).start,
        DomPoint {
            block: BlockIx(1),
            offset_u16: 5
        }
    );
    let e = text.as_str().find('e').unwrap();
    assert_eq!(
        at(e, e + 1).start,
        DomPoint {
            block: BlockIx(1),
            offset_u16: 7
        }
    );
}

#[test]
fn a_crlf_run_counts_as_one_dom_unit_because_xml_parsers_normalise_it() {
    let chapter = model(&doc("<p>a\r\nb</p>"));
    let b = chapter.text.as_str().find('b').unwrap();
    assert_eq!(chapter.text.to_dom(range(b, b + 1)).start.offset_u16, 2);
}

#[test]
fn every_range_round_trips_on_a_mixed_chapter() {
    let chapter = model(&doc(
        "<h1> T&amp;t </h1><p>a  <b> b</b>\u{1F732}<br/>\n c</p><p></p><p> </p><div>d&#233;<br/></div><p>e</p>",
    ));
    let text = &chapter.text;
    let bounds: Vec<usize> = (0..=text.as_str().len())
        .filter(|&at| text.as_str().is_char_boundary(at))
        .collect();
    for &start in &bounds {
        for &end in &bounds {
            if start <= end {
                let want = range(start, end);
                assert_eq!(
                    text.from_dom(text.to_dom(want)),
                    want,
                    "{want:?} via {:?}",
                    text.to_dom(want)
                );
            }
        }
    }
}

#[test]
fn nested_block_boundaries_have_no_dom_position_so_the_round_trip_snaps_past_them_once() {
    let chapter = model(&doc("<ul><li>a</li><li>b</li></ul>"));
    let text = &chapter.text;
    assert_eq!(text.as_str(), "a\nb");
    assert_eq!(text.from_dom(text.to_dom(range(0, 2))), range(0, 1));
    assert_eq!(text.from_dom(text.to_dom(range(1, 3))), range(2, 3));
    assert_eq!(text.from_dom(text.to_dom(range(0, 3))), range(0, 3));
}

#[test]
fn out_of_range_inputs_clamp_without_panicking() {
    let chapter = model(&doc("<p>é</p>"));
    let text = &chapter.text;
    let wild = VisRange { start: 1, end: 900 };
    assert_eq!(text.from_dom(text.to_dom(wild)), range(0, 2));
    let far = DomPoint {
        block: BlockIx(9),
        offset_u16: 9,
    };
    assert_eq!(
        text.from_dom(DomRange {
            start: far,
            end: far
        }),
        range(2, 2)
    );
    let empty = VisibleText::empty();
    assert_eq!(empty.from_dom(empty.to_dom(range(3, 8))), range(0, 0));
    assert!(empty.source_edits(range(0, 0), "").is_empty());
}
