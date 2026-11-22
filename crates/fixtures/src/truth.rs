use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Truth {
    pub book: String,
    pub category: String,
    pub meant_valid: bool,
    pub title: String,
    pub linear_chapters: u32,
    pub spine: Vec<SpineRow>,
    pub toc: Vec<TocRow>,
    pub terms: BTreeMap<String, TermTruth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bomb: Option<BombTruth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attack: Option<AttackTruth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clean: Option<CleanTruth>,
}

#[derive(Serialize, Default)]
pub struct CleanTruth {
    pub matches: BTreeMap<&'static str, u32>,
    pub chapters: BTreeMap<&'static str, u32>,
    pub planted: Vec<PlantedLine>,
    pub kept: Vec<String>,
    pub titles: Vec<RetitledChapter>,
}

#[derive(Serialize)]
pub struct RetitledChapter {
    pub chapter: u32,
    pub from: String,
    pub to: String,
    pub pulled: Option<String>,
}

#[derive(Serialize)]
pub struct PlantedLine {
    pub chapter: u32,
    pub rule: &'static str,
    pub text: String,
}

#[derive(Serialize)]
pub struct SpineRow {
    pub ix: u32,
    pub href: Option<String>,
    pub title: Option<String>,
    pub kind: &'static str,
    pub linear: bool,
    pub encoding: &'static str,
    #[serde(rename = "well_formed_after_decoding")]
    pub well_formed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defect: Option<&'static str>,
}

#[derive(Serialize)]
pub struct TocRow {
    pub depth: u16,
    pub label: String,
    pub target: Option<String>,
}

#[derive(Serialize, Default)]
pub struct TermTruth {
    pub class: &'static str,
    pub exact_total: u64,
    pub insensitive_total: u64,
    pub exact: BTreeMap<u32, u32>,
    pub insensitive: BTreeMap<u32, u32>,
}

#[derive(Serialize)]
pub struct BombTruth {
    pub trips: &'static str,
    pub stage: &'static str,
    pub detail: &'static str,
}

#[derive(Serialize)]
pub struct AttackTruth {
    pub probe_port: u16,
    pub rows: Vec<AttackRowTruth>,
}

#[derive(Serialize)]
pub struct AttackRowTruth {
    pub id: &'static str,
    pub family: &'static str,
    pub chapter: u32,
    pub trigger: &'static str,
    pub expect: &'static str,
    pub well_formed: bool,
    pub targets_ipc: bool,
    pub probes: Vec<String>,
}
