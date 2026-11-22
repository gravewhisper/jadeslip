use crate::books::finish_plain;
use crate::epub::{Item, Kind, Package, TocNode};
use crate::plants::Counts;
use crate::truth::Truth;
use crate::zip::ZipWriter;
use std::io;

const BOOK_FONT: &[u8] =
    include_bytes!("../../../app/ui/fonts/ibm-plex-mono-latin-400-normal.woff2");
const HUGE_LABEL_CHARS: usize = 100_000;
const DEEP_LEVELS: usize = 40;
pub const LABEL_MARK: &str = "tocprobe";
pub const FONT_FAMILY: &str = "JadeslipBookMono";

fn plain_page(title: &str, head: &str, body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title>{head}</head><body>{body}</body></html>"
    )
    .into_bytes()
}

fn beacon(port: u16, id: &str) -> String {
    format!(
        "<img src=x onerror=\"fetch('http://127.0.0.1:{port}/C/{id}')\"><script>fetch('http://127.0.0.1:{port}/C/{id}')</script></span><iframe src='http://127.0.0.1:{port}/C/{id}'></iframe>"
    )
}

fn hostile_labels(port: u16) -> Vec<(String, String)> {
    vec![
        (
            format!("{LABEL_MARK} markup {}", beacon(port, "toc_markup")),
            "plain".to_owned(),
        ),
        (
            format!(
                "{LABEL_MARK} entity &lt;b&gt; &amp;amp; {}",
                beacon(port, "toc_entity")
            ),
            "plain".to_owned(),
        ),
        (
            format!("{LABEL_MARK} \u{202E}reversed\u{202C}"),
            "plain".to_owned(),
        ),
        (
            format!("{LABEL_MARK} huge {}", "x".repeat(HUGE_LABEL_CHARS)),
            "plain".to_owned(),
        ),
        (String::new(), "plain".to_owned()),
    ]
}

fn deep_node(level: usize) -> TocNode {
    if level == 0 {
        return TocNode::link(format!("{LABEL_MARK} deepest"), "Text/plain.xhtml");
    }
    TocNode {
        label: format!("{LABEL_MARK} level {level}"),
        target: None,
        children: vec![deep_node(level - 1)],
    }
}

fn hostile_targets(port: u16) -> Vec<TocNode> {
    [
        (
            "script",
            "javascript:fetch('http://127.0.0.1:PORT/C/toc_script')".to_owned(),
        ),
        ("remote", "http://127.0.0.1:PORT/C/toc_remote".to_owned()),
        ("climb", "../../../../etc/passwd".to_owned()),
        ("gone", "Text/not-in-this-book.xhtml".to_owned()),
    ]
    .into_iter()
    .map(|(name, target)| {
        TocNode::link(
            format!("{LABEL_MARK} target {name}"),
            target.replace("PORT", &port.to_string()),
        )
    })
    .collect()
}

pub fn toc_hostile(port: u16, zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("toc-hostile", "Toc Hostile");
    package.items.push(Item::nav(false));
    package.items.push(Item::page(
        "plain",
        "Text/plain.xhtml",
        Kind::Text,
        Some("Plain chapter"),
        plain_page(
            "Plain chapter",
            "",
            "<h1>Plain chapter</h1><p>The only chapter.</p>",
        ),
    ));
    for (label, name) in hostile_labels(port) {
        package
            .toc
            .push(TocNode::link(label, format!("Text/{name}.xhtml")));
    }
    package.toc.extend(hostile_targets(port));
    package.toc.push(deep_node(DEEP_LEVELS));
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(finish_plain(&package, "toc", counts))
}

pub fn headings_hostile(port: u16, zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("headings-hostile", "Headings Hostile");
    for ordinal in 0..4 {
        let id = format!("h{ordinal}");
        let heading = format!(
            "{LABEL_MARK} heading {ordinal} &lt;img src=x onerror=&quot;fetch('http://127.0.0.1:{port}/C/heading_{ordinal}')&quot;&gt;"
        );
        package.items.push(Item::page(
            &id,
            &format!("Text/{id}.xhtml"),
            Kind::Text,
            None,
            plain_page(
                &id,
                "",
                &format!("<h1>{heading}</h1><p>Words under the heading.</p>"),
            ),
        ));
    }
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(finish_plain(&package, "toc", counts))
}

const STYLED: &[(&str, &str)] = &[
    ("hide_body", "body{display:none}"),
    (
        "hide_important",
        "body{display:none !important;visibility:hidden !important}",
    ),
    (
        "hide_scope",
        ".ss-book{display:none}.ss-body{opacity:0 !important;position:absolute;left:-9999px}",
    ),
    (
        "hide_strong",
        "html body{height:0 !important;overflow:hidden !important;opacity:0 !important;visibility:hidden !important}",
    ),
    (
        "hide_outrank",
        "html:not(#a#b#c#d#e),body:not(#a#b#c#d#e){display:none!important;visibility:hidden!important;opacity:0!important}",
    ),
    (
        "ink_outrank",
        "p:not(#a#b#c#d#e){color:#000000!important;background-color:#FFFFFF!important}",
    ),
    (
        "ink_dark",
        "body{color:#14161B !important;background:#FFFFFF !important}p{color:#000000 !important}a{color:#0000EE}",
    ),
    ("own_margin", "p{margin:0;text-indent:2em}"),
    ("hidden_note", "p.note{display:none}"),
];

fn styled_body(id: &str) -> String {
    if id == "hidden_note" {
        return format!("<h1>{id}</h1><p>Shown words.</p><p class=\"note\">Hidden note.</p>");
    }
    format!(
        "<h1>{id}</h1><p>The words of this chapter stay readable.</p><p>A <a href=\"font_face.xhtml\">link</a> sits here.</p>"
    )
}

pub fn reader_css(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("reader-css", "Reader Css");
    package.items.push(Item::nav(false));
    package.items.push(Item::resource(
        "book_font",
        "Res/book-mono.woff2",
        "font/woff2",
        BOOK_FONT.to_vec(),
    ));
    let font = format!(
        "<style>@font-face{{font-family:'{FONT_FAMILY}';src:url(\"../Res/book-mono.woff2\") format(\"woff2\")}}#probe{{font-family:'{FONT_FAMILY}',serif;font-size:40px;display:inline-block}}#control{{font-family:serif;font-size:40px;display:inline-block}}</style>"
    );
    package.items.push(Item::page(
        "font_face",
        "Text/font_face.xhtml",
        Kind::Text,
        Some("font_face"),
        plain_page("font_face", &font, "<h1>font_face</h1><p><span id=\"probe\">iiiiiiiiii</span></p><p><span id=\"control\">iiiiiiiiii</span></p>"),
    ));
    package
        .toc
        .push(TocNode::link("font_face", "Text/font_face.xhtml"));
    let shell_name = "<style>@font-face{font-family:'IBM Plex Sans';src:url(\"../Res/book-mono.woff2\") format(\"woff2\")}@font-face{font-family:'Literata';src:url(\"../Res/book-mono.woff2\") format(\"woff2\")}</style>";
    package.items.push(Item::page(
        "font_shell_name",
        "Text/font_shell_name.xhtml",
        Kind::Text,
        Some("font_shell_name"),
        plain_page(
            "font_shell_name",
            shell_name,
            "<h1>font_shell_name</h1><p>Words in a font that borrows the name of a shell font.</p>",
        ),
    ));
    package.toc.push(TocNode::link(
        "font_shell_name",
        "Text/font_shell_name.xhtml",
    ));
    for (id, css) in STYLED {
        let head = format!("<style>{css}</style>");
        package.items.push(Item::page(
            id,
            &format!("Text/{id}.xhtml"),
            Kind::Text,
            Some(id),
            plain_page(id, &head, &styled_body(id)),
        ));
        package
            .toc
            .push(TocNode::link(*id, format!("Text/{id}.xhtml")));
    }
    let body = styled_body("hide_inline");
    package.items.push(Item::page(
        "hide_inline",
        "Text/hide_inline.xhtml",
        Kind::Text,
        Some("hide_inline"),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>hide_inline</title></head><body style=\"display:none!important;visibility:hidden!important;opacity:0!important;height:0!important;overflow:hidden!important\">{body}</body></html>"
        )
        .into_bytes(),
    ));
    package
        .toc
        .push(TocNode::link("hide_inline", "Text/hide_inline.xhtml"));
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    Ok(finish_plain(&package, "reader", counts))
}
