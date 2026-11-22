use crate::epub::{Item, Package, TocNode};
use crate::plants::Counts;
use crate::png;
use crate::rng::derive;
use crate::truth::{AttackRowTruth, AttackTruth, Truth};
use crate::zip::{STORED, ZipWriter};
use std::collections::BTreeMap;
use std::io;

const BIG: &str = "display:block;position:fixed;left:0;top:0;width:100%;height:100%;background:#fdd;font-size:40px";

const BEACON: &str = "(function(){document.createElement('img').src='%H%/S/%ID%?ipc='+typeof(window.__TAURI_INTERNALS__);try{window.__TAURI_INTERNALS__.invoke('pwned',{from:'%ID%-internals'})}catch(e){}try{window.__TAURI__.core.invoke('pwned',{from:'%ID%-global'})}catch(e){}try{parent.__TAURI_INTERNALS__.invoke('pwned',{from:'%ID%-parent'})}catch(e){}try{window.webkit.messageHandlers.ipc.postMessage(JSON.stringify({cmd:'pwned',callback:1,error:2,payload:{from:'%ID%-webkit'}}))}catch(e){}try{fetch('ipc://localhost/pwned',{method:'POST',body:JSON.stringify({from:'%ID%-fetch'})})}catch(e){}try{document.documentElement.setAttribute('data-ran-%ID%','1')}catch(e){}})()";

#[derive(Clone, Copy)]
enum Trigger {
    Load,
    Click,
}

struct Row {
    id: &'static str,
    family: &'static str,
    trigger: Trigger,
    well_formed: bool,
    pre: &'static str,
    head: &'static str,
    body_attrs: &'static str,
    body: &'static str,
    resource: Option<&'static str>,
}

const fn load(
    id: &'static str,
    family: &'static str,
    head: &'static str,
    body: &'static str,
) -> Row {
    Row {
        id,
        family,
        trigger: Trigger::Load,
        well_formed: true,
        pre: "",
        head,
        body_attrs: "",
        body,
        resource: None,
    }
}

const fn click(id: &'static str, family: &'static str, body: &'static str) -> Row {
    Row {
        id,
        family,
        trigger: Trigger::Click,
        well_formed: true,
        pre: "",
        head: "",
        body_attrs: "",
        body,
        resource: None,
    }
}

const fn soup(id: &'static str, body: &'static str) -> Row {
    Row {
        id,
        family: "tag-soup",
        trigger: Trigger::Load,
        well_formed: false,
        pre: "",
        head: "",
        body_attrs: "",
        body,
        resource: None,
    }
}

const fn with_pre(
    id: &'static str,
    family: &'static str,
    pre: &'static str,
    well_formed: bool,
) -> Row {
    Row {
        id,
        family,
        trigger: Trigger::Load,
        well_formed,
        pre,
        head: "",
        body_attrs: "",
        body: "",
        resource: None,
    }
}

const fn with_resource(
    id: &'static str,
    family: &'static str,
    head: &'static str,
    body: &'static str,
    resource: &'static str,
) -> Row {
    Row {
        id,
        family,
        trigger: Trigger::Load,
        well_formed: true,
        pre: "",
        head,
        body_attrs: "",
        body,
        resource: Some(resource),
    }
}

const ROWS: &[Row] = &[
    load("script_inline_head", "script", "<script>%JS%</script>", ""),
    load("script_inline_body", "script", "", "<script>%JS%</script>"),
    load(
        "script_cdata",
        "script",
        "",
        "<script>//<![CDATA[\n%JS%\n//]]></script>",
    ),
    load(
        "script_src_remote",
        "script",
        "<script src=\"%H%/R/%ID%.js\"></script>",
        "",
    ),
    with_resource(
        "script_src_local",
        "script",
        "<script src=\"../Res/evil.js\"></script>",
        "",
        "evil.js",
    ),
    load(
        "script_svg_inline",
        "script",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><script>%JS%</script></svg>",
    ),
    load(
        "script_svg_xlink",
        "script",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"5\" height=\"5\"><script xlink:href=\"%H%/R/%ID%.js\"></script></svg>",
    ),
    load(
        "script_namespaced",
        "script",
        "",
        "<x:script xmlns:x=\"http://www.w3.org/1999/xhtml\">%JS%</x:script>",
    ),
    load(
        "event_body_onload",
        "event",
        "",
        "<div onload=\"%JS%\"><img src=\"x-invalid:\" onerror=\"%JS%\" alt=\"\"/></div>",
    ),
    Row {
        body_attrs: " onload=\"%JS%\"",
        ..load("event_body_attr", "event", "", "")
    },
    load(
        "event_img_onerror",
        "event",
        "",
        "<img src=\"x-invalid:\" onerror=\"%JS%\" alt=\"\"/>",
    ),
    load(
        "event_img_onerror_upper",
        "event",
        "",
        "<img src=\"x-invalid:\" ONERROR=\"%JS%\" alt=\"\"/>",
    ),
    load(
        "event_svg_onload",
        "event",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"%JS%\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "event_svg_animate_onbegin",
        "event",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><animate attributeName=\"x\" dur=\"1s\" onbegin=\"%JS%\"/></svg>",
    ),
    load(
        "event_details_ontoggle",
        "event",
        "",
        "<details open=\"open\" ontoggle=\"%JS%\"><summary>d</summary></details>",
    ),
    load(
        "event_video_onerror",
        "event",
        "",
        "<video src=\"x-invalid:\" onerror=\"%JS%\"></video>",
    ),
    load(
        "event_audio_onerror",
        "event",
        "",
        "<audio src=\"x-invalid:\" onerror=\"%JS%\"></audio>",
    ),
    load(
        "event_input_onfocus",
        "event",
        "",
        "<input autofocus=\"autofocus\" onfocus=\"%JS%\"/>",
    ),
    load(
        "event_marquee_onstart",
        "event",
        "",
        "<marquee onstart=\"%JS%\">m</marquee>",
    ),
    load(
        "event_style_onload",
        "event",
        "",
        "<style onload=\"%JS%\">p{}</style>",
    ),
    load(
        "frame_iframe_javascript",
        "frame",
        "",
        "<iframe src=\"javascript:parent.postMessage('ran:%ID%','*');void(document.createElement('img').src='%H%/S/%ID%')\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "frame_iframe_srcdoc",
        "frame",
        "",
        "<iframe srcdoc=\"&lt;script&gt;%JS%&lt;/script&gt;\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "frame_iframe_data",
        "frame",
        "",
        "<iframe src=\"data:text/html,&lt;script&gt;%JS%&lt;/script&gt;\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "frame_iframe_data_base64",
        "frame",
        "",
        "<iframe src=\"data:text/html;base64,PHNjcmlwdD5wYXJlbnQucG9zdE1lc3NhZ2UoJ3JhbicsJyonKTwvc2NyaXB0Pg==\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "frame_iframe_remote",
        "frame",
        "",
        "<iframe src=\"%H%/R/%ID%.html\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "frame_object_javascript",
        "frame",
        "",
        "<object data=\"javascript:parent.postMessage('ran:%ID%','*')\" width=\"5\" height=\"5\"></object>",
    ),
    load(
        "frame_object_remote",
        "frame",
        "",
        "<object data=\"%H%/R/%ID%.html\" width=\"5\" height=\"5\"></object>",
    ),
    load(
        "frame_embed_javascript",
        "frame",
        "",
        "<embed src=\"javascript:parent.postMessage('ran:%ID%','*')\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "frame_embed_remote",
        "frame",
        "",
        "<embed src=\"%H%/R/%ID%.html\" width=\"5\" height=\"5\"/>",
    ),
    with_resource(
        "svg_local_img",
        "svg-document",
        "",
        "<img src=\"../Res/evil_img.svg\" alt=\"\" width=\"5\" height=\"5\"/>",
        "evil_img.svg",
    ),
    with_resource(
        "svg_local_iframe",
        "svg-document",
        "",
        "<iframe src=\"../Res/evil_iframe.svg\" width=\"5\" height=\"5\"></iframe>",
        "evil_iframe.svg",
    ),
    with_resource(
        "svg_local_object",
        "svg-document",
        "",
        "<object data=\"../Res/evil_object.svg\" type=\"image/svg+xml\" width=\"5\" height=\"5\"></object>",
        "evil_object.svg",
    ),
    with_resource(
        "svg_local_embed",
        "svg-document",
        "",
        "<embed src=\"../Res/evil_embed.svg\" type=\"image/svg+xml\" width=\"5\" height=\"5\"/>",
        "evil_embed.svg",
    ),
    load(
        "resource_img",
        "resource",
        "",
        "<img src=\"%H%/R/%ID%.png\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "resource_img_srcset",
        "resource",
        "",
        "<img srcset=\"%H%/R/%ID%.png 1x\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "resource_picture_source",
        "resource",
        "",
        "<picture><source srcset=\"%H%/R/%ID%.png\"/><img src=\"data:,\" alt=\"\" width=\"5\" height=\"5\"/></picture>",
    ),
    load(
        "resource_video_poster",
        "resource",
        "",
        "<video poster=\"%H%/R/%ID%.png\" width=\"5\" height=\"5\"></video>",
    ),
    load(
        "resource_video_src",
        "resource",
        "",
        "<video src=\"%H%/R/%ID%.webm\" preload=\"auto\" width=\"5\" height=\"5\"></video>",
    ),
    load(
        "resource_video_source",
        "resource",
        "",
        "<video preload=\"auto\" width=\"5\" height=\"5\"><source src=\"%H%/R/%ID%.webm\"/></video>",
    ),
    load(
        "resource_audio_src",
        "resource",
        "",
        "<audio src=\"%H%/R/%ID%.ogg\" preload=\"auto\"></audio>",
    ),
    load(
        "resource_track",
        "resource",
        "",
        "<video width=\"5\" height=\"5\"><track default=\"default\" src=\"%H%/R/%ID%.vtt\"/></video>",
    ),
    load(
        "resource_input_image",
        "resource",
        "",
        "<input type=\"image\" src=\"%H%/R/%ID%.png\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "resource_svg_image",
        "resource",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><image href=\"%H%/R/%ID%.png\" width=\"5\" height=\"5\"/></svg>",
    ),
    load(
        "resource_svg_image_xlink",
        "resource",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"5\" height=\"5\"><image xlink:href=\"%H%/R/%ID%.png\" width=\"5\" height=\"5\"/></svg>",
    ),
    load(
        "resource_svg_use",
        "resource",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><use href=\"%H%/R/%ID%.svg#x\"/></svg>",
    ),
    load(
        "resource_svg_foreign_object",
        "resource",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><foreignObject width=\"5\" height=\"5\"><img xmlns=\"http://www.w3.org/1999/xhtml\" src=\"%H%/R/%ID%.png\" alt=\"\"/></foreignObject></svg>",
    ),
    load(
        "resource_link_stylesheet",
        "link",
        "<link rel=\"stylesheet\" href=\"%H%/R/%ID%.css\"/>",
        "",
    ),
    load(
        "resource_link_prefetch",
        "link",
        "<link rel=\"prefetch\" href=\"%H%/R/%ID%\"/>",
        "",
    ),
    load(
        "resource_link_preload",
        "link",
        "<link rel=\"preload\" as=\"image\" href=\"%H%/R/%ID%.png\"/>",
        "",
    ),
    load(
        "resource_link_icon",
        "link",
        "<link rel=\"icon\" href=\"%H%/R/%ID%.png\"/>",
        "",
    ),
    load(
        "resource_link_body",
        "link",
        "",
        "<link rel=\"stylesheet\" href=\"%H%/R/%ID%.css\"/>",
    ),
    with_resource(
        "css_file_import",
        "css",
        "<link rel=\"stylesheet\" href=\"../Res/evil.css\"/>",
        "<p id=\"cssfile\">styled</p>",
        "evil.css",
    ),
    load(
        "css_import",
        "css",
        "<style>@import url(\"%H%/R/%ID%.css\");</style>",
        "",
    ),
    load(
        "css_import_bare",
        "css",
        "<style>@import \"%H%/R/%ID%.css\";</style>",
        "",
    ),
    load(
        "css_font_face",
        "css",
        "<style>@font-face{font-family:evil;src:url(\"%H%/R/%ID%.woff2\") format(\"woff2\")}p{font-family:evil}</style>",
        "<p>font user</p>",
    ),
    load(
        "css_background",
        "css",
        "<style>p{background:url(\"%H%/R/%ID%.png\")}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_cursor",
        "css",
        "<style>p{cursor:url(\"%H%/R/%ID%.png\"),auto}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_list_style",
        "css",
        "<style>li{list-style-image:url(\"%H%/R/%ID%.png\")}</style>",
        "<ul><li>x</li></ul>",
    ),
    load(
        "css_border_image",
        "css",
        "<style>p{border:1px solid;border-image:url(\"%H%/R/%ID%.png\") 1}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_mask",
        "css",
        "<style>p{-webkit-mask-image:url(\"%H%/R/%ID%.png\")}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_filter",
        "css",
        "<style>p{filter:url(\"%H%/R/%ID%.svg#x\")}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_image_set",
        "css",
        "<style>p{background-image:-webkit-image-set(url(\"%H%/R/%ID%.png\") 1x)}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_escaped_url",
        "css",
        "<style>p{background:\\75\\72\\6c(\"%H%/R/%ID%.png\")}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_comment_split",
        "css",
        "<style>@im/**/port url(\"%H%/R/%ID%.css\");p{background:u/**/rl(\"%H%/R/%ID%.png\")}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_custom_property",
        "css",
        "<style>p{--x:url(\"%H%/R/%ID%.png\");background:var(--x)}</style>",
        "<p>styled</p>",
    ),
    load(
        "css_attribute_exfil",
        "css",
        "<style>p[title^=\"s\"]{background:url(\"%H%/R/%ID%.png\")}</style>",
        "<p title=\"secret\">styled</p>",
    ),
    load(
        "css_inline_url",
        "css",
        "",
        "<p style=\"background:url('%H%/R/%ID%.png')\">styled</p>",
    ),
    load(
        "css_inline_entity_url",
        "css",
        "",
        "<p style=\"background:&#117;rl('%H%/R/%ID%.png')\">styled</p>",
    ),
    load(
        "css_javascript_url",
        "css",
        "",
        "<p style=\"background:url('javascript:parent.postMessage(1,1)')\">styled</p>",
    ),
    with_resource(
        "css_svg_background",
        "css",
        "<style>p{background:url(\"../Res/evil_cssbg.svg\")}</style>",
        "<p>styled</p>",
        "evil_cssbg.svg",
    ),
    load(
        "css_overlay_fixed",
        "css-escape",
        "",
        "<div id=\"fixedov\" style=\"position:fixed;left:0;top:0;width:100%;height:100%;z-index:2147483647;background:rgba(255,0,0,.3)\">over</div>",
    ),
    load(
        "css_host_escape",
        "css-escape",
        "<style>:host{display:none}html,body{display:none}:root{display:none}*{visibility:hidden}</style>",
        "<p>styled</p>",
    ),
    load(
        "doc_base",
        "document",
        "<base href=\"%H%/R/%ID%/\"/>",
        "<img src=\"relative.png\" alt=\"\" width=\"5\" height=\"5\"/><a href=\"relative\">x</a>",
    ),
    load(
        "doc_meta_refresh_remote",
        "document",
        "<meta http-equiv=\"refresh\" content=\"0;url=%H%/R/%ID%.html\"/>",
        "",
    ),
    load(
        "doc_meta_refresh_javascript",
        "document",
        "<meta http-equiv=\"refresh\" content=\"0;url=javascript:parent.postMessage('ran:%ID%','*')\"/>",
        "",
    ),
    load(
        "doc_meta_csp_relax",
        "document",
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src * 'unsafe-inline'\"/><script>%JS%</script>",
        "",
    ),
    with_pre(
        "doc_xml_stylesheet",
        "document",
        "<?xml-stylesheet type=\"text/xsl\" href=\"%H%/R/%ID%.xsl\"?>\n",
        true,
    ),
    with_pre(
        "doc_doctype_external",
        "dtd",
        "<!DOCTYPE html SYSTEM \"%H%/R/%ID%.dtd\">\n",
        true,
    ),
    Row {
        body: "<p>&lol4;</p>",
        ..with_pre(
            "doc_entity_expansion",
            "dtd",
            "<!DOCTYPE html [<!ENTITY lol \"lol\"><!ENTITY lol1 \"&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;\"><!ENTITY lol2 \"&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;\"><!ENTITY lol3 \"&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;\"><!ENTITY lol4 \"&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;\">]>\n",
            true,
        )
    },
    Row {
        body: "<p>&ext;</p>",
        ..with_pre(
            "doc_entity_external_file",
            "dtd",
            "<!DOCTYPE html [<!ENTITY ext SYSTEM \"file:///etc/hostname\">]>\n",
            true,
        )
    },
    Row {
        body: "<p>&ext;</p>",
        ..with_pre(
            "doc_entity_external_remote",
            "dtd",
            "<!DOCTYPE html [<!ENTITY ext SYSTEM \"%H%/R/%ID%.txt\">]>\n",
            true,
        )
    },
    load(
        "doc_xinclude",
        "document",
        "",
        "<xi:include xmlns:xi=\"http://www.w3.org/2001/XInclude\" href=\"file:///etc/hostname\" parse=\"text\"/>",
    ),
    load(
        "scheme_img_ipc",
        "scheme",
        "",
        "<img src=\"ipc://localhost/pwned\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "scheme_iframe_ipc",
        "scheme",
        "",
        "<iframe src=\"ipc://localhost/pwned\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "scheme_iframe_tauri",
        "scheme",
        "",
        "<iframe src=\"tauri://localhost/index.html\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "scheme_iframe_app",
        "scheme",
        "",
        "<iframe src=\"jadeslip://app/index.html\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "scheme_img_file",
        "scheme",
        "",
        "<img src=\"file:///etc/hostname\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "scheme_iframe_file",
        "scheme",
        "",
        "<iframe src=\"file:///etc/hostname\" width=\"5\" height=\"5\"></iframe>",
    ),
    load(
        "scheme_img_book_traversal",
        "scheme",
        "",
        "<img src=\"jadeslip-book://forged-token/../../../../etc/hostname\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "scheme_img_relative_traversal",
        "scheme",
        "",
        "<img src=\"../../../../../../etc/hostname\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    load(
        "scheme_img_chapter_as_image",
        "scheme",
        "",
        "<img src=\"control_plain.xhtml\" alt=\"\" width=\"5\" height=\"5\"/>",
    ),
    with_resource(
        "scheme_img_unlisted",
        "scheme",
        "",
        "<img src=\"../Res/unlisted.png\" alt=\"\" width=\"5\" height=\"5\"/>",
        "unlisted.png",
    ),
    click(
        "click_a_remote",
        "click-navigation",
        "<a href=\"%H%/R/%ID%.html\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_target_top",
        "click-navigation",
        "<a href=\"%H%/R/%ID%.html\" target=\"_top\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_target_parent",
        "click-navigation",
        "<a href=\"%H%/R/%ID%.html\" target=\"_parent\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_target_blank",
        "click-navigation",
        "<a href=\"%H%/R/%ID%.html\" target=\"_blank\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_ping",
        "click-navigation",
        "<a href=\"#x\" ping=\"%H%/R/%ID%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_download",
        "click-navigation",
        "<a href=\"%H%/R/%ID%.bin\" download=\"x\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_svg_document",
        "click-navigation",
        "<a href=\"../Res/evil_nav.svg\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_ipc",
        "click-scheme",
        "<a href=\"ipc://localhost/pwned\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_app",
        "click-scheme",
        "<a href=\"jadeslip://app/index.html\" target=\"_top\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_file",
        "click-scheme",
        "<a href=\"file:///etc/hostname\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_a_data",
        "click-scheme",
        "<a href=\"data:text/html,&lt;script&gt;%JS%&lt;/script&gt;\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_plain",
        "click-javascript",
        "<a href=\"javascript:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_mixed_case",
        "click-javascript",
        "<a href=\"JaVaScRiPt:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_leading_space",
        "click-javascript",
        "<a href=\"   javascript:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_entity_decimal",
        "click-javascript",
        "<a href=\"java&#115;cript:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_entity_hex",
        "click-javascript",
        "<a href=\"&#x6A;avascript:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_entity_tab",
        "click-javascript",
        "<a href=\"java&#9;script:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_entity_newline",
        "click-javascript",
        "<a href=\"java&#10;script:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_js_percent",
        "click-javascript",
        "<a href=\"java%73cript:%JS%\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_vbscript",
        "click-javascript",
        "<a href=\"vbscript:msgbox(1)\" style=\"%BIG%\">go</a>",
    ),
    click(
        "click_svg_a_href",
        "click-javascript",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"%BIG%\"><a href=\"javascript:%JS%\"><rect width=\"100%\" height=\"100%\" fill=\"red\"/></a></svg>",
    ),
    click(
        "click_svg_a_xlink",
        "click-javascript",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" style=\"%BIG%\"><a xlink:href=\"javascript:%JS%\"><rect width=\"100%\" height=\"100%\" fill=\"red\"/></a></svg>",
    ),
    click(
        "click_svg_set_href",
        "click-javascript",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"%BIG%\"><a><set attributeName=\"href\" to=\"javascript:%JS%\"/><rect width=\"100%\" height=\"100%\" fill=\"red\"/></a></svg>",
    ),
    click(
        "click_math_href",
        "click-javascript",
        "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" href=\"javascript:%JS%\" style=\"%BIG%\"><mtext>go</mtext></math>",
    ),
    click(
        "click_area",
        "click-javascript",
        "<img usemap=\"#m\" src=\"../Res/safe.png\" alt=\"\" style=\"%BIG%\"/><map name=\"m\"><area shape=\"default\" href=\"javascript:%JS%\" alt=\"\"/></map>",
    ),
    click(
        "click_form_get",
        "click-form",
        "<form action=\"%H%/R/%ID%.html\" method=\"get\"><input type=\"hidden\" name=\"q\" value=\"1\"/><button style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_form_post",
        "click-form",
        "<form action=\"%H%/R/%ID%.html\" method=\"post\"><input type=\"hidden\" name=\"q\" value=\"1\"/><button style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_form_top",
        "click-form",
        "<form action=\"%H%/R/%ID%.html\" target=\"_top\"><button style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_form_javascript",
        "click-form",
        "<form action=\"javascript:%JS%\"><button style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_form_ipc",
        "click-form",
        "<form action=\"ipc://localhost/pwned\" method=\"post\" enctype=\"text/plain\"><input type=\"hidden\" name=\"from\" value=\"form\"/><button style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_button_formaction",
        "click-form",
        "<form><button formaction=\"%H%/R/%ID%.html\" style=\"%BIG%\">go</button></form>",
    ),
    click(
        "click_onclick",
        "click-event",
        "<div onclick=\"%JS%\" style=\"%BIG%\">go</div>",
    ),
    load(
        "forge_data_ss_block",
        "forgery",
        "",
        "<div data-ss-b=\"0\" data-ss-w=\"0\"><p data-ss-b=\"9999\">forged block number</p></div>",
    ),
    load(
        "forge_data_ss_orig",
        "forgery",
        "",
        "<img src=\"../Res/safe.png\" data-ss-orig-src=\"%H%/R/%ID%.png\" alt=\"\" width=\"5\" height=\"5\"/><a href=\"#x\" data-ss-orig-href=\"javascript:%JS%\">x</a>",
    ),
    load(
        "forge_scheme_url",
        "forgery",
        "",
        "<a href=\"jadeslip-book://forged-token/OEBPS/Text/control_plain.xhtml\">x</a><img src=\"jadeslip-book://forged-token/OEBPS/Res/safe.png\" alt=\"\"/>",
    ),
    load(
        "forge_id_clobber",
        "forgery",
        "",
        "<img id=\"__TAURI_INTERNALS__\" alt=\"\"/><a id=\"__TAURI__\" name=\"core\" href=\"#x\">x</a><form id=\"ss-root\"><input name=\"getElementById\"/><input name=\"parentNode\"/></form>",
    ),
    load(
        "text_double_decode",
        "text",
        "",
        "<p>&lt;img src=x onerror=&quot;%JS%&quot;&gt;</p><p>&amp;lt;script&amp;gt;%JS%&amp;lt;/script&amp;gt;</p>",
    ),
    load(
        "text_cdata",
        "text",
        "",
        "<p><![CDATA[<img src=x onerror=\"%JS%\"><script>%JS%</script>]]></p>",
    ),
    load(
        "text_comment",
        "text",
        "",
        "<!-- --><script>%JS%</script><!-- -->",
    ),
    load(
        "mxss_template",
        "mxss",
        "",
        "<template><img src=\"x-invalid:\" onerror=\"%JS%\" alt=\"\"/></template>",
    ),
    load(
        "mxss_noscript_title",
        "mxss",
        "",
        "<noscript><p title=\"&lt;/noscript&gt;&lt;img src=x onerror=%JS%&gt;\">n</p></noscript>",
    ),
    load(
        "mxss_svg_title",
        "mxss",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><title>&lt;/title&gt;&lt;img src=x onerror=%JS%&gt;</title></svg>",
    ),
    load(
        "mxss_math_annotation",
        "mxss",
        "",
        "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><annotation-xml encoding=\"text/html\"><img xmlns=\"http://www.w3.org/1999/xhtml\" src=\"x-invalid:\" onerror=\"%JS%\" alt=\"\"/></annotation-xml></math>",
    ),
    load(
        "mxss_svg_foreign_script",
        "mxss",
        "",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><foreignObject><script xmlns=\"http://www.w3.org/1999/xhtml\">%JS%</script></foreignObject></svg>",
    ),
    soup(
        "soup_svg_style_img",
        "<svg><style><img src=x onerror=%JS%></style></svg>",
    ),
    soup(
        "soup_math_mglyph",
        "<math><mtext><table><mglyph><style><!--</style><img title=\"--&gt;&lt;img src=1 onerror=%JS%&gt;\"></mglyph></table></mtext></math>",
    ),
    soup(
        "soup_noscript_close",
        "<noscript><p title=\"</noscript><img src=x onerror=%JS%>\"></noscript>",
    ),
    Row {
        well_formed: true,
        ..soup("soup_comment_break", "<!--><img src=x onerror=%JS%>-->")
    },
    soup("soup_unclosed_img", "<img src=x onerror=%JS%"),
    soup("soup_unquoted_onerror", "<img src=x onerror=%JS%>"),
    soup("soup_slash_separators", "<img/src=x/onerror=%JS%>"),
    Row {
        well_formed: true,
        ..soup("soup_script_upper", "<SCRIPT>%JS%</SCRIPT>")
    },
    soup("soup_script_mixed", "<ScRiPt>%JS%</sCrIpT>"),
    soup("soup_script_slash", "<script/xss>%JS%</script/>"),
    soup("soup_script_unclosed", "<script>%JS%"),
    soup("soup_nul_in_tag", "<scr\u{0}ipt>%JS%</scr\u{0}ipt>"),
    soup(
        "soup_nested_script",
        "<scr<script>ipt>%JS%</scr</script>ipt>",
    ),
    soup(
        "soup_attr_breakout",
        "<p title=\"a\"onmouseover=\"%JS%\" style=\"%BIG%\">go</p>",
    ),
    soup(
        "soup_javascript_unquoted",
        "<a href=javascript:%JS% style=\"%BIG%\">go</a>",
    ),
    load(
        "search_text_img",
        "search-text",
        "",
        "<p>searchprobe &lt;img src=x onerror=&quot;%JS%&quot;&gt; searchprobe</p>",
    ),
    load(
        "search_text_script",
        "search-text",
        "",
        "<p>searchprobe &lt;script&gt;%JS%&lt;/script&gt; &lt;svg onload=&quot;%JS%&quot;/&gt;</p>",
    ),
    load(
        "search_text_breakout",
        "search-text",
        "",
        "<p>&lt;/span&gt;&lt;/button&gt;&lt;img src=x onerror=&quot;%JS%&quot;&gt; searchprobe &lt;/span&gt;&lt;iframe src=&quot;%H%/S/%ID%&quot;&gt;</p>",
    ),
    load(
        "search_text_in_match",
        "search-text",
        "",
        "<p>searchprobe&lt;img src=x onerror=&quot;%JS%&quot;&gt;</p>",
    ),
    load(
        "search_title_markup",
        "search-text",
        "",
        "<p>searchprobe under a hostile contents label</p>",
    ),
    Row {
        id: "encoding_utf7",
        family: "encoding",
        trigger: Trigger::Load,
        well_formed: false,
        pre: "",
        head: "",
        body_attrs: "",
        body: "+ADw-script+AD4-%JS%+ADw-/script+AD4-",
        resource: None,
    },
];

const TITLE_PAYLOAD: &str = "<img src=\"%H%/S/%ID%\" onerror=\"%JS%\"> label";

const RESOURCES: &[(&str, &str)] = &[
    ("evil.js", "application/javascript"),
    ("evil.css", "text/css"),
    ("evil_img.svg", "image/svg+xml"),
    ("evil_iframe.svg", "image/svg+xml"),
    ("evil_object.svg", "image/svg+xml"),
    ("evil_embed.svg", "image/svg+xml"),
    ("evil_nav.svg", "image/svg+xml"),
    ("evil_cssbg.svg", "image/svg+xml"),
];

fn expand(template: &str, id: &str, port: u16) -> String {
    let js = BEACON.replace('&', "&amp;").replace('<', "&lt;");
    template
        .replace("%JS%", &js)
        .replace("%BIG%", BIG)
        .replace("%ID%", id)
        .replace("%H%", &origin(port))
}

fn origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn resource_text(file: &str, port: u16) -> String {
    let id = file.split('.').next().unwrap_or(file);
    let template = match file {
        "evil.js" => BEACON.to_owned(),
        "evil.css" => "@import url(\"%H%/R/evil_css_import.css\");\n#cssfile{background:url(\"%H%/R/evil_css_bg.png\")}\n".to_owned(),
        _ => "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"50\" height=\"50\" onload=\"%JS%\"><script>%JS%</script><image href=\"%H%/R/%ID%_image.png\" width=\"10\" height=\"10\"/><rect width=\"50\" height=\"50\" fill=\"red\"/></svg>\n".to_owned(),
    };
    if file == "evil.js" {
        template.replace("%ID%", id).replace("%H%", &origin(port))
    } else {
        expand(&template, id, port)
    }
}

fn probes_in(text: &str, port: u16, into: &mut Vec<String>) {
    let host = origin(port);
    let mut rest = text;
    while let Some(at) = rest.find(&host) {
        let tail = &rest[at + host.len()..];
        let end = tail
            .find(['"', '\'', '(', ')', ' ', ',', ';', '<', '&', '?', '\n', '#'])
            .unwrap_or(tail.len());
        let path = tail[..end].to_owned();
        if !into.contains(&path) {
            into.push(path);
        }
        rest = &tail[end..];
    }
}

fn chapter_xhtml(row: &Row, port: u16) -> Vec<u8> {
    let encoding = if row.id == "encoding_utf7" {
        "UTF-7"
    } else {
        "UTF-8"
    };
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"{encoding}\"?>\n{}<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{}</title>{}</head>\n<body{}>\n<p>attack row {}</p>\n{}\n</body></html>\n",
        expand(row.pre, row.id, port),
        row.id,
        expand(row.head, row.id, port),
        expand(row.body_attrs, row.id, port),
        row.id,
        expand(row.body, row.id, port),
    );
    text.into_bytes()
}

fn control_xhtml(port: u16) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>control_plain</title></head>\n<body>\n<h1 id=\"top\">Control chapter</h1>\n<p>attack row control_plain</p>\n<p><img src=\"../Res/safe.png\" alt=\"local image\" width=\"32\" height=\"32\"/></p>\n<p><a href=\"#top\">same chapter link</a></p>\n<p><a href=\"{}.xhtml\">other chapter link</a></p>\n<p><a href=\"{}/C/control_external_click.html\">external link</a></p>\n</body></html>\n",
        ROWS[0].id,
        origin(port)
    )
    .into_bytes()
}

pub fn build(port: u16, zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new("attack", "Attack Fixture");
    package.items.push(Item::nav(false));
    package
        .toc
        .push(TocNode::link("Control chapter", "Text/control_plain.xhtml"));

    let control = Item::page(
        "control_plain",
        "Text/control_plain.xhtml",
        crate::epub::Kind::Text,
        Some("Control chapter"),
        control_xhtml(port),
    );
    package.items.push(control);
    let mut rows = vec![AttackRowTruth {
        id: "control_plain",
        family: "control",
        chapter: 0,
        trigger: "click",
        expect: "local-image-renders-and-external-link-hits-only-after-trusted-click",
        well_formed: true,
        targets_ipc: false,
        probes: vec!["/C/control_external_click.html".to_owned()],
    }];

    let bodies: BTreeMap<&str, String> = RESOURCES
        .iter()
        .map(|(file, _)| (*file, resource_text(file, port)))
        .collect();
    for (ix, row) in ROWS.iter().enumerate() {
        let bytes = chapter_xhtml(row, port);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut probes = Vec::new();
        probes_in(&text, port, &mut probes);
        if let Some(resource) = row.resource.and_then(|r| bodies.get(r)) {
            probes_in(resource, port, &mut probes);
        }
        let mut item = Item::page(
            row.id,
            &format!("Text/{}.xhtml", row.id),
            crate::epub::Kind::Text,
            None,
            bytes,
        );
        item.well_formed = row.well_formed;
        if row.id == "encoding_utf7" {
            item.encoding = "utf-7-declared";
        }
        item.title = Some(row.id.to_owned());
        package.items.push(item);
        let label = if row.id == "search_title_markup" {
            expand(TITLE_PAYLOAD, row.id, port)
        } else {
            row.id.to_owned()
        };
        package
            .toc
            .push(TocNode::link(label, format!("Text/{}.xhtml", row.id)));
        rows.push(AttackRowTruth {
            id: row.id,
            family: row.family,
            chapter: ix as u32 + 1,
            trigger: match row.trigger {
                Trigger::Load => "load",
                Trigger::Click => "click",
            },
            expect: "no-probe-hit-no-pwned-call",
            well_formed: row.well_formed,
            targets_ipc: text.contains("pwned"),
            probes,
        });
    }
    for (file, media_type) in RESOURCES {
        package.items.push(Item::resource(
            &file.replace('.', "_"),
            &format!("Res/{file}"),
            media_type,
            bodies[file].clone().into_bytes(),
        ));
    }
    let safe_png = png::page(derive("attack-safe", 0), 32, 32);
    package.items.push(Item::resource(
        "safe_png",
        "Res/safe.png",
        "image/png",
        safe_png.clone(),
    ));

    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    zip.add(b"OEBPS/Res/unlisted.png", &safe_png, STORED)?;

    Ok(Truth {
        book: "attack".to_owned(),
        category: "attack".to_owned(),
        meant_valid: true,
        title: package.title.clone(),
        linear_chapters: package.linear_chapters(),
        spine: package.spine_rows(),
        toc: package.toc_rows(),
        terms: counts.finish(false),
        bomb: None,
        attack: Some(AttackTruth {
            probe_port: port,
            rows,
        }),
        clean: None,
    })
}
