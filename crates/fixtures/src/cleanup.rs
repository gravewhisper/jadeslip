use crate::books::finish_plain;
use crate::epub::{Item, Kind, Package, TocNode};
use crate::plants::Counts;
use crate::truth::{CleanTruth, PlantedLine, RetitledChapter, Truth};
use crate::zip::ZipWriter;
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const NAME: &str = "planted-junk";
const CHAPTERS: u32 = 40;
const NO_HEADING: u32 = 7;
const PULLED: &[u32] = &[6, 16, 26, 36];
const MISMATCHED: u32 = 31;
const REPORT: &str = "If you find any errors ( broken links, non-standard content, etc.. ), Please let us know < report chapter > so we can fix it as soon as possible.";
const REPORT_LINES: &[u32] = &[12, 13];
const REPORT_DIV: u32 = 21;
const NOTE_BLOCK: u32 = 18;
const NOTE_TAIL: &str =
    "Thanks to everyone who voted this week, the bonus chapter comes on Friday.";
const STORY: &[&str] = &[
    "Han Mo walked down the stone road while the wind carried ash from the burning valley.",
    "The elders whispered behind their sleeves, unsure whether to bow or to run.",
    "He set the bronze lamp on the table and waited for the old man to speak first.",
    "Far below, the river turned silver under a moon that seemed too close to the mountains.",
    "Nobody in the sect remembered the last time the iron gate had opened on its own.",
];
const WATERMARKS: &[&str] = &[
    "Read latest chapters at novelfire.net",
    "This chapter is from lightnovelworld.com",
    "Source: https://www.novelplace.com/iron-monsoon-807",
    "Visit freewebnovel.com for the fastest updates",
];
const BARE_DOMAIN: &str = "novelbin.net";
const CREDIT: &str = "Translator: SilverLotusTL";
const EDITOR: &str = "Editor: Kenji";
const TL_NOTES: &[&str] = &[
    "(TL Note: Dao means the way, the path of cultivation.)",
    "[T/N: A li is about half a kilometre.]",
];
const PATREON: &str =
    "Support us on Patreon to read ten chapters ahead and vote for the next bonus chapter!";
const DIALOGUE: &str = "\u{201c}Did you see his Patreon page?\u{201d} she asked with a grin.";
const PATRONS: &str = "The patrons of the inn fell silent when the stranger walked in.";
const WORDS: &[&str] = &[
    "Iron Gate", "Ash Valley", "The Old Lamp", "Silver River", "Nine Heavens", "Bronze Bell",
    "Quiet Sect", "Falling Star",
];

enum Line {
    Story(String),
    Junk(&'static str, String),
    Kept(String),
    Pulled(String),
    Rule,
}

fn word_of(ordinal: u32) -> &'static str {
    WORDS[ordinal as usize % WORDS.len()]
}

fn title_of(ordinal: u32) -> String {
    format!("Chapter {}: {}", ordinal + 1, word_of(ordinal))
}

fn heading_of(ordinal: u32) -> Option<String> {
    let number = ordinal + 1;
    let word = word_of(ordinal);
    if ordinal == NO_HEADING {
        return None;
    }
    if PULLED.contains(&ordinal) {
        return Some(format!("Chapter {number}"));
    }
    if ordinal == MISMATCHED {
        return Some(format!("Chapter {number}"));
    }
    Some(match ordinal % 10 {
        2 => format!("Chapter {number} - {number} {word}"),
        3 => format!("Chapter {number} \u{2013} {word}"),
        4 => format!("Chapter {number} - {word}"),
        8 => format!("Chapter {number} {word}"),
        9 => format!("PJ {number} - {word}"),
        _ => title_of(ordinal),
    })
}

fn label_of(ordinal: u32) -> String {
    heading_of(ordinal).unwrap_or_else(|| title_of(ordinal))
}

fn fixed_title(ordinal: u32) -> Option<String> {
    (ordinal != NO_HEADING && ordinal != MISMATCHED)
        .then(|| format!("{} {}", ordinal + 1, word_of(ordinal)))
}

fn story(ordinal: u32, at: u32) -> Line {
    let sentence = STORY[((ordinal + at) as usize) % STORY.len()];
    Line::Story(format!("{sentence} ({}.{})", ordinal + 1, at + 1))
}

fn lines_of(ordinal: u32) -> Vec<Line> {
    let mut lines = Vec::new();
    if ordinal == NO_HEADING {
        lines.push(Line::Kept(title_of(ordinal)));
    } else if PULLED.contains(&ordinal) {
        lines.push(Line::Pulled(title_of(ordinal)));
    } else if ordinal == MISMATCHED {
        lines.push(Line::Kept(format!(
            "Chapter {}: {}",
            ordinal + 2,
            word_of(ordinal)
        )));
    } else if ordinal % 3 != 2 {
        lines.push(Line::Junk("heading", label_of(ordinal)));
    }
    if ordinal % 4 == 1 {
        lines.push(Line::Junk("notes", CREDIT.to_owned()));
    }
    if ordinal % 8 == 5 {
        lines.push(Line::Junk("notes", EDITOR.to_owned()));
    }
    lines.push(story(ordinal, 0));
    if ordinal.is_multiple_of(5) {
        let text = WATERMARKS[(ordinal / 5) as usize % WATERMARKS.len()];
        lines.push(Line::Junk("watermark", text.to_owned()));
    }
    lines.push(story(ordinal, 1));
    if ordinal % 6 == 2 {
        let text = TL_NOTES[(ordinal / 6) as usize % TL_NOTES.len()];
        lines.push(Line::Junk("notes", text.to_owned()));
    }
    if ordinal == 4 || ordinal == 24 {
        lines.push(Line::Kept(DIALOGUE.to_owned()));
    }
    if ordinal == 10 {
        lines.push(Line::Kept(PATRONS.to_owned()));
    }
    lines.push(story(ordinal, 2));
    if ordinal % 11 == 3 {
        lines.push(Line::Junk("watermark", BARE_DOMAIN.to_owned()));
    }
    if ordinal == 9 || ordinal == 29 {
        lines.push(Line::Junk("notes", PATREON.to_owned()));
    }
    if REPORT_LINES.contains(&ordinal) {
        lines.push(Line::Junk("watermark", REPORT.to_owned()));
    }
    if ordinal == NOTE_BLOCK {
        lines.push(Line::Rule);
        lines.push(Line::Junk("notes", TL_NOTES[0].to_owned()));
        lines.push(Line::Junk("notes", NOTE_TAIL.to_owned()));
    }
    if ordinal == REPORT_DIV {
        lines.push(Line::Junk("watermark", REPORT.to_owned()));
    }
    lines
}

fn text_of(line: &Line) -> &str {
    match line {
        Line::Story(text) | Line::Junk(_, text) | Line::Kept(text) | Line::Pulled(text) => text,
        Line::Rule => "",
    }
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

fn body_of(ordinal: u32, lines: &[Line]) -> String {
    let heading = heading_of(ordinal)
        .map(|text| format!("<h1>{}</h1>\n", escaped(&text)))
        .unwrap_or_default();
    let (lines, tail) = match lines.split_last() {
        Some((last, rest)) if ordinal == REPORT_DIV => (
            rest,
            format!(
                "\n<div align=\"left\">\n  {} </div>",
                escaped(text_of(last))
            ),
        ),
        _ => (lines, String::new()),
    };
    if ordinal.is_multiple_of(2) {
        let paragraphs: Vec<String> = lines
            .iter()
            .map(|line| match line {
                Line::Rule => "<hr/>".to_owned(),
                _ => format!("<p>{}</p>", escaped(text_of(line))),
            })
            .collect();
        format!("{heading}{}{tail}", paragraphs.join("\n"))
    } else {
        let texts: Vec<String> = lines.iter().map(|line| escaped(text_of(line))).collect();
        format!("{heading}<div>{}</div>{tail}", texts.join("<br/><br/>\n"))
    }
}

fn page(ordinal: u32, lines: &[Line]) -> Vec<u8> {
    let title = escaped(&label_of(ordinal));
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title></head>\n<body>\n{}\n</body></html>\n",
        body_of(ordinal, lines)
    )
    .into_bytes()
}

pub fn planted_junk(zip: &mut ZipWriter) -> io::Result<Truth> {
    let mut package = Package::new(NAME, "Planted Junk");
    package.items.push(Item::ncx());
    package.items.push(Item::nav(false));
    let mut truth = CleanTruth::default();
    let mut chapters: BTreeMap<&'static str, BTreeSet<u32>> = BTreeMap::new();
    for ordinal in 0..CHAPTERS {
        let lines = lines_of(ordinal);
        let href = format!("Text/c{ordinal:02}.xhtml");
        let label = label_of(ordinal);
        package.toc.push(TocNode::link(label, href.clone()));
        let heading = heading_of(ordinal);
        package.items.push(Item::page(
            &format!("c{ordinal:02}"),
            &href,
            Kind::Text,
            heading.as_deref(),
            page(ordinal, &lines),
        ));
        if let (Some(from), Some(to)) = (heading, fixed_title(ordinal)) {
            *truth.matches.entry("titles").or_default() += 1;
            chapters.entry("titles").or_default().insert(ordinal);
            let pulled = lines.iter().find_map(|line| match line {
                Line::Pulled(text) => Some(text.clone()),
                _ => None,
            });
            truth.titles.push(RetitledChapter {
                chapter: ordinal,
                from,
                to,
                pulled,
            });
        }
        for line in &lines {
            match line {
                Line::Junk(rule, text) => {
                    *truth.matches.entry(rule).or_default() += 1;
                    chapters.entry(rule).or_default().insert(ordinal);
                    truth.planted.push(PlantedLine {
                        chapter: ordinal,
                        rule,
                        text: text.clone(),
                    });
                }
                Line::Kept(text) => truth.kept.push(text.clone()),
                Line::Story(_) | Line::Pulled(_) | Line::Rule => {}
            }
        }
    }
    truth.chapters = chapters
        .into_iter()
        .map(|(rule, set)| (rule, set.len() as u32))
        .collect();
    let mut counts = Counts::default();
    package.write_all(zip, &mut counts)?;
    let mut done = finish_plain(&package, "cleanup", counts);
    done.clean = Some(truth);
    Ok(done)
}
