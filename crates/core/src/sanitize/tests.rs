use super::*;
use crate::DEFAULT_LIMITS;
use crate::chapter::BlockLayout;
use crate::markup::scan;
use proptest::prelude::*;

const SESSION: u64 = 0xabc;

fn name(entry: &str) -> EntryName {
    EntryName::parse(entry).unwrap()
}

fn map() -> UrlMap {
    UrlMap::new(SESSION, name("OEBPS/Text/c1.xhtml"))
}

fn everything_serveable(_: &EntryName) -> bool {
    true
}

fn nothing_is_a_sheet(_: &EntryName) -> bool {
    false
}

fn render(source: &str) -> Sanitized {
    let scanned = scan(source, &DEFAULT_LIMITS).unwrap();
    let (layout, ranges) = BlockLayout::build(&scanned.events, source);
    let context = Context {
        urls: ResourceUrls {
            map: map(),
            serveable: &everything_serveable,
        },
        is_stylesheet: &nothing_is_a_sheet,
    };
    sanitize(
        &scanned.events,
        source.len(),
        &Layout {
            blocks: &layout.blocks,
            ranges: &ranges,
            body_inner: layout.body_inner,
        },
        &context,
    )
}

fn inner(source: &str) -> String {
    let xhtml = render(source).xhtml;
    let start = xhtml.find('>').unwrap() + 1;
    xhtml[start..xhtml.len() - "</div>".len()].to_owned()
}

fn css(source: &str) -> String {
    let urls = ResourceUrls {
        map: map(),
        serveable: &everything_serveable,
    };
    crate::css::sanitize_stylesheet(source, &urls)
}

fn inline(source: &str) -> String {
    let urls = ResourceUrls {
        map: map(),
        serveable: &everything_serveable,
    };
    crate::css::sanitize_inline(source, &urls)
}

#[test]
fn lookup_tables_are_sorted_for_binary_search() {
    for table in [
        HTML_ATTRS,
        SVG_ELEMENTS,
        SVG_ATTRS,
        MATH_ELEMENTS,
        MATH_ATTRS,
    ] {
        assert!(table.windows(2).all(|pair| pair[0] < pair[1]), "{table:?}");
    }
    assert!(HTML_ATTRS.len() <= 64 && MATH_ATTRS.len() <= 64 && SVG_ATTRS.len() <= 128);
}

#[test]
fn scheme_urls_invert_to_relative_source_hrefs() {
    let map = map();
    let cases = [
        (
            "../Res/safe.png",
            "jadeslip-book://0000000000000abc/OEBPS/Res/safe.png",
            "../Res/safe.png",
        ),
        (
            "pic.png#f",
            "jadeslip-book://0000000000000abc/OEBPS/Text/pic.png#f",
            "pic.png#f",
        ),
        (
            "../../top.png",
            "jadeslip-book://0000000000000abc/top.png",
            "../../top.png",
        ),
        (
            "../Res/a%20b.png",
            "jadeslip-book://0000000000000abc/OEBPS/Res/a%20b.png",
            "../Res/a%20b.png",
        ),
        (
            "../Res/caf\u{e9}.png",
            "jadeslip-book://0000000000000abc/OEBPS/Res/caf%C3%A9.png",
            "../Res/caf%C3%A9.png",
        ),
    ];
    for (source, url, back) in cases {
        assert_eq!(map.to_scheme(source).as_deref(), Some(url), "{source}");
        assert_eq!(map.to_source_href(url).as_deref(), Some(back), "{url}");
    }
}

#[test]
fn scheme_mapping_refuses_everything_outside_the_book() {
    let map = map();
    for href in [
        "http://evil/x.png",
        "javascript:alert(1)",
        "/etc/passwd",
        "../../../etc/passwd",
        "..\\..\\x",
        "//evil/x",
        "",
    ] {
        assert_eq!(map.to_scheme(href), None, "{href}");
    }
    for url in [
        "jadeslip-book://forged/OEBPS/x.png",
        "jadeslip-book://0000000000000abd/OEBPS/x.png",
        "jadeslip-book://0000000000000abc/../../etc/passwd",
        "http://0000000000000abc/OEBPS/x.png",
    ] {
        assert_eq!(map.to_source_href(url), None, "{url}");
    }
}

proptest! {
    #[test]
    fn to_scheme_and_to_source_href_are_inverses(
        dirs in prop::collection::vec("[a-zA-Z0-9 é%_.-]{1,6}", 0..4),
        file in "[a-zA-Z0-9 é_-]{1,8}\\.png",
        fragment in prop::option::of("[a-z0-9]{1,5}"),
    ) {
        let dirs: Vec<String> = dirs
            .into_iter()
            .filter(|dir| dir != "." && dir != ".." && !dir.contains('%'))
            .collect();
        let mut path = dirs.join("/");
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(&file);
        let encoded = crate::package::percent_encode_path(&path);
        let suffix = fragment.map_or(String::new(), |fragment| format!("#{fragment}"));
        let map = UrlMap::new(SESSION, name("OEBPS/Text/c1.xhtml"));
        let href = format!("../../{encoded}{suffix}");
        let url = map.to_scheme(&href).expect("the href stays in the book");
        let back = map.to_source_href(&url).expect("the session matches");
        prop_assert_eq!(map.to_scheme(&back), Some(url.clone()));
        prop_assert_eq!(back, href);
    }
}

#[test]
fn hrefs_are_classified_by_what_a_browser_would_do() {
    for (raw, want) in [
        ("c2.xhtml#x", Href::Relative("c2.xhtml#x".into())),
        ("  #x\n", Href::Relative("#x".into())),
        ("a/b:c.xhtml", Href::Relative("a/b:c.xhtml".into())),
        ("http://x.test/", Href::External("http://x.test/".into())),
        ("HTTPS://x.test/", Href::External("HTTPS://x.test/".into())),
        ("mailto:a@x.test", Href::External("mailto:a@x.test".into())),
        ("JaVaScRiPt:x", Href::Blocked),
        ("  javascript:x", Href::Blocked),
        ("java\tscript:x", Href::Blocked),
        ("java\nscript:x", Href::Blocked),
        ("java%73cript:x", Href::Blocked),
        ("data:text/html,x", Href::Blocked),
        ("file:///etc/hostname", Href::Blocked),
        ("ipc://localhost/x", Href::Blocked),
        ("//evil.test/x", Href::Blocked),
        ("/abs", Href::Blocked),
        ("\\\\evil\\x", Href::Blocked),
        ("http:evil.test", Href::Blocked),
    ] {
        assert_eq!(classify_href(raw), want, "{raw:?}");
    }
}

#[test]
fn blocks_and_wrappers_are_marked_and_book_marks_are_stripped() {
    let out = inner(
        "<html><body><div data-ss-w=\"9\"><p data-ss-b=\"7\" onclick=\"x\" class=\"c\">a</p><h2>b</h2></div><p>c</p></body></html>",
    );
    assert_eq!(
        out,
        "<div data-ss-w=\"\"><p class=\"c\" data-ss-b=\"0\">a</p><h2 data-ss-b=\"1\">b</h2></div><p data-ss-b=\"2\">c</p>"
    );
}

#[test]
fn a_body_with_direct_text_makes_the_root_the_block() {
    let xhtml = render("<html lang=\"en\" class=\"h\"><body class=\"b\" onload=\"x\" id=\"top\">a <i>b</i></body></html>").xhtml;
    assert_eq!(
        xhtml,
        "<div xmlns=\"http://www.w3.org/1999/xhtml\" class=\"ss-body b h\" id=\"top\" lang=\"en\" data-ss-b=\"0\">a <i>b</i></div>"
    );
}

#[test]
fn two_bodies_with_text_each_get_their_own_block() {
    let out = inner("<html><body>a</body><body>b</body></html>");
    assert!(out.contains("<div data-ss-b=\"0\">a</div>"), "{out}");
    assert!(out.contains("<div data-ss-b=\"1\">b</div>"), "{out}");
}

#[test]
fn content_after_a_lone_inner_block_keeps_it_out_of_the_root() {
    assert_eq!(
        inner("<html>a</html> <div></div>"),
        "<div data-ss-b=\"0\">a</div> <div data-ss-w=\"\"></div>"
    );
}

#[test]
fn text_follows_the_point_fix_semantics() {
    assert_eq!(
        inner("<p>a\u{1}b\u{c}c\u{fffe}d</p>"),
        "<p data-ss-b=\"0\">ab cd</p>"
    );
    assert_eq!(
        inner("<p>&amp;&lt;&#13;&#xA0;&nosuch;&#0;<![CDATA[<&amp;]]></p>"),
        "<p data-ss-b=\"0\">&amp;&lt;&#13;\u{a0}&amp;nosuch;&amp;#0;&lt;&amp;amp;</p>"
    );
}

#[test]
fn attribute_values_are_rebuilt_and_duplicates_dropped() {
    assert_eq!(
        inner("<p title=\"a&quot;b<c&#10;\u{1}\" TITLE=\"second\" id=x>t</p>"),
        "<p title=\"a&quot;b&lt;c&#10;\" id=\"x\" data-ss-b=\"0\">t</p>"
    );
}

#[test]
fn links_keep_only_allowed_hrefs_and_only_on_anchors() {
    assert_eq!(
        inner(
            "<p><a href=\"c2.xhtml#x\" target=\"_top\" ping=\"y\" download=\"z\">a</a><a href=\" JAVASCRIPT:x\">b</a><span href=\"c2.xhtml\">c</span></p>"
        ),
        "<p data-ss-b=\"0\"><a href=\"c2.xhtml#x\">a</a><a>b</a><span>c</span></p>"
    );
}

#[test]
fn images_map_into_the_book_scheme_and_lose_everything_else() {
    assert_eq!(
        inner("<p><img src=\"../Res/a.png\" srcset=\"x 1x\" usemap=\"#m\" alt=\"A\"/></p>"),
        "<p data-ss-b=\"0\"><img src=\"jadeslip-book://0000000000000abc/OEBPS/Res/a.png\" alt=\"A\"/></p>"
    );
    assert_eq!(
        inner("<p><img src=\"http://x.test/a.png\" alt=\"A\"></img></p>"),
        "<p data-ss-b=\"0\"><img alt=\"A\"/></p>"
    );
}

#[test]
fn svg_keeps_the_drawing_subset_and_nothing_active() {
    let out = inner(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 5 5\" onload=\"x\"><title>t</title><script>x</script><use href=\"#a\"/><foreignObject><p>x</p></foreignObject><animate/><set/><linearGradient id=\"g\" xlink:href=\"#h\"/><rect fill=\"url(#g)\" stroke=\"url(http://x.test/a)\" transform=\"rotate(3)\" filter=\"u\\72l(x)\"/><a xlink:href=\"javascript:x\"><circle r=\"1\"/></a><image xlink:href=\"p.png\"/><image href=\"http://x.test/p.png\"/></svg>",
    );
    assert_eq!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 5 5\" data-ss-b=\"0\"><linearGradient id=\"g\" href=\"#h\"/><rect fill=\"url(#g)\" transform=\"rotate(3)\"/><circle r=\"1\"/><image href=\"jadeslip-book://0000000000000abc/OEBPS/Text/p.png\"/><image/></svg>"
    );
}

#[test]
fn mathml_keeps_presentation_and_drops_annotation() {
    assert_eq!(
        inner(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" href=\"javascript:x\"><semantics><mi mathvariant=\"bold\">x</mi><annotation>a</annotation><annotation-xml><p>b</p></annotation-xml><maction/></semantics></math>"
        ),
        "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" data-ss-b=\"0\"><semantics><mi mathvariant=\"bold\">x</mi></semantics></math>"
    );
}

#[test]
fn unknown_tags_vanish_but_their_text_stays() {
    assert_eq!(
        inner("<p><blink onclick=\"x\">a</blink></p>"),
        "<p data-ss-b=\"0\">a</p>"
    );
}

#[test]
fn image_page_detection_reads_rendered_content() {
    assert_eq!(
        render("<body><img src=\"a.png\"/></body>").kind,
        ChapterKind::ImagePage
    );
    assert_eq!(
        render("<body><p><svg/></p></body>").kind,
        ChapterKind::ImagePage
    );
    assert_eq!(
        render("<body><p> <br/> </p></body>").kind,
        ChapterKind::Empty
    );
    assert_eq!(render("<body><p>&#160;</p></body>").kind, ChapterKind::Text);
    assert_eq!(
        render("<body><p>a</p><img src=\"a.png\"/></body>").kind,
        ChapterKind::Text
    );
}

#[test]
fn selectors_scope_and_root_elements_map() {
    for (input, want) in [
        ("p{color:red}", ".ss-book p{color:red}"),
        (
            "HTML,BODY{color:red}",
            ".ss-book .ss-body,.ss-book .ss-body{color:red}",
        ),
        (
            "html body p.x{color:red}",
            ".ss-book .ss-body p.x{color:red}",
        ),
        (
            "html.a>body.b{color:red}",
            ".ss-book .ss-body.a.b{color:red}",
        ),
        (
            "body>p+q ~ r{color:red}",
            ".ss-book .ss-body>p+q~r{color:red}",
        ),
        (
            "#a.b[c=\"d\"]:first-child::before{color:red}",
            ".ss-book #a.b[c=\"d\"]:first-child::before{color:red}",
        ),
        (
            "p:not(.a,:is(b)){color:red}",
            ".ss-book p:not(.a,:is(b)){color:red}",
        ),
        (
            "li:nth-child(2n+1){color:red}",
            ".ss-book li:nth-child(2n +1){color:red}",
        ),
        ("*{color:red}", ".ss-book *{color:red}"),
        (":host{color:red}", ""),
        (":root,p{color:red}", ".ss-book p{color:red}"),
        ("::slotted(p){color:red}", ""),
        (":not(:host){color:red}", ""),
        ("a|b{color:red}", ""),
        ("> p{color:red}", ""),
        ("p >{color:red}", ""),
        ("p{}", ""),
    ] {
        assert_eq!(css(input), want, "{input}");
    }
}

#[test]
fn only_media_and_font_face_rules_survive() {
    assert_eq!(
        css("@import url(a.css);@im/**/port url(b.css);p{color:red}"),
        ".ss-book p{color:red}"
    );
    assert_eq!(
        css(
            "@charset \"utf-8\";@namespace x url(y);@keyframes k{from{top:0}}@page{margin:0}@supports (a:b){p{top:0}}"
        ),
        ""
    );
    assert_eq!(
        css("@media screen and (min-width: 3em){p{color:red}@media print{q{top:0}}}"),
        "@media screen and (min-width: 3em){.ss-book p{color:red}@media print{.ss-book q{top:0}}}"
    );
    assert_eq!(css("@media url(x){p{top:0}}"), "");
    assert!(!css("@media url/**/(x){p{top:0}}").contains("url("));
    assert_eq!(
        css("@media scr/**/een and (color){p{top:0}}"),
        "@media scr een and (color){.ss-book p{top:0}}"
    );
    assert_eq!(
        css("@font-face{font-family:f;src:url(f.woff2) format(\"woff2\"),local(\"x\")}"),
        "@font-face{font-family:f;src:url(\"jadeslip-book://0000000000000abc/OEBPS/Text/f.woff2\") format(\"woff2\"),local(\"x\")}"
    );
    assert_eq!(
        css("@font-face{font-family:f;src:url(http://x.test/f.woff2)}"),
        ""
    );
}

#[test]
fn declarations_rebuild_under_the_allowlist() {
    for (input, want) in [
        ("color:red;behavior:url(x);-moz-binding:url(y)", "color:red"),
        ("COLOR : RED ! important", "color:RED!important"),
        (
            "background:url(a.png) no-repeat",
            "background:url(\"jadeslip-book://0000000000000abc/OEBPS/Text/a.png\") no-repeat",
        ),
        (
            "background:URL('a.png')",
            "background:url(\"jadeslip-book://0000000000000abc/OEBPS/Text/a.png\")",
        ),
        ("background:\\75\\72\\6c(http://x.test/a.png)", ""),
        ("background:\\75\\72\\6c(\"http://x.test/a.png\")", ""),
        ("background:u/**/rl(a.png)", ""),
        ("background:url(http://x.test/a.png);color:red", "color:red"),
        ("background:url(javascript:x)", ""),
        ("background:url(data:image/png;base64,x)", ""),
        ("background:var(--x)", ""),
        ("--x:url(http://x.test/)", ""),
        ("background-image:image-set(\"a.png\" 1x)", ""),
        ("background-image:-webkit-image-set(url(a.png) 1x)", ""),
        ("width:expression(alert(1))", ""),
        ("position:fixed;left:0", "left:0"),
        ("position:ST\\49 CKY", ""),
        ("position:relative", "position:relative"),
        (
            "position:absolute !important",
            "position:absolute!important",
        ),
        (
            "fill:url(#g);stroke:url(http://x.test/a)",
            "fill:url(\"#g\")",
        ),
        ("background:url/**/(\"http://x.test/a.png\")", ""),
        ("background:url (\"http://x.test/a.png\")", ""),
        ("background:var/**/(--x)", ""),
        ("background:image-set/**/(\"http://x.test/a.png\" 1x)", ""),
        ("margin:ca/**/lc(1px)", ""),
        (
            "margin:calc(1px + (2px * 3))",
            "margin:calc(1px + (2px * 3))",
        ),
        (
            "color:rgb(1,2,3);margin:calc(1px + 2%)",
            "color:rgb(1,2,3);margin:calc(1px + 2%)",
        ),
        ("content:\"a\\\"b\"", "content:\"a\\\"b\""),
        ("color:red}p{color:blue", ""),
        ("color:red;;;", "color:red"),
        ("<svg/onload=x>", ""),
    ] {
        assert_eq!(inline(input), want, "{input}");
    }
}

#[test]
fn deep_css_nesting_is_refused_without_overflowing_the_stack() {
    let depth = 200_000;
    let selector = format!(
        "{}p{}{{color:red}}",
        ":not(".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(css(&selector), "");
    let value = format!("color:{}{}", "calc(".repeat(depth), ")".repeat(depth));
    assert_eq!(inline(&value), "");
    let media = format!(
        "{}p{{color:red}}{}",
        "@media print{".repeat(depth),
        "}".repeat(depth)
    );
    assert!(css(&media).len() < 1 << 20);
    let blocks = "{".repeat(depth);
    assert_eq!(css(&blocks), "");
}

#[test]
fn a_twice_escaped_head_title_reads_as_text() {
    let page = render(
        "<html><head><title>Mina-chan&#38;apos;s Day &amp;amp; Night</title></head><body><p>Hi</p></body></html>",
    );
    assert_eq!(page.head_title.as_deref(), Some("Mina-chan's Day & Night"));
    let plain =
        render("<html><head><title>Tom &amp; Jerry</title></head><body><p>Hi</p></body></html>");
    assert_eq!(plain.head_title.as_deref(), Some("Tom & Jerry"));
}

#[test]
fn mailto_links_cannot_ask_the_mail_client_to_attach_files() {
    for (href, want) in [
        (
            "mailto:a@x.test?attach=/home/user/.ssh/id_rsa",
            "mailto:a@x.test",
        ),
        (
            "mailto:a@x.test?subject=hi&attachment=file:///home/user/.gnupg/secring.gpg",
            "mailto:a@x.test?subject=hi",
        ),
        (
            "MAILTO:a@x.test,b@x.test?Subject=Hi%20there&ATTACH=x&Body=a%26b&cc=c@x.test&bcc=d@x.test&in-reply-to=x",
            "mailto:a@x.test,b@x.test?Subject=Hi%20there&Body=a%26b&cc=c@x.test&bcc=d@x.test",
        ),
    ] {
        let kept = match classify_href(href) {
            Href::External(url) => url,
            other => panic!("{other:?}"),
        };
        assert_eq!(kept, want, "the reader hands {kept:?} to xdg-open on click");
    }
}

#[test]
fn body_style_cannot_hide_the_chapter_root() {
    let page = render(
        "<html><body style=\"display:none!important;opacity:0!important;visibility:hidden!important\"><p>Hi</p></body></html>",
    );
    let root = &page.xhtml[..page.xhtml.find('>').unwrap()];
    assert!(
        !root.contains("!important"),
        "the .ss-body root carries an element-attached !important style, which beats chapter-view.js's guard: {root}"
    );
}
