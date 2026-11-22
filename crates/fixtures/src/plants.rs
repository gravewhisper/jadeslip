use crate::truth::TermTruth;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub enum Schedule {
    Every(u32, u32),
    First,
    Last,
    Never,
}

pub struct Term {
    pub visible: &'static str,
    pub class: &'static str,
    pub spellings: &'static [&'static str],
    pub schedule: Schedule,
}

pub const HEADING_TERM: &str = "quartzlantern";
pub const HIDDEN_TERM: &str = "quillhidden";
pub const SPLIT_BY_BREAK: &str = "emberqstone";
pub const SPLIT_BY_BLOCK: &str = "frostqgate";

pub const TERMS: &[Term] = &[
    Term {
        visible: "zephyrquill",
        class: "plain",
        spellings: &["zephyrquill"],
        schedule: Schedule::Every(3, 0),
    },
    Term {
        visible: "ZEPHYRQUILL",
        class: "plain-case-variant",
        spellings: &["ZEPHYRQUILL"],
        schedule: Schedule::Every(5, 1),
    },
    Term {
        visible: "ZephyrQuill",
        class: "plain-case-variant",
        spellings: &["ZephyrQuill"],
        schedule: Schedule::Every(11, 2),
    },
    Term {
        visible: "firstquark",
        class: "plain-first-chapter-only",
        spellings: &["firstquark"],
        schedule: Schedule::First,
    },
    Term {
        visible: "lastquark",
        class: "plain-last-chapter-only",
        spellings: &["lastquark"],
        schedule: Schedule::Last,
    },
    Term {
        visible: "quillmarble",
        class: "split-inline",
        spellings: &[
            "quill<b>mar</b>ble",
            "<i>q</i>uillmarble",
            "quillmarb<em>l</em>e",
            "<span class=\"k\">quill</span><a href=\"#n\">marble</a>",
            "q<b><i>uill</i>mar</b>ble",
        ],
        schedule: Schedule::Every(9, 3),
    },
    Term {
        visible: "Tomq & Jerrold",
        class: "entity",
        spellings: &[
            "Tomq &amp; Jerrold",
            "Tomq &#38; Jerrold",
            "Tomq &#x26; Jerrold",
        ],
        schedule: Schedule::Every(7, 4),
    },
    Term {
        visible: "<glimmerq>",
        class: "entity",
        spellings: &["&lt;glimmerq&gt;", "&#60;glimmerq&#62;"],
        schedule: Schedule::Every(13, 5),
    },
    Term {
        visible: "&lt;glimmerq&gt;",
        class: "entity-single-decode",
        spellings: &["&amp;lt;glimmerq&amp;gt;"],
        schedule: Schedule::Every(17, 6),
    },
    Term {
        visible: "caféxnoir",
        class: "entity-non-ascii",
        spellings: &[
            "caf&#233;xnoir",
            "caf&#xE9;xnoir",
            "caféxnoir",
            "caf<b>&#233;</b>xnoir",
        ],
        schedule: Schedule::Every(19, 7),
    },
    Term {
        visible: "goldq\u{a0}leaf",
        class: "entity-nbsp",
        spellings: &["goldq&#160;leaf", "goldq\u{a0}leaf"],
        schedule: Schedule::Every(23, 8),
    },
    Term {
        visible: "春江花月夜",
        class: "non-ascii",
        spellings: &["春江花月夜", "春江<b>花月</b>夜", "&#26149;江花月夜"],
        schedule: Schedule::Every(6, 2),
    },
    Term {
        visible: "Ångströmvægt",
        class: "non-ascii",
        spellings: &["Ångströmvægt"],
        schedule: Schedule::Every(29, 9),
    },
    Term {
        visible: "ÅNGSTRÖMVÆGT",
        class: "non-ascii-case-variant",
        spellings: &["ÅNGSTRÖMVÆGT"],
        schedule: Schedule::Every(31, 10),
    },
    Term {
        visible: "🜲emberq",
        class: "non-bmp",
        spellings: &["🜲emberq", "&#x1F732;emberq"],
        schedule: Schedule::Every(37, 11),
    },
    Term {
        visible: "cafe\u{301}q",
        class: "non-ascii-decomposed",
        spellings: &["cafe\u{301}q"],
        schedule: Schedule::Every(41, 12),
    },
    Term {
        visible: "丂亐乥",
        class: "gbk-ascii-trail-bytes",
        spellings: &["丂亐乥"],
        schedule: Schedule::Every(43, 13),
    },
    Term {
        visible: "silverq heronx",
        class: "whitespace-collapse",
        spellings: &[
            "silverq\n      heronx",
            "silverq  heronx",
            "silverq\theronx",
        ],
        schedule: Schedule::Every(47, 14),
    },
    Term {
        visible: HIDDEN_TERM,
        class: "hidden-decoys-plus-visible",
        spellings: &[HIDDEN_TERM],
        schedule: Schedule::Every(40, 15),
    },
    Term {
        visible: SPLIT_BY_BREAK,
        class: "zero-split-by-br",
        spellings: &["emberq<br/>stone"],
        schedule: Schedule::Every(53, 16),
    },
    Term {
        visible: SPLIT_BY_BLOCK,
        class: "zero-split-by-block",
        spellings: &[],
        schedule: Schedule::Never,
    },
    Term {
        visible: HEADING_TERM,
        class: "heading",
        spellings: &[],
        schedule: Schedule::Never,
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Sparse,
    Dense,
}

pub fn occurs(term: &Term, ordinal: u32, total: u32, density: Density) -> bool {
    match term.schedule {
        Schedule::First => ordinal == 0,
        Schedule::Last => ordinal + 1 == total,
        Schedule::Never => false,
        Schedule::Every(every, phase) => match density {
            Density::Sparse => ordinal % every == phase % every,
            Density::Dense => ordinal % 2 == phase % 2,
        },
    }
}

#[derive(Default)]
pub struct Counts(BTreeMap<&'static str, BTreeMap<u32, u32>>);

impl Counts {
    pub fn add(&mut self, term: &'static str, spine_ix: u32) {
        *self.0.entry(term).or_default().entry(spine_ix).or_default() += 1;
    }

    pub fn finish(self, planted: bool) -> BTreeMap<String, TermTruth> {
        let mut out = BTreeMap::new();
        if !planted {
            return out;
        }
        let mut folded: BTreeMap<String, BTreeMap<u32, u32>> = BTreeMap::new();
        for term in TERMS {
            let per_chapter = self.0.get(term.visible);
            let bucket = folded.entry(term.visible.to_lowercase()).or_default();
            for (&ix, &n) in per_chapter.into_iter().flatten() {
                *bucket.entry(ix).or_default() += n;
            }
        }
        for term in TERMS {
            let exact = self.0.get(term.visible).cloned().unwrap_or_default();
            let insensitive = folded[&term.visible.to_lowercase()].clone();
            out.insert(
                term.visible.to_owned(),
                TermTruth {
                    class: term.class,
                    exact_total: exact.values().map(|&n| u64::from(n)).sum(),
                    insensitive_total: insensitive.values().map(|&n| u64::from(n)).sum(),
                    exact,
                    insensitive,
                },
            );
        }
        out
    }
}
