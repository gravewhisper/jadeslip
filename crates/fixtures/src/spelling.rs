use crate::epub::{Item, Kind, Package};
use crate::plants::Counts;
use crate::zip::ZipWriter;
use std::fs::File;
use std::io;
use std::path::Path;

pub struct Planted {
    pub chapter: u32,
    pub wrong: &'static str,
    pub right: &'static str,
}

pub const PROSE: u32 = 0;
pub const LINES: u32 = 1;
pub const FORMS: u32 = 2;
pub const SCRIPTS: u32 = 3;
pub const FRENCH_UNTAGGED: u32 = 4;
pub const TAGGED: u32 = 5;
pub const AGAIN: u32 = 6;
pub const BARE: u32 = 7;
pub const CHAPTERS: u32 = 8;

pub const PLANTED: &[Planted] = &[
    Planted {
        chapter: PROSE,
        wrong: "recieve",
        right: "receive",
    },
    Planted {
        chapter: PROSE,
        wrong: "seperate",
        right: "separate",
    },
    Planted {
        chapter: PROSE,
        wrong: "definately",
        right: "definitely",
    },
    Planted {
        chapter: PROSE,
        wrong: "Teh",
        right: "The",
    },
    Planted {
        chapter: LINES,
        wrong: "occured",
        right: "occurred",
    },
    Planted {
        chapter: LINES,
        wrong: "wierd",
        right: "weird",
    },
    Planted {
        chapter: LINES,
        wrong: "grammer",
        right: "grammar",
    },
    Planted {
        chapter: LINES,
        wrong: "tommorow",
        right: "tomorrow",
    },
    Planted {
        chapter: AGAIN,
        wrong: "definately",
        right: "definitely",
    },
    Planted {
        chapter: AGAIN,
        wrong: "definately",
        right: "definitely",
    },
    Planted {
        chapter: BARE,
        wrong: "acheive",
        right: "achieve",
    },
];

pub const BRITISH_IN_PROSE: &[&str] = &["harbour", "grey"];

const PROSE_BODY: &str = r#"<h1>The Lighthouse Keeper</h1>
<p>The storm had been gathering over the harbour wall since noon, and by evening the fishing boats were tied two deep along the quay. Nobody in the village expected to recieve news from the mainland before the weather broke.</p>
<p>Anna climbed the spiral stairs with a lantern in one hand and a bundle of letters in the other. She kept the logbook in a <em>seperate</em> drawer, away from the damp, because the ink ran whenever the sea air found it.</p>
<p>From the gallery she could see the whole bay. The waves were breaking white against the northern rocks, and a single light moved slowly past the point. It was definately a ship, and it was much too close to the shore.</p>
<p>Teh lamp turned above her head with a steady, patient sound. She had trimmed the wick herself that morning, polished the great lens, and checked the oil twice. There was nothing more to do except watch, and wait, and write down what she saw.</p>
<p>An hour later the wind dropped without warning. The silence was stranger than the noise had been. She opened the window on the landward side and listened for the church bell, which would ring if the lifeboat crew were called from their beds.</p>
<p>When the first grey light came over the hills, the ship was gone from the point and riding safely at anchor in the deep water behind the island. Anna closed the book, wiped her hands, and went down to light the kitchen fire.</p>"#;

const LINES_BODY: &str = "<h1>Notes from the Road</h1>\n<div>\u{1F680} It occured to me that the road was longer than the map had promised.<br/>We had walked since dawn and the river was still ahead of us.<br/>\u{1F680}\u{1F680} A wierd bird kept pace with the cart all afternoon.<br/>My sister corrected my <b>gram</b>mer whenever I wrote the journal aloud.<br/>Caf\u{E9} owners &#8212; tommorow we will be there &#8212; serve soup at noon.<br/></div>";

const FORMS_BODY: &str = "<h1>Ordinary Forms</h1>\n<p>Don't worry, it's fine. Don\u{2019}t worry, it\u{2019}s fine. The dogs' bowls and the cat's bed were clean. She said 'hello' and \u{2018}goodbye\u{2019} to everyone.</p>\n<p>A well-known, long-term, twenty-one year old mother-in-law lives nearby. We read an e-mail about the x-ray.</p>\n<p>There were 12 boats, 3rd place, 1st prize, A4 paper, and room 101b. NASA and the BBC and UNESCO sent a reply. HELLO, shouted NOBODY.</p>\n<p>See https://example.invalid/some/longpathz or www.exampelz.invalid or writer@exampelz.invalid for details.</p>\n<p>The inter\u{AD}esting caf\u{E9} was na\u{EF}ve about the r\u{E9}sum\u{E9} and the cafe\u{301}.</p>\n<p>Captain Wenzhi met Elder Zhao at the gate. Later that night, Captain Wenzhi spoke with Elder Zhao's guard. Wenzhi nodded.</p>";

const SCRIPTS_BODY: &str = "<h1>Other Scripts</h1>\n<p>\u{5929}\u{5730}\u{4EBA}\u{5C71}\u{6C34}\u{98CE}\u{4E91}\u{96E8}\u{96EA}\u{65E5}\u{661F}\u{6D77}\u{6CB3}\u{6E56}\u{6797}\u{6728}\u{77F3}\u{706B}\u{91D1}\u{571F}</p>\n<p>\u{3053}\u{3093}\u{306B}\u{3061}\u{306F} \u{30AB}\u{30BF}\u{30AB}\u{30CA} \u{4E16}\u{754C}</p>\n<p>\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{C138}\u{ACC4}</p>\n<p>\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43C}\u{438}\u{440} \u{434}\u{43E}\u{431}\u{440}\u{44B}\u{439} \u{434}\u{435}\u{43D}\u{44C}</p>\n<p>\u{393}\u{3B5}\u{3B9}\u{3B1} \u{3C3}\u{3BF}\u{3C5} \u{3BA}\u{3CC}\u{3C3}\u{3BC}\u{3B5}</p>\n<p>\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}</p>\n<p>\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}</p>\n<p>\u{E2A}\u{E27}\u{E31}\u{E2A}\u{E14}\u{E35} \u{E0A}\u{E32}\u{E27}\u{E42}\u{E25}\u{E01}</p>\n<p>\u{928}\u{92E}\u{938}\u{94D}\u{924}\u{947} \u{926}\u{941}\u{928}\u{93F}\u{92F}\u{93E}</p>";

pub const FRENCH: &str = "Le vieux gardien montait lentement les marches du phare, une lanterne \u{E0} la main. La mer \u{E9}tait calme ce soir, mais les p\u{EA}cheurs restaient pr\u{E8}s du feu et parlaient de la temp\u{EA}te qui approchait depuis le nord.";

const CODE: &str = "fn zqxw_init(mut bufz: Vec&lt;u8&gt;) -&gt; usizz { bufz.lenn() }";

const AGAIN_BODY: &str = "<h1>The Second Letter</h1>\n<p>The port master wrote that the boat would definately arrive by Friday.</p>\n<p>He added that it would definately carry the mail and three passengers from the island.</p>";

const BARE_BODY: &str = "We hope to acheive a calm crossing before the tide turns.";

fn page(title: &str, body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title></head><body>{body}</body></html>\n"
    )
    .into_bytes()
}

fn chapter(package: &mut Package, ordinal: u32, title: &str, body: &str) {
    package.items.push(Item::page(
        &format!("c{ordinal:02}"),
        &format!("Text/c{ordinal:02}.xhtml"),
        Kind::Text,
        Some(title),
        page(title, body),
    ));
}

fn write(package: &Package, path: &Path) -> io::Result<()> {
    let mut zip = ZipWriter::new(File::create(path)?);
    package.write_all(&mut zip, &mut Counts::default())?;
    zip.finish()?;
    Ok(())
}

pub fn write_spelling_book(path: &Path) -> io::Result<()> {
    let mut package = Package::new("spelling", "Spelling Marks");
    chapter(&mut package, PROSE, "The Lighthouse Keeper", PROSE_BODY);
    chapter(&mut package, LINES, "Notes from the Road", LINES_BODY);
    chapter(&mut package, FORMS, "Ordinary Forms", FORMS_BODY);
    chapter(&mut package, SCRIPTS, "Other Scripts", SCRIPTS_BODY);
    chapter(
        &mut package,
        FRENCH_UNTAGGED,
        "Untagged French",
        &format!("<h1>French</h1>\n<p>{FRENCH}</p>"),
    );
    chapter(
        &mut package,
        TAGGED,
        "Tagged Passages",
        &format!(
            "<h1>Tagged</h1>\n<p xml:lang=\"fr\">{FRENCH}</p>\n<p lang=\"fr\">{FRENCH}</p>\n<pre><code>{CODE}</code></pre>\n<p>The clerk said <code>zqxwvut</code> was the password.</p>"
        ),
    );
    chapter(&mut package, AGAIN, "The Second Letter", AGAIN_BODY);
    chapter(&mut package, BARE, "Bare Text", BARE_BODY);
    write(&package, path)
}

pub fn write_french_book(path: &Path) -> io::Result<()> {
    let mut package = Package::new("spelling-french", "Le Phare");
    package.language = "fr-FR";
    chapter(
        &mut package,
        0,
        "Le Phare",
        &format!("<h1>Le Phare</h1>\n<p>{FRENCH}</p>"),
    );
    write(&package, path)
}

pub fn write_worst_chapter(path: &Path, words: usize) -> io::Result<()> {
    let mut body = String::with_capacity(words * 12);
    body.push_str("<h1>Worst</h1>\n<p>");
    for word in 0..words {
        let mut left = word + 1;
        body.push_str("zq");
        while left > 0 {
            body.push(char::from(b'a' + (left % 26) as u8));
            left /= 26;
        }
        body.push_str("xv ");
    }
    body.push_str("</p>");
    let mut package = Package::new("spelling-worst", "Worst Chapter");
    chapter(&mut package, 0, "Worst", &body);
    write(&package, path)
}
