use crate::plants::{self, Counts, Density, HEADING_TERM, HIDDEN_TERM, SPLIT_BY_BREAK, TERMS};
use crate::rng::{Rng, derive};
use std::sync::OnceLock;

const FUNCTION_WORDS: &[&str] = &[
    "the", "of", "and", "to", "a", "in", "he", "was", "that", "his", "it", "with", "for", "on",
    "as", "she", "had", "at", "but", "not",
];
const ONSETS: &[&str] = &[
    "b", "br", "c", "ch", "d", "dr", "f", "fl", "g", "gr", "h", "j", "k", "l", "m", "n", "p", "pr",
    "r", "s", "sh", "st", "t", "th", "tr", "v", "w", "y",
];
const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ea", "oo", "ou"];
const CODAS: &[&str] = &["", "", "n", "r", "s", "t", "l", "m", "nd", "st", "ng"];
const HAN_FILLER: &str = "天地人山水风云雨雪日星海河湖林木石火金土东南西北中上下左右大小多少长短高低明暗前后内外来去出入行走立坐看听说话心手足目耳口书剑刀门城路家国王师弟子道法神魔龙虎鸟鱼草花叶根";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    H1P,
    H1DivBr,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filler {
    Latin,
    Han,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Decl {
    Utf8,
    GbkXml,
    GbkMeta,
    GbkHttpEquiv,
}

impl Decl {
    pub fn label(self) -> &'static str {
        match self {
            Decl::Utf8 => "utf-8",
            Decl::GbkXml => "gbk-xml-declaration",
            Decl::GbkMeta => "gbk-meta-charset",
            Decl::GbkHttpEquiv => "gbk-meta-http-equiv",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    UnslashedBr,
    BareAmpersand,
    NbspNoDtd,
    StrayClose,
    ImpliedClose,
    UnquotedAttr,
}

impl Defect {
    pub fn label(self) -> &'static str {
        match self {
            Defect::UnslashedBr => "unslashed-br",
            Defect::BareAmpersand => "bare-ampersand",
            Defect::NbspNoDtd => "nbsp-no-dtd",
            Defect::StrayClose => "stray-close",
            Defect::ImpliedClose => "implied-close",
            Defect::UnquotedAttr => "unquoted-attribute",
        }
    }

    fn tokens(self) -> &'static [(Option<&'static str>, &'static str)] {
        match self {
            Defect::UnslashedBr => &[],
            Defect::BareAmpersand => &[
                (Some("Tomq & Jerrold"), "Tomq & Jerrold"),
                (None, "fish & chips"),
            ],
            Defect::NbspNoDtd => &[
                (Some("goldq\u{a0}leaf"), "goldq&nbsp;leaf"),
                (Some("caféxnoir"), "caf&eacute;xnoir"),
            ],
            Defect::StrayClose => &[
                (Some("zephyrquill"), "zephyrquill</b>"),
                (None, "hold</span>"),
                (Some("quillmarble"), "quill<b>mar</i></b>ble"),
            ],
            Defect::ImpliedClose => &[
                (Some("zephyrquill"), "<ul><li>zephyrquill<li>hold</ul>"),
                (Some("quillmarble"), "<b>quill<i>marble</b></i>"),
            ],
            Defect::UnquotedAttr => &[
                (
                    Some("zephyrquill"),
                    "<span class=k id=a1>zephyrquill</span>",
                ),
                (None, "<a href=#n>hold</a>"),
            ],
        }
    }
}

#[derive(Clone)]
pub struct ChapterSpec {
    pub book: &'static str,
    pub ordinal: u32,
    pub total: u32,
    pub title: Option<String>,
    pub words: (usize, usize),
    pub shape: Shape,
    pub filler: Filler,
    pub decl: Decl,
    pub density: Density,
    pub defect: Option<Defect>,
    pub anchors: &'static [&'static str],
}

impl ChapterSpec {
    fn plants_heading(&self) -> bool {
        self.ordinal.is_multiple_of(25) && self.title.is_some()
    }

    pub fn heading(&self) -> Option<String> {
        match (&self.title, self.plants_heading()) {
            (Some(title), true) => Some(format!("{title} {HEADING_TERM}")),
            (title, _) => title.clone(),
        }
    }
}

fn vocab() -> &'static [String] {
    static VOCAB: OnceLock<Vec<String>> = OnceLock::new();
    VOCAB.get_or_init(|| {
        let mut rng = Rng::new(derive("vocabulary", 0));
        (0..4000)
            .map(|_| {
                let mut word = String::new();
                for _ in 0..rng.range(1, 3) {
                    word.push_str(ONSETS[rng.below(ONSETS.len())]);
                    word.push_str(VOWELS[rng.below(VOWELS.len())]);
                    word.push_str(CODAS[rng.below(CODAS.len())]);
                }
                word
            })
            .collect()
    })
}

fn latin_word(rng: &mut Rng) -> &'static str {
    let r = rng.below(100);
    if r < 35 {
        return FUNCTION_WORDS[rng.below(FUNCTION_WORDS.len())];
    }
    let words = vocab();
    let a = rng.below(words.len());
    let b = rng.below(words.len());
    &words[a.min(b)]
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn title_words(rng: &mut Rng) -> String {
    format!(
        "{} {}",
        capitalize(latin_word(rng)),
        capitalize(latin_word(rng))
    )
}

fn han_word(rng: &mut Rng) -> String {
    let chars: Vec<char> = HAN_FILLER.chars().collect();
    (0..rng.range(2, 4))
        .map(|_| chars[rng.below(chars.len())])
        .collect()
}

fn filler_tokens(rng: &mut Rng, filler: Filler, count: usize) -> Vec<String> {
    let mut tokens = Vec::with_capacity(count);
    let mut left_in_sentence = 0;
    for _ in 0..count {
        if left_in_sentence == 0 {
            left_in_sentence = rng.range(6, 22);
            let token = match filler {
                Filler::Latin => capitalize(latin_word(rng)),
                Filler::Han => han_word(rng),
            };
            tokens.push(token);
        } else {
            let mut token = match filler {
                Filler::Latin => latin_word(rng).to_owned(),
                Filler::Han => han_word(rng),
            };
            if left_in_sentence == 1 {
                token.push_str(match filler {
                    Filler::Latin => ".",
                    Filler::Han => "。",
                });
            }
            tokens.push(token);
        }
        left_in_sentence -= 1;
    }
    tokens
}

fn doc_open(spec: &ChapterSpec, head_title: &str, extra_head: &str) -> String {
    let (prolog, meta) = match spec.decl {
        Decl::Utf8 => ("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n", ""),
        Decl::GbkXml => ("<?xml version=\"1.0\" encoding=\"GBK\"?>\n", ""),
        Decl::GbkMeta => ("", "<meta charset=\"gbk\"/>"),
        Decl::GbkHttpEquiv => (
            "",
            "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\"/>",
        ),
    };
    let doctype = if spec.defect.is_some() {
        ""
    } else {
        "<!DOCTYPE html>\n"
    };
    format!(
        "{prolog}{doctype}<html xmlns=\"http://www.w3.org/1999/xhtml\"><head>{meta}<title>{head_title}</title>{extra_head}</head>\n<body>\n"
    )
}

pub fn encode(decl: Decl, text: &str) -> Vec<u8> {
    match decl {
        Decl::Utf8 => text.as_bytes().to_vec(),
        _ => encoding_rs::GBK.encode(text).0.into_owned(),
    }
}

pub fn render(spec: &ChapterSpec, spine_ix: u32, counts: &mut Counts) -> Vec<u8> {
    let mut rng = Rng::for_item(spec.book, u64::from(spec.ordinal));
    let word_count = rng.range(spec.words.0, spec.words.1);
    let mut tokens = filler_tokens(&mut rng, spec.filler, word_count);

    for term in TERMS {
        if plants::occurs(term, spec.ordinal, spec.total, spec.density) {
            let times = rng.range(1, 3);
            for _ in 0..times {
                let spelling = term.spellings[rng.below(term.spellings.len())];
                let at = rng.below(tokens.len() + 1);
                tokens.insert(at, spelling.to_owned());
                if term.visible != SPLIT_BY_BREAK {
                    counts.add(term.visible, spine_ix);
                }
            }
        }
    }
    if let Some(defect) = spec.defect {
        for (term, source) in defect.tokens() {
            let at = rng.below(tokens.len() + 1);
            tokens.insert(at, (*source).to_owned());
            if let Some(term) = term {
                counts.add(term, spine_ix);
            }
        }
    }

    let hidden = spec.ordinal.is_multiple_of(7);
    let title = spec.heading();
    if spec.plants_heading() {
        counts.add(HEADING_TERM, spine_ix);
    }
    let head_title = match (&spec.title, hidden) {
        (Some(t), true) => format!("{t} {HIDDEN_TERM}"),
        (Some(t), false) => t.clone(),
        (None, true) => HIDDEN_TERM.to_owned(),
        (None, false) => "Untitled".to_owned(),
    };
    let extra_head = if hidden {
        format!(
            "<meta name=\"description\" content=\"{HIDDEN_TERM}\"/><style>/* {HIDDEN_TERM} */</style>"
        )
    } else {
        String::new()
    };
    if hidden {
        let at = rng.below(tokens.len() + 1);
        tokens.insert(at, format!("<span title=\"{HIDDEN_TERM}\">hold</span>"));
        let at = rng.below(tokens.len() + 1);
        tokens.insert(at, format!("<!-- {HIDDEN_TERM} -->"));
        let at = rng.below(tokens.len() + 1);
        tokens.insert(at, format!("<script>/* {HIDDEN_TERM} */</script>"));
    }

    let mut out = doc_open(spec, &head_title, &extra_head);
    if let Some(title) = &title {
        out.push_str("<h1>");
        out.push_str(title);
        out.push_str("</h1>\n");
    }
    let joiner = match spec.filler {
        Filler::Latin => " ",
        Filler::Han => "",
    };
    for anchor in spec.anchors {
        out.push_str(&format!("<p id=\"{anchor}\">Section {anchor}.</p>\n"));
    }
    match spec.shape {
        Shape::H1P => {
            let implied = spec.defect == Some(Defect::ImpliedClose);
            let mut rest = tokens.as_slice();
            while !rest.is_empty() {
                let take = rng.range(30, 70).min(rest.len());
                out.push_str("<p>");
                out.push_str(&rest[..take].join(joiner));
                out.push_str(if implied { "\n" } else { "</p>\n" });
                rest = &rest[take..];
            }
            if spec.density == Density::Dense || spec.ordinal % 53 == 16 {
                out.push_str("<p>frostq</p>\n<p>gate</p>\n");
            }
        }
        Shape::H1DivBr => {
            let br = if spec.defect == Some(Defect::UnslashedBr) {
                "<br>"
            } else {
                "<br/>"
            };
            out.push_str("<div>\n");
            let mut rest = tokens.as_slice();
            while !rest.is_empty() {
                let take = rng.range(40, 110).min(rest.len());
                out.push_str(&rest[..take].join(joiner));
                rest = &rest[take..];
                if !rest.is_empty() {
                    out.push_str(br);
                    out.push('\n');
                    out.push_str(br);
                    out.push('\n');
                }
            }
            out.push_str("\n</div>\n");
        }
    }
    out.push_str("</body></html>\n");
    encode(spec.decl, &out)
}
