use super::*;
use crate::DEFAULT_LIMITS;

fn trace(source: &str) -> (String, Fixes) {
    let scanned = scan(source, &DEFAULT_LIMITS).expect("scan succeeds");
    let mut out = String::new();
    for event in &scanned.events {
        match event {
            Event::Open {
                name,
                attrs,
                self_closed,
                ..
            } => {
                out.push_str(&format!("<{name}"));
                for attr in attrs {
                    out.push_str(&format!(" {}={}", attr.name, attr.raw_value));
                }
                out.push_str(if *self_closed { "/>" } else { ">" });
            }
            Event::Close { name, .. } => out.push_str(&format!("</{name}>")),
            Event::Text { raw, .. } | Event::CData { raw, .. } => out.push_str(raw),
            Event::Other { .. } => out.push('#'),
        }
    }
    (out, scanned.fixes)
}

fn fixed(source: &str) -> String {
    let (_, fixes) = trace(source);
    splice(source, fixes.point)
}

#[test]
fn well_formed_documents_need_no_fixes() {
    let source = "<?xml version=\"1.0\"?>\n<!DOCTYPE html>\n<html><body a=\"1\" b='2'><p>x &amp; &#160; y<br/>z</p><![CDATA[<raw>]]><!-- c --></body></html>\n";
    let (out, fixes) = trace(source);
    assert!(fixes.point.is_empty() && !fixes.structural);
    assert_eq!(
        out,
        "#\n#\n<html><body a=1 b=2><p>x &amp; &#160; y<br/>z</p><raw>#</body></html>\n"
    );
}

#[test]
fn nesting_that_xml_allows_stays_nested_rather_than_being_repaired_as_html() {
    let source = "<html><p>a<div>b<p>c</p></div></p><br>d</br></html>";
    let (out, fixes) = trace(source);
    assert!(fixes.point.is_empty() && !fixes.structural);
    assert_eq!(out, source);
}

#[test]
fn point_fixes_cover_each_local_defect() {
    assert_eq!(fixed("<a>x<br>y</a>"), "<a>x<br/>y</a>");
    assert_eq!(fixed("<a>fish & chips</a>"), "<a>fish &amp; chips</a>");
    assert_eq!(fixed("<a>x&nbsp;y&eacute;</a>"), "<a>x&#160;y&#233;</a>");
    assert_eq!(fixed("<a>x&bogus;</a>"), "<a>x&amp;bogus;</a>");
    assert_eq!(fixed("<a>1 < 2</a>"), "<a>1 &lt; 2</a>");
    assert_eq!(fixed("<a>]]></a>"), "<a>]]&gt;</a>");
    assert_eq!(fixed("<a>x\u{1}y</a>"), "<a>xy</a>");
    assert_eq!(fixed("<a b=c d>x</a>"), "<a b=\"c\" d=\"d\">x</a>");
    assert_eq!(fixed("<a b=\"1\"c=\"2\">x</a>"), "<a b=\"1\" c=\"2\">x</a>");
    assert_eq!(fixed("<a b=\"x&y<z\"/>"), "<a b=\"x&amp;y&lt;z\"/>");
    assert_eq!(fixed("<P>x</p>"), "<P>x</P>");
}

#[test]
fn removing_an_invisible_character_cannot_form_a_forbidden_cdata_end_in_text() {
    assert_eq!(fixed("<a>]\u{1}]></a>"), "<a>]]&gt;</a>");
}

#[test]
fn stray_and_implied_closes_are_balanced_and_marked_structural() {
    for (source, want) in [
        ("<p>a</b>c</p>", "<p>a#c</p>"),
        ("<ul><li>a<li>b</ul>", "<ul><li>a</li><li>b</li></ul>"),
        (
            "<body><p>a\n<p>b\n</body>",
            "<body><p>a\n</p><p>b\n</p></body>",
        ),
        ("<p><b>a<i>b</b></i></p>", "<p><b>a<i>b</i></b>#</p>"),
        (
            "<div><p>a<ul><li>b</ul>c</div>",
            "<div><p>a</p><ul><li>b</li></ul>c</div>",
        ),
        (
            "<table><tr><td>a<td>b<tr><td>c</table>",
            "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>",
        ),
        (
            "<ul><li>a<ul><li>b</ul><li>c</ul>",
            "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>",
        ),
        ("<a><b>", "<a><b></b></a>"),
    ] {
        let (out, fixes) = trace(source);
        assert_eq!(out, want, "{source}");
        assert!(fixes.structural, "{source}");
    }
}

#[test]
fn script_is_raw_only_in_a_document_that_needs_repair() {
    let (out, fixes) = trace("<a><script>if (a<b && c) { x(\"</p>\") }</script></a>");
    assert_eq!(out, "<a><script>if (a<b && c) { x(\"</p>\") }</script></a>");
    assert!(fixes.structural);
}

#[test]
fn unterminated_markup_ends_the_scan_without_rescanning() {
    for source in [
        "<a>x<!-- never",
        "<a>x<![CDATA[ never",
        "<a>x<?pi never",
        "<a>x<!DOCTYPE never",
        "<a>x<b c=\"never",
        "<a>x<b",
        "<a>x</a",
    ] {
        let (_, fixes) = trace(source);
        assert!(fixes.structural, "{source}");
    }
}

#[test]
fn doctype_internal_subset_is_not_expanded() {
    let source = "<!DOCTYPE a [<!ENTITY x \"<b>boom</b>\">]><a>&x;</a>";
    let (out, fixes) = trace(source);
    assert_eq!(out, "#<a>&x;</a>");
    assert!(fixes.structural);
    assert_eq!(fixes.point.len(), 1);
}

#[test]
fn limits_fail_with_a_limit_error() {
    let deep = "<div>".repeat(129) + &"</div>".repeat(129);
    assert!(matches!(scan(&deep, &DEFAULT_LIMITS), Err(Error::Limit(_))));
    let at_limit = "<div>".repeat(128) + &"</div>".repeat(128);
    assert!(scan(&at_limit, &DEFAULT_LIMITS).is_ok());

    let attrs: String = (0..257).map(|ix| format!(" a{ix}=\"1\"")).collect();
    assert!(matches!(
        scan(&format!("<a{attrs}/>"), &DEFAULT_LIMITS),
        Err(Error::Limit(_))
    ));

    let long = format!("<a b=\"{}\"/>", "x".repeat(1 << 20));
    assert!(matches!(scan(&long, &DEFAULT_LIMITS), Err(Error::Limit(_))));
}

#[test]
fn markup_event_count_is_capped_so_a_tiny_compressed_chapter_cannot_exhaust_memory() {
    let limits = Limits {
        max_markup_events: 100,
        ..DEFAULT_LIMITS
    };
    assert!(scan(&format!("<a>{}</a>", "<b/>".repeat(40)), &limits).is_ok());
    assert!(matches!(
        scan(&format!("<a>{}</a>", "<b/>".repeat(200)), &limits),
        Err(Error::Limit(_))
    ));
}

#[test]
fn void_tags_nested_in_the_strict_pass_do_not_trip_the_depth_limit() {
    let source = format!("<a>{}</a>", "<br>".repeat(500));
    let (_, fixes) = trace(&source);
    assert_eq!(fixes.point.len(), 500);
    assert!(!fixes.structural);
}

#[test]
fn class_table_is_case_insensitive_and_unknown_by_default() {
    assert_eq!(class_of("DIV"), ElementClass::Container);
    assert_eq!(class_of("p"), ElementClass::Block);
    assert_eq!(class_of("svg"), ElementClass::Replaced);
    assert_eq!(class_of("span"), ElementClass::Inline);
    assert_eq!(class_of("script"), ElementClass::NonRendered);
    assert_eq!(class_of("iframe"), ElementClass::Forbidden);
    assert_eq!(class_of("frobnicate-long-name"), ElementClass::Unknown);
}

#[test]
fn entity_table_is_sorted_for_binary_search() {
    assert!(NAMED_ENTITIES.windows(2).all(|pair| pair[0].0 < pair[1].0));
}

#[test]
fn splice_applies_sorted_edits_and_drops_contained_ones() {
    let edit = |start, end, text: &str| SourceEdit {
        span: Span::new(start, end),
        text: text.to_owned(),
    };
    assert_eq!(
        splice("abcdef", vec![edit(4, 5, "X"), edit(1, 2, "Y")]),
        "aYcdXf"
    );
    assert_eq!(
        splice("abcdef", vec![edit(2, 3, "inner"), edit(1, 4, "OUT")]),
        "aOUTef"
    );
    assert_eq!(splice("abcdef", vec![edit(2, 2, "+")]), "ab+cdef");
    assert_eq!(splice("abcdef", vec![]), "abcdef");
}
